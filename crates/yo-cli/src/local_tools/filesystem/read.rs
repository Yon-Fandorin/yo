use std::{
    fs::File,
    io::Read,
    ops::Range,
    os::unix::fs::MetadataExt,
    sync::atomic::{AtomicBool, Ordering},
};

use serde_json::Value;
use yo_core::{ToolExecutionError, ToolExecutionResult};

use super::{
    descriptor::{FileIdentity, OpenRegularError, open_regular_file},
    output::{error, json_string},
    path::AdmittedPath,
};
use crate::local_tools::execution::{completed, failed, interrupted};

pub(super) fn read_file(
    file: impl Read,
    limit: usize,
    cancelled: &AtomicBool,
) -> ToolExecutionResult {
    if cancelled.load(Ordering::Acquire) {
        return interrupted();
    }
    let result = read_bounded(file, limit);
    if cancelled.load(Ordering::Acquire) {
        return interrupted();
    }
    match result {
        Ok((mut bytes, truncated)) => match std::str::from_utf8(&bytes) {
            Ok(_) => completed(
                String::from_utf8(bytes).expect("validated UTF-8 remains valid"),
                truncated,
            ),
            Err(error) if truncated && error.error_len().is_none() => {
                bytes.truncate(error.valid_up_to());
                completed(
                    String::from_utf8(bytes).expect("valid UTF-8 prefix remains valid"),
                    true,
                )
            },
            Err(_) => failed("read_file supports UTF-8 text files only"),
        },
        Err(_) => failed("read_file failed"),
    }
}

fn read_bounded(mut reader: impl Read, limit: usize) -> std::io::Result<(Vec<u8>, bool)> {
    let mut output = Vec::with_capacity(limit.min(64 * 1024));
    let mut chunk = [0_u8; 8 * 1024];
    let target = limit.saturating_add(1);
    while output.len() < target {
        let remaining = target.saturating_sub(output.len());
        let read_len = chunk.len().min(remaining);
        let count = reader.read(&mut chunk[..read_len])?;
        if count == 0 {
            break;
        }
        output.extend_from_slice(&chunk[..count]);
    }
    let truncated = output.len() > limit;
    output.truncate(limit);
    Ok((output, truncated))
}

const MAX_FILE_BYTES: usize = 16 * 1024 * 1024;
const MAX_ITEMS: usize = 8;
const MAX_LINES: usize = 400;
const MAX_ITEM_BYTES: usize = 16_384;

#[derive(Clone, Debug)]
pub(super) struct ReadRequest {
    path: AdmittedPath,
    offset: usize,
    limit: usize,
}

pub(super) fn parse_requests(
    arguments: &Value,
    admit_path: fn(&str) -> Result<AdmittedPath, ToolExecutionError>,
) -> Result<Vec<ReadRequest>, ToolExecutionError> {
    let files = arguments
        .get("files")
        .and_then(Value::as_array)
        .ok_or_else(|| ToolExecutionError::new("validated read_files array is unavailable"))?;
    if files.is_empty() || files.len() > MAX_ITEMS {
        return Err(ToolExecutionError::new(
            "read_files requires between 1 and 8 file windows",
        ));
    }
    files
        .iter()
        .map(|item| {
            let path = item
                .get("path")
                .and_then(Value::as_str)
                .ok_or_else(|| ToolExecutionError::new("validated file path is unavailable"))?;
            let offset = bounded_integer(item.get("offset"), 1, usize::MAX, 1, "offset")?;
            let limit = bounded_integer(item.get("limit"), 1, MAX_LINES, MAX_LINES, "limit")?;
            Ok(ReadRequest {
                path: admit_path(path)?,
                offset,
                limit,
            })
        })
        .collect()
}

fn bounded_integer(
    value: Option<&Value>,
    minimum: usize,
    maximum: usize,
    default: usize,
    name: &str,
) -> Result<usize, ToolExecutionError> {
    let Some(value) = value else {
        return Ok(default);
    };
    let number = value
        .as_u64()
        .and_then(|number| usize::try_from(number).ok());
    match number {
        Some(number) if (minimum..=maximum).contains(&number) => Ok(number),
        _ => Err(ToolExecutionError::new(format!(
            "read_files {name} is outside its admitted range"
        ))),
    }
}

pub(super) fn execute(
    workspace: File,
    denied_credential: Option<FileIdentity>,
    requests: Vec<ReadRequest>,
    cancelled: &AtomicBool,
) -> ToolExecutionResult {
    let mut items = Vec::with_capacity(requests.len());
    for request in requests {
        if cancelled.load(Ordering::Acquire) {
            return interrupted();
        }
        items.push(read_item(
            &workspace,
            denied_credential,
            &request,
            cancelled,
        ));
        if cancelled.load(Ordering::Acquire) {
            return interrupted();
        }
    }
    completed(format!("{{\"results\":[{}]}}", items.join(",")), false)
}

fn read_item(
    workspace: &File,
    denied_credential: Option<FileIdentity>,
    request: &ReadRequest,
    cancelled: &AtomicBool,
) -> String {
    read_item_after_capture(workspace, denied_credential, request, cancelled, || {})
}

