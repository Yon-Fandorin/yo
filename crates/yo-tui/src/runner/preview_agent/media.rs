use std::{
    fs::{self, File},
    io::{self, Cursor, Read},
};

use base64::{Engine, engine::general_purpose::STANDARD};
use image::{ImageFormat, Rgb, RgbImage, imageops::rotate270};
use serde_json::json;
use yo_core::{MessageContent, ToolOutput};

pub(in crate::runner) fn media_response(input: &str) -> Option<String> {
    match input {
        "footnotes" => Some("## Notes and sources\n\nOne claim[^source] and a second reference[^source].\n\n[^source]: Read [documentation](https://example.com/docs) and preserve `literal code`.\n\n    - Nested **detail** stays with this note.\n\nA Korean note[^출처].\n\n[^출처]: 한글 설명도 좁은 화면에서 줄바꿈합니다.\n\nUnresolved reference stays literal: [^missing].\n\n```text\n[^source]: this code stays literal\n```".into()),
        "syntax" => Some("## Code, with context\n\n```rust\n// Preserve indentation and comments\nfn greet(name: &str) -> String {\n    format!(\"Hello, {name}!\")\n}\n```\n\n```python\n# A small transformation\ndef totals(values):\n    return [value * 2 for value in values]\n```\n\n```json\n{\n  \"theme\": \"slate\",\n  \"enabled\": true,\n  \"retries\": 3\n}\n```".into()),
        "chart-series" => Some("## Shared-scale comparison\n\n```linechart\nheight: 6\nBaseline: 12 18 15 24 21\nCurrent: 8 12 22 18 30\nTarget: 20 20 20 20 20\n```\n\nNumbered points match the legend. Crosses mark plotted overlaps; mixed cells are muted.".into()),
        "chart-heights" => Some("## Compact trend\n\n```linechart\nheight: 3\n12 8 16 10 24\n```\n\n## Detailed relationship\n\n```scatterchart\nheight: 10\n0,0 1,5 10,10\n```".into()),
        "scatter" => Some("## Input size and latency\n\nX: kilobytes · Y: milliseconds\n\n```scatterchart\n4,12 8,18 16,17 32,26 64,31\n```\n\n## Fixed input size\n\n```scatterchart\n5,10 5,20 5,30\n```".into()),
        "histogram" => Some("## Request latency distribution\n\nMilliseconds · interval counts\n\n```histogram\nbins: 4\n12 14 15 15 18 20 21 22 25 27 29 32\n```\n\n## Constant values\n\n```histogram\n5 5 5\n```".into()),
        "charts" => Some("## Rendering checks\n\nMeasured examples · milliseconds\n\n```chart\nMarkdown: 18\nTables: 12\nCode: 24\nImages: 31\n```\n\n## Latency over time\n\n```linechart\n31 27 29 22 18 21 16 12\n```\n\n## Concurrent tasks\n\n```stepchart\n1 1 4 4 2 2 0\n```\n\n## Change from baseline\n\n```chart\nBefore: -12\nSame: 0\nAfter: 8\n```\n\n## Scientific notation\n\n```linechart\n0 1e308 -1e308\n```\n\n## Very small values\n\n```sparkline\n0 5e-324\n```\n\n## Numeric notation\n\n```chart\nTiny: 5e-324\nDecimal: 1.2300\nSigned: +2.00\nZero: -0.0\n```".into()),
        "images" => Some(sample_image()),
        "message-image" => MessageContent {
            block: json!({"type":"image","mimeType":"image/png","data":sample_image_data(),"_meta":{"source":"offline answer block"}}),
        }.to_snapshot(),
        "image-orientation" => Some(sample_oriented_image()),
        "media-errors" => Some("## Useful fallbacks\n\n![Incomplete image](data:image/png;base64,broken)\n\n![External reference](https://example.com/diagram.png)\n\n```chart\nReading: pending\n```\n\n```unknown-language\nOriginal source stays readable.\n```".into()),
        "showcase" => Some(["syntax", "charts", "images", "media-errors"].into_iter().filter_map(media_response).collect::<Vec<_>>().join("\n\n")),
        _ => input.strip_prefix("image ").map(local_image),
    }
}

pub(super) fn sample_batch_read() -> String {
    let result = json!({"results":[
        {"path":"src/main.rs","status":"ok","start":2,"end":4,"total":9,"next_offset":5,"content":"fn main() {\n    println!(\"batch preview\");\n}\n"},
        {"path":"missing.txt","status":"error","error":"unavailable"},
        {"path":"empty.txt","status":"ok","start":0,"end":0,"total":0,"content":""}
    ]}).to_string();
    ToolOutput {
        tool:"read_files".to_owned(), server:Some("preview".to_owned()),
        arguments:Some(json!({"files":[{"path":"src/main.rs"},{"path":"missing.txt"},{"path":"empty.txt"}]})),
        result:Some(json!({"content":[{"type":"text","text":result}],"note":"Offline fixture: no files were read."})),
        content_items:None, error:None,
        plain_text:format!("preview.read_files\n{result}\nOffline fixture: no files were read."),
    }.to_snapshot().expect("bounded batch read fixture")
}

