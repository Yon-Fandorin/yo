//! Bounded, typed summary source derived from admitted semantic replay.

use std::io::{self, Write};

use serde::Serialize;

use crate::{ModelInputPart, ModelReplayItem, ModelReplayRole};

const MAX_SOURCE_BYTES: usize = 16 * 1024 * 1024;
const MAX_IMAGES: usize = 64;

/// A summary-only user message. Construction derives its manifest and images
/// together, so neither a caller-authored manifest nor a path can select pixels.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ImageSummarySource {
    parts: Vec<ModelInputPart>,
}

#[derive(Serialize)]
struct Manifest<'a> {
    schema: &'static str,
    history: Vec<History<'a>>,
}

#[derive(Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum History<'a> {
    Message {
        role: &'static str,
        content: &'a str,
        refusal: &'a Option<String>,
    },
    MultimodalUser {
        parts: Vec<ManifestPart<'a>>,
    },
    FunctionCall {
        call_id: &'a str,
        name: &'a str,
        arguments: &'a str,
    },
    FunctionCallOutput {
        call_id: &'a str,
        output: &'a str,
    },
}

#[derive(Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum ManifestPart<'a> {
    Text {
        text: &'a str,
    },
    ImageRef {
        index: u32,
        sha256: &'a str,
        byte_length: usize,
        width: u32,
        height: u32,
    },
}

#[derive(Serialize)]
struct Source<'a> {
    role: &'static str,
    parts: &'a [ModelInputPart],
}

impl ImageSummarySource {
    /// Preserves source occurrence order, including repeated snapshots. The
    /// caller remains responsible for selecting complete committed replay groups.
    pub fn from_replay_groups(groups: &[Vec<ModelReplayItem>]) -> Result<Self, &'static str> {
        let mut history = Vec::new();
        let mut images = Vec::new();
        let mut png_bytes = 0_usize;
        let mut item_count = 0_usize;
        for item in groups.iter().flatten() {
            item_count = item_count
                .checked_add(1)
                .ok_or("summary item count overflow")?;
            if item_count > 4096 {
                return Err("summary source exceeds replay item limit");
            }
            history.push(match item {
                ModelReplayItem::Message {
                    role,
                    content,
                    refusal,
                } => {
                    if content.len() > MAX_SOURCE_BYTES
                        || (refusal.is_some() && *role != ModelReplayRole::Assistant)
                    {
                        return Err("summary source contains an invalid message");
                    }
                    let role = match role {
                        ModelReplayRole::System => "system",
                        ModelReplayRole::Developer => "developer",
                        ModelReplayRole::User => "user",
                        ModelReplayRole::Assistant => "assistant",
                    };
                    History::Message {
                        role,
                        content,
                        refusal,
                    }
                },
                ModelReplayItem::MultimodalUser { parts } => {
                    ModelInputPart::validate_user_parts(parts)?;
                    let mut manifest_parts = Vec::with_capacity(parts.len());
                    for part in parts {
                        manifest_parts.push(match part {
                            ModelInputPart::Text { text } => ManifestPart::Text { text },
                            ModelInputPart::Image { snapshot } => {
                                if images.len() == MAX_IMAGES {
                                    return Err("summary source exceeds 64 images");
                                }
                                png_bytes = png_bytes
                                    .checked_add(snapshot.png().len())
                                    .ok_or("summary image bytes overflow")?;
                                if png_bytes > MAX_SOURCE_BYTES {
                                    return Err("summary image bytes exceed 16 MiB");
                                }
                                let index = images.len() as u32;
                                images.push(part.clone());
                                ManifestPart::ImageRef {
                                    index,
                                    sha256: snapshot.sha256(),
                                    byte_length: snapshot.png().len(),
                                    width: snapshot.width(),
                                    height: snapshot.height(),
                                }
                            },
                        });
                    }
                    History::MultimodalUser {
                        parts: manifest_parts,
                    }
                },
                ModelReplayItem::FunctionCall {
                    call_id,
                    name,
                    arguments,
                } => History::FunctionCall {
                    call_id,
                    name,
                    arguments,
                },
                ModelReplayItem::FunctionCallOutput { call_id, output } => {
                    History::FunctionCallOutput { call_id, output }
                },
                ModelReplayItem::ProviderPrivateAssistant { .. } => continue,
            });
        }
        if images.is_empty() {
            return Err("image summary source requires an image");
        }
        let mut manifest = BoundedWriter::new(Vec::new());
        serde_json::to_writer(
            &mut manifest,
            &Manifest {
                schema: "yo.image-summary-source/v1",
                history,
            },
        )
        .map_err(|_| "encoded summary manifest exceeds 16 MiB")?;
        let text = String::from_utf8(manifest.inner).expect("JSON serializer emits UTF-8");
        let mut parts = Vec::with_capacity(images.len() + 1);
        parts.push(ModelInputPart::Text { text });
        parts.extend(images);
        let source = Self { parts };
        serde_json::to_writer(
            BoundedWriter::new(io::sink()),
            &Source {
                role: "user",
                parts: &source.parts,
            },
        )
        .map_err(|_| "complete encoded image summary source exceeds 16 MiB")?;
        Ok(source)
    }

    pub fn parts(&self) -> &[ModelInputPart] {
        &self.parts
    }
}

struct BoundedWriter<W> {
    inner: W,
    written: usize,
}

impl<W> BoundedWriter<W> {
    const fn new(inner: W) -> Self {
        Self { inner, written: 0 }
    }
}

impl<W: Write> Write for BoundedWriter<W> {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        let next = self
            .written
            .checked_add(bytes.len())
            .filter(|size| *size <= MAX_SOURCE_BYTES)
            .ok_or_else(|| io::Error::other("image summary source exceeds encoded byte limit"))?;
        self.inner.write_all(bytes)?;
        self.written = next;
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        self.inner.flush()
    }
}

#[cfg(test)]
mod tests;