fn read_item_after_capture(
    workspace: &File,
    denied_credential: Option<FileIdentity>,
    request: &ReadRequest,
    cancelled: &AtomicBool,
    after_capture: impl FnOnce(),
) -> String {
    let file = match open_regular_file(workspace, request.path.components(), denied_credential) {
        Ok(file) => file,
        Err(OpenRegularError::NotRegular) => {
            return error(request.path.display(), "not_regular");
        },
        Err(OpenRegularError::Unavailable) => {
            return error(request.path.display(), "unavailable");
        },
    };
    let before = match Snapshot::capture(&file) {
        Ok(snapshot) => snapshot,
        Err(_) => return error(request.path.display(), "unavailable"),
    };
    if before.size > MAX_FILE_BYTES as u64 {
        return error(request.path.display(), "too_large");
    }
    after_capture();
    let mut bytes = Vec::with_capacity((before.size as usize).min(MAX_FILE_BYTES));
    let mut reader = file;
    let mut chunk = [0_u8; 16 * 1024];
    loop {
        if cancelled.load(Ordering::Acquire) {
            return String::new();
        }
        let remaining = MAX_FILE_BYTES + 1 - bytes.len();
        let read_limit = remaining.min(chunk.len());
        let count = match reader.read(&mut chunk[..read_limit]) {
            Ok(count) => count,
            Err(_) => return error(request.path.display(), "unavailable"),
        };
        if count == 0 {
            break;
        }
        bytes.extend_from_slice(&chunk[..count]);
        if bytes.len() == MAX_FILE_BYTES + 1 {
            break;
        }
    }
    let after = match Snapshot::capture(&reader) {
        Ok(snapshot) => snapshot,
        Err(_) => return error(request.path.display(), "changed_during_read"),
    };
    if before != after || after.size != bytes.len() as u64 {
        return error(request.path.display(), "changed_during_read");
    }
    if bytes.len() > MAX_FILE_BYTES {
        return error(request.path.display(), "too_large");
    }
    let Ok(content) = std::str::from_utf8(&bytes) else {
        return error(request.path.display(), "non_utf8");
    };
    render_window(request, content)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct Snapshot {
    device: u64,
    inode: u64,
    size: u64,
    modified_seconds: i64,
    modified_nanoseconds: i64,
    changed_seconds: i64,
    changed_nanoseconds: i64,
}

impl Snapshot {
    fn capture(file: &File) -> std::io::Result<Self> {
        let metadata = file.metadata()?;
        Ok(Self {
            device: metadata.dev(),
            inode: metadata.ino(),
            size: metadata.len(),
            modified_seconds: metadata.mtime(),
            modified_nanoseconds: metadata.mtime_nsec(),
            changed_seconds: metadata.ctime(),
            changed_nanoseconds: metadata.ctime_nsec(),
        })
    }
}

fn render_window(request: &ReadRequest, content: &str) -> String {
    let (total, lines) = selected_line_ranges(content.as_bytes(), request.offset, request.limit);
    if total == 0 {
        return if request.offset == 1 {
            format!(
                "{{\"path\":{},\"status\":\"ok\",\"start\":0,\"end\":0,\"total\":0,\"content\":\"\"}}",
                json_string(request.path.display())
            )
        } else {
            error(request.path.display(), "offset_out_of_range")
        };
    }
    if request.offset > total {
        return error(request.path.display(), "offset_out_of_range");
    }
    for count in (1..=lines.len()).rev() {
        let span = selected_span(content, &lines[..count]);
        let end = request.offset + count - 1;
        let rendered = success_item(
            request.path.display(),
            request.offset,
            end,
            total,
            (end < total).then_some(end + 1),
            span,
        );
        if rendered.len() <= MAX_ITEM_BYTES {
            return rendered;
        }
    }
    error(request.path.display(), "line_too_large")
}

fn selected_line_ranges(
    bytes: &[u8],
    requested_offset: usize,
    requested_limit: usize,
) -> (usize, Vec<Range<usize>>) {
    let mut selected = Vec::with_capacity(requested_limit.min(MAX_LINES));
    let mut start = 0;
    let mut total = 0;
    for (index, byte) in bytes.iter().enumerate() {
        if *byte == b'\n' {
            total += 1;
            if total >= requested_offset && selected.len() < requested_limit {
                selected.push(start..index + 1);
            }
            start = index + 1;
        }
    }
    if start < bytes.len() {
        total += 1;
        if total >= requested_offset && selected.len() < requested_limit {
            selected.push(start..bytes.len());
        }
    }
    (total, selected)
}

fn selected_span<'a>(content: &'a str, lines: &[Range<usize>]) -> &'a str {
    let start = lines[0].start;
    let end = lines[lines.len() - 1].end;
    &content[start..end]
}

fn success_item(
    path: &str,
    start: usize,
    end: usize,
    total: usize,
    next_offset: Option<usize>,
    content: &str,
) -> String {
    let mut output = format!(
        "{{\"path\":{},\"status\":\"ok\",\"start\":{start},\"end\":{end},\"total\":{total}",
        json_string(path)
    );
    if let Some(next_offset) = next_offset {
        output.push_str(&format!(",\"next_offset\":{next_offset}"));
    }
    output.push_str(&format!(",\"content\":{}}}", json_string(content)));
    output
}