pub(super) fn sample_tool_image() -> String {
    let data = sample_image_data();
    ToolOutput {
        tool: "capture".to_owned(), server: Some("preview".to_owned()),
        arguments: Some("offline fixture".into()),
        result: Some(format!(r#"{{"content":[{{"type":"text","text":"Preview only: no external tool was called."}},{{"type":"image","mimeType":"image/png","data":"{data}"}}]}}"#).parse().expect("fixture result JSON")),
        content_items: None, error: None,
        plain_text: "preview.capture\nOffline PNG image · 128 × 64".to_owned(),
    }.to_snapshot().expect("bounded tool output fixture")
}

fn sample_image() -> String {
    format!(
        "## Image in the conversation\n\n![Mountain study](data:image/png;base64,{})\n\nThe image scrolls with the answer. Try resizing the pane.\n\nUse `image /absolute/path.png` here to inspect your own local PNG or JPEG.",
        sample_image_data()
    )
}

fn sample_oriented_image() -> String {
    let upright = RgbImage::from_fn(64, 128, |x, y| {
        if (i64::from(x) - 46).pow(2) + (i64::from(y) - 22).pow(2) < 90 {
            Rgb([244, 204, 119])
        } else if y > 75 {
            Rgb([38, 103, 102])
        } else {
            Rgb([62, 86, 140])
        }
    });
    let mut stored = Cursor::new(Vec::new());
    rotate270(&upright)
        .write_to(&mut stored, ImageFormat::Jpeg)
        .expect("small JPEG fixture");
    let stored = stored.into_inner();
    // Little-endian TIFF Orientation=6: restore the stored landscape to upright portrait.
    let exif = b"Exif\0\0II\x2a\0\x08\0\0\0\x01\0\x12\x01\x03\0\x01\0\0\0\x06\0\0\0\0\0\0\0";
    let mut jpeg = stored[..2].to_vec();
    jpeg.extend_from_slice(&[0xff, 0xe1]);
    jpeg.extend_from_slice(&u16::try_from(exif.len() + 2).unwrap().to_be_bytes());
    jpeg.extend_from_slice(exif);
    jpeg.extend_from_slice(&stored[2..]);
    format!(
        "## JPEG orientation\n\n![Upright portrait](data:image/jpeg;base64,{})\n\nEXIF rotates the stored 128 x 64 JPEG to 64 x 128. The sun belongs at the top right, ground at the bottom.",
        STANDARD.encode(jpeg)
    )
}

pub(super) fn sample_image_data() -> String {
    // A deterministic landscape fixture is encoded as a real PNG, then follows
    // exactly the same decoder and cell renderer as an attached image.
    let image = RgbImage::from_fn(128, 64, |x, y| {
        let sky = Rgb([35 + (y * 2) as u8, 66 + y as u8, 102 + y as u8]);
        if (i64::from(x) - 95).pow(2) + (i64::from(y) - 17).pow(2) < 70 {
            Rgb([244, 204, 119])
        } else if y > (38.0 + (f64::from(x) / 12.0).sin() * 8.0) as u32 {
            Rgb([38, 103, 102])
        } else if y > (29.0 + (f64::from(x) / 17.0).sin() * 9.0) as u32 {
            Rgb([62, 86, 110])
        } else {
            sky
        }
    });
    let mut bytes = Cursor::new(Vec::new());
    image
        .write_to(&mut bytes, ImageFormat::Png)
        .expect("small in-memory PNG fixture");
    STANDARD.encode(bytes.into_inner())
}

fn local_image(path: &str) -> String {
    let path = path.trim();
    let load = || -> io::Result<Vec<u8>> {
        let metadata = fs::metadata(path)?;
        if !metadata.is_file() || metadata.len() > 1_048_576 {
            return Err(io::Error::other(
                "choose a regular PNG/JPEG file no larger than 1 MiB",
            ));
        }
        let mut bytes = Vec::new();
        File::open(path)?.take(1_048_577).read_to_end(&mut bytes)?;
        if bytes.len() > 1_048_576 {
            return Err(io::Error::other("image exceeds 1 MiB"));
        }
        Ok(bytes)
    };
    match load() {
        Ok(bytes) => format!(
            "![Local image](data:image/png;base64,{})",
            STANDARD.encode(bytes)
        ),
        Err(error) => format!(
            "Image preview unavailable.\n\n{error}\n\nTry `image /absolute/path.png` with a readable PNG or JPEG."
        ),
    }
}
