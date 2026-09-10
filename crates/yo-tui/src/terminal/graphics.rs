//! Kitty PNG transport. Cell layout and image decoding stay outside this owner.
use std::{env, process, str};

use base64::{Engine, engine::general_purpose::STANDARD};

use crate::surface::{FrameDiff, Surface};

fn first_id() -> u32 {
    0x7000_0000 | ((process::id() & 0x00ff_ffff) << 4)
}
const LIMIT: usize = 16;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) enum Graphics {
    #[default]
    Cells,
    Kitty,
    KittyTmux,
}

impl Graphics {
    pub(crate) fn from_env() -> Self {
        Self::detect(
            env::var("YO_TUI_IMAGE_PROTOCOL").ok().as_deref(),
            env::var("TERM_PROGRAM").ok().as_deref(),
            env::var_os("KITTY_WINDOW_ID").is_some(),
            env::var_os("TMUX").is_some(),
        )
    }

    fn detect(setting: Option<&str>, terminal: Option<&str>, kitty: bool, tmux: bool) -> Self {
        match setting {
            Some("off" | "cells") => Self::Cells,
            Some("kitty-tmux") => Self::KittyTmux,
            Some("kitty") if !tmux => Self::Kitty,
            None | Some("auto")
                if !tmux
                    && (kitty || matches!(terminal, Some("ghostty" | "WezTerm" | "kitty"))) =>
            {
                Self::Kitty
            },
            _ => Self::Cells,
        }
    }

    fn command(self, bytes: &mut Vec<u8>, command: &str) {
        if self == Self::KittyTmux {
            bytes.extend_from_slice(b"\x1bPtmux;");
        }
        let command = format!("\x1b_G{command}\x1b\\");
        for byte in command.bytes() {
            bytes.push(byte);
            if self == Self::KittyTmux && byte == 0x1b {
                bytes.push(byte);
            }
        }
        if self == Self::KittyTmux {
            bytes.extend_from_slice(b"\x1b\\");
        }
    }

    pub(crate) fn clear(self, bytes: &mut Vec<u8>) {
        if self == Self::Cells {
            return;
        }
        // Delete only yo's reserved images, never another application's images.
        for index in 0..LIMIT {
            self.command(
                bytes,
                &format!("a=d,d=I,i={},q=2", first_id() + index as u32),
            );
        }
    }

    pub(crate) fn prepare(
        self,
        previous: Option<&Surface>,
        current: &Surface,
        diff: &FrameDiff<'_>,
        bytes: &mut Vec<u8>,
    ) {
        if self == Self::Cells {
            return;
        }
        let same = previous.is_some_and(|surface| surface.rasters == current.rasters);
        let repaint = current.rasters.iter().any(|image| {
            diff.spans().iter().any(|span| {
                span.row() >= image.area.origin.y
                    && span.row() < image.area.origin.y + image.area.size.height
            })
        });
        if same && !repaint {
            return;
        }
        if current.rasters.is_empty() && previous.is_none_or(|surface| surface.rasters.is_empty()) {
            return;
        }
        if previous.is_some() {
            self.clear(bytes);
        }
        for (index, image) in current.rasters.iter().take(LIMIT).enumerate() {
            let encoded = STANDARD.encode(&image.png);
            // Terminal coordinates stay outside tmux passthrough so tmux translates panes.
            bytes.extend_from_slice(
                format!(
                    "\x1b[{};{}H",
                    image.area.origin.y + 1,
                    image.area.origin.x + 1
                )
                .as_bytes(),
            );
            let chunks = encoded.as_bytes().chunks(4096);
            let count = chunks.len();
            for (part, chunk) in chunks.enumerate() {
                let more = usize::from(part + 1 < count);
                let header = if part == 0 {
                    format!(
                        "a=T,f=100,i={},c={},r={},C=1,q=2,m={more};",
                        first_id() + index as u32,
                        image.area.size.width,
                        image.area.size.height
                    )
                } else {
                    format!("m={more};")
                };
                self.command(
                    bytes,
                    &(header + str::from_utf8(chunk).expect("base64 is ASCII")),
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::surface::{Point, RasterImage, Rect, Size};

    // 외부 terminal을 알 수 없는 tmux에서는 추측으로 APC를 보내지 않는다.
    #[test]
    fn tmux_requires_explicit_transport_selection() {
        assert_eq!(
            Graphics::detect(None, Some("ghostty"), false, false),
            Graphics::Kitty
        );
        assert_eq!(
            Graphics::detect(None, Some("ghostty"), false, true),
            Graphics::Cells
        );
        assert_eq!(
            Graphics::detect(Some("kitty-tmux"), None, false, true),
            Graphics::KittyTmux
        );
        assert_eq!(
            Graphics::detect(Some("off"), Some("kitty"), true, false),
            Graphics::Cells
        );
    }

    // 원본 PNG 바이트를 chunk로 전달하고 tmux ESC를 이중화한다. fallback 모드는 출력하지 않는다.
    #[test]
    fn png_transport_is_chunked_and_scoped_to_owned_ids() {
        let mut surface = Surface::new(Size::new(20, 10)).unwrap();
        surface.rasters.push(RasterImage {
            area: Rect::new(Point::new(2, 1), Size::new(8, 4)),
            png: vec![7; 5000].into(),
        });
        let diff = FrameDiff::complete(surface.size(), &surface);
        let mut bytes = Vec::new();
        Graphics::KittyTmux.prepare(None, &surface, &diff, &mut bytes);
        let output = String::from_utf8(bytes).unwrap();
        assert!(output.contains("\x1bPtmux;\x1b\x1b_G"));
        assert!(output.contains("c=8,r=4,C=1,q=2,m=1;"));
        assert!(output.contains("m=0;"));
        assert!(!output.contains("d=A"));
        let mut bytes = Vec::new();
        Graphics::Cells.prepare(None, &surface, &diff, &mut bytes);
        assert!(bytes.is_empty());
    }
}