#[cfg(test)]
mod tests {
    use std::{
        ffi::OsString,
        fs::{self, OpenOptions},
        io::{Read, Write},
        sync::{
            Arc,
            atomic::{AtomicBool, AtomicUsize, Ordering},
        },
    };

    use super::{
        AdmittedPath, MAX_FILE_BYTES, ReadRequest, read_item_after_capture, render_window,
        selected_line_ranges,
    };
    use crate::local_tools::tests::TestDirectory;

    fn request(content_offset: usize, limit: usize) -> ReadRequest {
        ReadRequest {
            path: AdmittedPath::new("src/lib.rs".to_owned(), Vec::new()),
            offset: content_offset,
            limit,
        }
    }

    struct CountingReader {
        reads: Arc<AtomicUsize>,
    }

    impl Read for CountingReader {
        fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
            self.reads.fetch_add(1, Ordering::Relaxed);
            buffer.fill(b'x');
            Ok(buffer.len())
        }
    }

    // bounded reader는 limit+1 byte로 truncation을 판별한 즉시 멈춰 무한하거나 거대한
    // 입력을 끝까지 drain하지 않고 반환한다.
    #[test]
    fn bounded_reader_stops_after_the_truncation_probe() {
        let reads = Arc::new(AtomicUsize::new(0));
        let (output, truncated) = super::read_bounded(
            CountingReader {
                reads: Arc::clone(&reads),
            },
            16,
        )
        .unwrap();

        assert_eq!(output.len(), 16);
        assert!(truncated);
        assert_eq!(reads.load(Ordering::Relaxed), 1);
    }

    // legacy 4 MiB probe가 multi-byte scalar 한가운데서 끝나도 완전한 UTF-8 prefix를
    // Completed+truncated로 넘겨 common truncation marker가 붙을 수 있게 합니다.
    #[test]
    fn legacy_reader_truncates_only_an_incomplete_final_scalar() {
        let bytes = [b'a', 0xE2, 0x82, 0xAC];
        let result = super::read_file(&bytes[..], 3, &AtomicBool::new(false));
        assert_eq!(result.outcome(), yo_core::ToolExecutionOutcome::Completed);
        assert_eq!(result.output(), "a");
        assert!(result.truncated());

        let malformed = super::read_file(&[b'a', 0xFF, b'b'][..], 3, &AtomicBool::new(false));
        assert_eq!(malformed.outcome(), yo_core::ToolExecutionOutcome::Failed);
    }

    // LF만 line separator로 취급하고 final LF가 가짜 빈 줄을 만들지 않으며, 다음 offset은
    // 실제로 아직 읽지 않은 첫 line을 가리킵니다.
    #[test]
    fn logical_line_windows_preserve_original_terminators() {
        assert_eq!(
            selected_line_ranges(b"a\r\nb\n", 1, 400),
            (2, vec![0..3, 3..5])
        );
        assert_eq!(
            render_window(&request(1, 1), "a\r\nb\n"),
            r#"{"path":"src/lib.rs","status":"ok","start":1,"end":1,"total":2,"next_offset":2,"content":"a\r\n"}"#
        );
        assert_eq!(
            render_window(&request(1, 400), ""),
            r#"{"path":"src/lib.rs","status":"ok","start":0,"end":0,"total":0,"content":""}"#
        );
    }

    // 첫 logical line 하나가 compact item bound를 넘으면 잘린 성공으로 가장하지 않고
    // 고정 per-file 오류가 됩니다.
    #[test]
    fn oversized_first_line_is_a_discriminating_item_error() {
        assert_eq!(
            render_window(&request(1, 1), &"x".repeat(16_384)),
            r#"{"path":"src/lib.rs","status":"error","error":"line_too_large"}"#
        );
    }

    // 첫 capture는 한도 안이지만 read 전에 한 byte가 추가되면 stable oversized 파일로
    // 오분류하지 않고 두 metadata capture의 차이를 먼저 changed_during_read로 판정합니다.
    #[test]
    fn concurrent_growth_precedes_the_stable_size_limit() {
        let directory = TestDirectory::new();
        let path = directory.0.join("growing.txt");
        fs::write(&path, vec![b'x'; MAX_FILE_BYTES]).unwrap();
        let workspace = fs::File::open(&directory.0).unwrap();
        let request = ReadRequest {
            path: AdmittedPath::new(
                "growing.txt".to_owned(),
                vec![OsString::from("growing.txt")],
            ),
            offset: 1,
            limit: 1,
        };

        let result =
            read_item_after_capture(&workspace, None, &request, &AtomicBool::new(false), || {
                OpenOptions::new()
                    .append(true)
                    .open(&path)
                    .unwrap()
                    .write_all(b"y")
                    .unwrap();
            });

        assert_eq!(
            result,
            r#"{"path":"growing.txt","status":"error","error":"changed_during_read"}"#
        );
    }
}
