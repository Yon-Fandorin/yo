use std::{collections::BTreeSet, path::Path};

use serde::{Deserialize, Serialize};

use crate::{bounded_file, review_protocol, slice_worktree};

const REQUEST_SCHEMA: &str = "yo.slice-cost-report-request/v1alpha1";
const REPORT_SCHEMA: &str = "yo.slice-cost-report/v1alpha1";
const POLICY: &str = "owners-separated/no-cross-owner-total/v1alpha1";
const REQUEST_LIMIT: usize = 256 * 1024;
const SOURCE_LIMIT: usize = 4 * 1024 * 1024;
const REPORT_LIMIT: usize = 512 * 1024;
const MAX_SOURCES: usize = 64;

#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Request {
    schema: String,
    slice: String,
    candidate_commit: String,
    owners: Owners,
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Owners {
    packet: PacketOwner,
    provider: ProviderOwner,
    coordinator_context: ContextOwner,
    command_output: CommandOwner,
    elapsed: ElapsedOwner,
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct PacketOwner {
    basis: String,
    sources: Vec<Source>,
    publication_count: u64,
    rendered_bytes: Measurement,
    managed_tokens: Measurement,
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct ProviderOwner {
    basis: String,
    sources: Vec<Source>,
    request_count: u64,
    usage: Usage,
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct ContextOwner {
    basis: String,
    sources: Vec<Source>,
    usage: Usage,
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Usage {
    input_tokens: Measurement,
    output_tokens: Measurement,
    total_tokens: Measurement,
    reasoning_tokens: Measurement,
    cache_read_input_tokens: Measurement,
    cache_write_input_tokens: Measurement,
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct CommandOwner {
    basis: String,
    sources: Vec<Source>,
    command_count: u64,
    complete_log_bytes: Measurement,
    returned_bytes: Measurement,
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct ElapsedOwner {
    basis: String,
    sources: Vec<Source>,
    total_milliseconds: Measurement,
    critical_bottleneck: Bottleneck,
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Bottleneck {
    name: String,
    elapsed_milliseconds: u64,
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Source {
    path: String,
    hash: String,
    schema: String,
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(tag = "availability", rename_all = "snake_case", deny_unknown_fields)]
enum Measurement {
    Reported { value: u64 },
    Partial { value: u64, reason: String },
    Unavailable { reason: String },
}

impl Measurement {
    const fn value(&self) -> Option<u64> {
        match self {
            Self::Reported { value } | Self::Partial { value, .. } => Some(*value),
            Self::Unavailable { .. } => None,
        }
    }

    fn validate(&self, label: &str) -> Result<(), String> {
        match self {
            Self::Reported { .. } => Ok(()),
            Self::Partial { reason, .. } | Self::Unavailable { reason } => {
                require_text(reason, &format!("{label} reason"))
            },
        }
    }
}

#[derive(Deserialize)]
struct SchemaEnvelope {
    schema: String,
}

#[derive(Serialize)]
struct Report<'a> {
    schema: &'static str,
    aggregation_policy: &'static str,
    slice: &'a str,
    candidate_commit: &'a str,
    request: CapturedInput<'a>,
    source_artifacts: Vec<CapturedSource>,
    owners: &'a Owners,
}

#[derive(Serialize)]
struct CapturedInput<'a> {
    path: &'a Path,
    hash: String,
    bytes: usize,
}

#[derive(Serialize)]
struct CapturedSource {
    owner: &'static str,
    path: String,
    hash: String,
    schema: String,
    bytes: usize,
}

pub(crate) fn run(repository: &Path, request_path: &Path, output: &Path) -> Result<(), String> {
    let request_bytes =
        bounded_file::read_regular(request_path, REQUEST_LIMIT, "Slice cost report request")?;
    let request: Request = serde_json::from_slice(&request_bytes)
        .map_err(|error| format!("invalid Slice cost report request: {error}"))?;
    validate_request(&request)?;

    let workspace = slice_worktree::workspace_root(repository)?;
    let request_path = canonical_input(&workspace, request_path)?;
    let output = canonical_output(&workspace, output)?;
    if request_path == output {
        return Err("Slice cost report output must differ from its request".to_owned());
    }

    let source_count = owner_sources(&request.owners)
        .iter()
        .map(|(_, sources)| sources.len())
        .sum::<usize>();
    if source_count > MAX_SOURCES {
        return Err(format!(
            "Slice cost report supports at most {MAX_SOURCES} sources"
        ));
    }

    let mut paths = BTreeSet::new();
    let mut identities = BTreeSet::new();
    let mut captured = Vec::new();
    for (owner, sources) in owner_sources(&request.owners) {
        for source in sources {
            let path = canonical_input(&workspace, Path::new(&source.path))?;
            if path == request_path || path == output {
                return Err(
                    "Slice cost source paths must differ from request and output".to_owned(),
                );
            }
            let bytes = bounded_file::read_regular(&path, SOURCE_LIMIT, "Slice cost source")?;
            let hash = review_protocol::digest(&bytes);
            require_hash(&source.hash, "Slice cost source hash")?;
            if hash != source.hash {
                return Err(format!(
                    "Slice cost source hash changed: {}",
                    path.display()
                ));
            }
            let envelope: SchemaEnvelope = serde_json::from_slice(&bytes)
                .map_err(|error| format!("Slice cost source is not JSON: {error}"))?;
            if envelope.schema != source.schema {
                return Err(format!(
                    "Slice cost source schema changed: {}",
                    path.display()
                ));
            }
            if !paths.insert(path.clone())
                || !identities.insert((envelope.schema.clone(), hash.clone()))
            {
                return Err("Slice cost sources cannot be counted more than once".to_owned());
            }
            captured.push(CapturedSource {
                owner,
                path: path.to_string_lossy().into_owned(),
                hash,
                schema: envelope.schema,
                bytes: bytes.len(),
            });
        }
    }
    let report = Report {
        schema: REPORT_SCHEMA,
        aggregation_policy: POLICY,
        slice: &request.slice,
        candidate_commit: &request.candidate_commit,
        request: CapturedInput {
            path: &request_path,
            hash: review_protocol::digest(&request_bytes),
            bytes: request_bytes.len(),
        },
        source_artifacts: captured,
        owners: &request.owners,
    };
    let mut report_bytes = serde_json::to_vec_pretty(&report)
        .map_err(|error| format!("cannot encode Slice cost report: {error}"))?;
    report_bytes.push(b'\n');
    if report_bytes.len() > REPORT_LIMIT {
        return Err("Slice cost report exceeds its output limit".to_owned());
    }

    revalidate_sources(&request, &workspace)?;
    let current_request =
        bounded_file::read_regular(&request_path, REQUEST_LIMIT, "Slice cost report request")?;
    if current_request != request_bytes {
        return Err("Slice cost report request changed before publication".to_owned());
    }
    let created = bounded_file::publish_new_or_exact(
        &output,
        &report_bytes,
        REPORT_LIMIT,
        "Slice cost report",
    )?;
    println!(
        "{}",
        serde_json::to_string(&serde_json::json!({
            "schema": "yo.slice-cost-report-publication/v1alpha1",
            "ok": true,
            "status": if created { "written" } else { "reused" },
            "report_path": output,
            "report_hash": review_protocol::digest(&report_bytes)
        }))
        .map_err(|error| format!("cannot encode Slice cost publication: {error}"))?
    );
    Ok(())
}

fn validate_request(request: &Request) -> Result<(), String> {
    if request.schema != REQUEST_SCHEMA {
        return Err(format!(
            "Slice cost request must use schema `{REQUEST_SCHEMA}`"
        ));
    }
    require_text(&request.slice, "Slice cost slice")?;
    review_protocol::require_commit(&request.candidate_commit, "Slice cost candidate")?;
    for (label, basis) in [
        ("packet", &request.owners.packet.basis),
        ("provider", &request.owners.provider.basis),
        (
            "coordinator context",
            &request.owners.coordinator_context.basis,
        ),
        ("command output", &request.owners.command_output.basis),
        ("elapsed", &request.owners.elapsed.basis),
    ] {
        require_text(basis, &format!("{label} basis"))?;
    }
    validate_measurements(&request.owners)?;
    let command = &request.owners.command_output;
    if let (Some(complete), Some(returned)) = (
        command.complete_log_bytes.value(),
        command.returned_bytes.value(),
    ) && returned > complete
    {
        return Err("returned command bytes cannot exceed complete log bytes".to_owned());
    }
    let elapsed = &request.owners.elapsed;
    require_text(&elapsed.critical_bottleneck.name, "elapsed bottleneck name")?;
    if elapsed.critical_bottleneck.elapsed_milliseconds == 0 {
        return Err("elapsed bottleneck must be nonzero".to_owned());
    }
    if let Some(total) = elapsed.total_milliseconds.value()
        && total < elapsed.critical_bottleneck.elapsed_milliseconds
    {
        return Err("elapsed total cannot be smaller than its bottleneck".to_owned());
    }
    Ok(())
}

fn validate_measurements(owners: &Owners) -> Result<(), String> {
    let values = [
        ("packet rendered bytes", &owners.packet.rendered_bytes),
        ("packet managed tokens", &owners.packet.managed_tokens),
        ("provider input", &owners.provider.usage.input_tokens),
        ("provider output", &owners.provider.usage.output_tokens),
        ("provider total", &owners.provider.usage.total_tokens),
        (
            "provider reasoning",
            &owners.provider.usage.reasoning_tokens,
        ),
        (
            "provider cache read",
            &owners.provider.usage.cache_read_input_tokens,
        ),
        (
            "provider cache write",
            &owners.provider.usage.cache_write_input_tokens,
        ),
        (
            "coordinator input",
            &owners.coordinator_context.usage.input_tokens,
        ),
        (
            "coordinator output",
            &owners.coordinator_context.usage.output_tokens,
        ),
        (
            "coordinator total",
            &owners.coordinator_context.usage.total_tokens,
        ),
        (
            "coordinator reasoning",
            &owners.coordinator_context.usage.reasoning_tokens,
        ),
        (
            "coordinator cache read",
            &owners.coordinator_context.usage.cache_read_input_tokens,
        ),
        (
            "coordinator cache write",
            &owners.coordinator_context.usage.cache_write_input_tokens,
        ),
        (
            "command complete log",
            &owners.command_output.complete_log_bytes,
        ),
        ("command returned", &owners.command_output.returned_bytes),
        ("elapsed total", &owners.elapsed.total_milliseconds),
    ];
    for (label, value) in values {
        value.validate(label)?;
    }
    Ok(())
}

fn owner_sources(owners: &Owners) -> [(&'static str, &[Source]); 5] {
    [
        ("packet", &owners.packet.sources),
        ("provider", &owners.provider.sources),
        ("coordinator_context", &owners.coordinator_context.sources),
        ("command_output", &owners.command_output.sources),
        ("elapsed", &owners.elapsed.sources),
    ]
}

fn revalidate_sources(request: &Request, workspace: &Path) -> Result<(), String> {
    for (_, sources) in owner_sources(&request.owners) {
        for source in sources {
            let path = canonical_input(workspace, Path::new(&source.path))?;
            let bytes = bounded_file::read_regular(&path, SOURCE_LIMIT, "Slice cost source")?;
            if review_protocol::digest(&bytes) != source.hash {
                return Err(format!(
                    "Slice cost source changed before publication: {}",
                    path.display()
                ));
            }
        }
    }
    Ok(())
}

fn canonical_input(workspace: &Path, path: &Path) -> Result<std::path::PathBuf, String> {
    let resolved = review_protocol::resolve_input_path(workspace, &path.to_string_lossy());
    std::fs::canonicalize(&resolved).map_err(|error| {
        format!(
            "cannot resolve Slice cost input {}: {error}",
            resolved.display()
        )
    })
}

fn canonical_output(workspace: &Path, path: &Path) -> Result<std::path::PathBuf, String> {
    let resolved = review_protocol::resolve_input_path(workspace, &path.to_string_lossy());
    let parent = resolved
        .parent()
        .ok_or_else(|| "Slice cost output has no parent".to_owned())?;
    let parent = std::fs::canonicalize(parent)
        .map_err(|error| format!("cannot resolve Slice cost output parent: {error}"))?;
    let name = resolved
        .file_name()
        .ok_or_else(|| "Slice cost output has no file name".to_owned())?;
    Ok(parent.join(name))
}

fn require_hash(value: &str, label: &str) -> Result<(), String> {
    if value.strip_prefix("sha256:").is_some_and(|hash| {
        hash.len() == 64
            && hash
                .bytes()
                .all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f'))
    }) {
        Ok(())
    } else {
        Err(format!("{label} must be canonical SHA-256"))
    }
}

fn require_text(value: &str, label: &str) -> Result<(), String> {
    if value.trim().is_empty() || value.len() > 4096 || value.contains('\0') {
        Err(format!("{label} must be nonblank and bounded"))
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use serde_json::{Value, json};

    use super::{POLICY, run};
    use crate::{review_protocol::digest, test_support::TestRepository};

    struct Fixture {
        repository: TestRepository,
        request: std::path::PathBuf,
        output: std::path::PathBuf,
        source: std::path::PathBuf,
    }

    impl Fixture {
        fn new() -> Self {
            let repository = TestRepository::new("cost-report");
            repository.write("base", "base\n");
            repository.git(["add", "base"]);
            repository.git(["commit", "-qm", "base"]);
            let source = repository.path.join("usage.json");
            std::fs::write(&source, br#"{"schema":"example.usage/v1","value":1}"#).unwrap();
            let request = repository.path.join("request.json");
            let output = repository.path.join("report.json");
            let head = crate::git::output_in(&repository.path, &["rev-parse", "HEAD"], false)
                .unwrap()
                .trim()
                .to_owned();
            let source_ref = json!({
                "path": source,
                "hash": digest(&std::fs::read(&source).unwrap()),
                "schema": "example.usage/v1"
            });
            let reported = json!({"availability":"reported","value":10});
            let unavailable = json!({"availability":"unavailable","reason":"not exposed"});
            let document = json!({
                "schema": "yo.slice-cost-report-request/v1alpha1",
                "slice": "cost-report",
                "candidate_commit": head,
                "owners": {
                    "packet": {"basis":"manifest metrics","sources":[],"publication_count":1,"rendered_bytes":reported,"managed_tokens":{"availability":"reported","value":3}},
                    "provider": {"basis":"provider receipt","sources":[source_ref],"request_count":1,"usage":usage(reported.clone())},
                    "coordinator_context": {"basis":"host observation","sources":[],"usage":usage(unavailable)},
                    "command_output": {"basis":"bounded log","sources":[],"command_count":1,"complete_log_bytes":{"availability":"reported","value":100},"returned_bytes":{"availability":"reported","value":10}},
                    "elapsed": {"basis":"wall clock","sources":[],"total_milliseconds":{"availability":"reported","value":1000},"critical_bottleneck":{"name":"tests","elapsed_milliseconds":900}}
                }
            });
            std::fs::write(&request, serde_json::to_vec(&document).unwrap()).unwrap();
            Self {
                repository,
                request,
                output,
                source,
            }
        }

        fn document(&self) -> Value {
            serde_json::from_slice(&std::fs::read(&self.request).unwrap()).unwrap()
        }

        fn write(&self, value: &Value) {
            std::fs::write(&self.request, serde_json::to_vec(value).unwrap()).unwrap();
        }
    }

    fn usage(value: Value) -> Value {
        json!({
            "input_tokens": value,
            "output_tokens": {"availability":"reported","value":2},
            "total_tokens": {"availability":"partial","value":12,"reason":"one round absent"},
            "reasoning_tokens": {"availability":"unavailable","reason":"not exposed"},
            "cache_read_input_tokens": {"availability":"reported","value":4},
            "cache_write_input_tokens": {"availability":"unavailable","reason":"not exposed"}
        })
    }

    // report는 다섯 owner를 그대로 보존하며 서로 다른 단위의 grand total을 만들지 않는다.
    #[test]
    fn publishes_owner_separated_report_without_cross_owner_total() {
        let fixture = Fixture::new();
        run(&fixture.repository.path, &fixture.request, &fixture.output).unwrap();
        let report: Value =
            serde_json::from_slice(&std::fs::read(&fixture.output).unwrap()).unwrap();
        assert_eq!(report["aggregation_policy"], POLICY);
        assert!(report["owners"]["packet"].is_object());
        assert!(report.get("total").is_none());
        assert_eq!(report["source_artifacts"][0]["bytes"], 39);
    }

    // source bytes나 schema가 request의 content address와 다르면 수치를 발행하지 않는다.
    #[test]
    fn rejects_stale_hash_or_schema() {
        let fixture = Fixture::new();
        std::fs::write(
            &fixture.source,
            br#"{"schema":"example.usage/v1","value":2}"#,
        )
        .unwrap();
        assert!(
            run(&fixture.repository.path, &fixture.request, &fixture.output)
                .unwrap_err()
                .contains("hash changed")
        );

        let fixture = Fixture::new();
        let mut document = fixture.document();
        document["owners"]["provider"]["sources"][0]["schema"] = json!("other/v1");
        fixture.write(&document);
        assert!(
            run(&fixture.repository.path, &fixture.request, &fixture.output)
                .unwrap_err()
                .contains("schema changed")
        );
    }

    // 하나의 artifact를 서로 다른 owner에 넣어 같은 비용을 두 번 세는 요청은 닫힌다.
    #[test]
    fn rejects_cross_owner_double_counting() {
        let fixture = Fixture::new();
        let mut document = fixture.document();
        document["owners"]["packet"]["sources"] = document["owners"]["provider"]["sources"].clone();
        fixture.write(&document);
        assert!(
            run(&fixture.repository.path, &fixture.request, &fixture.output)
                .unwrap_err()
                .contains("more than once")
        );
    }

    // command 반환량과 elapsed bottleneck은 자기 owner의 complete 관측을 넘을 수 없다.
    #[test]
    fn rejects_inverted_command_or_elapsed_measurements() {
        let fixture = Fixture::new();
        let mut document = fixture.document();
        document["owners"]["command_output"]["returned_bytes"]["value"] = json!(101);
        fixture.write(&document);
        assert!(
            run(&fixture.repository.path, &fixture.request, &fixture.output)
                .unwrap_err()
                .contains("complete log")
        );

        let fixture = Fixture::new();
        let mut document = fixture.document();
        document["owners"]["elapsed"]["total_milliseconds"]["value"] = json!(899);
        fixture.write(&document);
        assert!(
            run(&fixture.repository.path, &fixture.request, &fixture.output)
                .unwrap_err()
                .contains("bottleneck")
        );
    }

    // conflicting output은 덮어쓰지 않지만 exact retry는 같은 report로 수렴한다.
    #[test]
    fn publication_is_create_only_and_exact_retry_is_idempotent() {
        let fixture = Fixture::new();
        run(&fixture.repository.path, &fixture.request, &fixture.output).unwrap();
        run(&fixture.repository.path, &fixture.request, &fixture.output).unwrap();
        std::fs::write(&fixture.output, b"conflict\n").unwrap();
        assert!(run(&fixture.repository.path, &fixture.request, &fixture.output).is_err());
        assert_eq!(std::fs::read(&fixture.output).unwrap(), b"conflict\n");
    }
}
