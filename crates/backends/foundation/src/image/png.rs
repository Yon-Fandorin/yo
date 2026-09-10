//! Structural verification of an already canonical RGBA8 PNG. No source
//! decoding, normalization, color conversion or live admission happens here.

use std::io::{self, BufRead, Read};

use crc32fast::hash as png_chunk_crc;
use flate2::{Decompress, FlushDecompress, Status};

use super::InputImageSnapshotError;

pub(super) fn validate_png(
    png: &[u8],
    width: u32,
    height: u32,
) -> Result<(), InputImageSnapshotError> {
    let invalid = InputImageSnapshotError::Encoding;
    if png.get(..8) != Some(b"\x89PNG\r\n\x1a\n") {
        return Err(invalid);
    }
    let mut cursor = 8_usize;
    let mut header = false;
    let mut data_bytes = 0_usize;
    while cursor < png.len() {
        let length_bytes: [u8; 4] = png
            .get(cursor..cursor + 4)
            .ok_or(invalid)?
            .try_into()
            .unwrap();
        let length = u32::from_be_bytes(length_bytes) as usize;
        let end = cursor
            .checked_add(12)
            .and_then(|n| n.checked_add(length))
            .ok_or(invalid)?;
        let chunk = png.get(cursor..end).ok_or(invalid)?;
        let kind = &chunk[4..8];
        let payload = &chunk[8..8 + length];
        let crc = u32::from_be_bytes(chunk[8 + length..].try_into().unwrap());
        if png_chunk_crc(&chunk[4..8 + length]) != crc {
            return Err(invalid);
        }
        match kind {
            b"IHDR" if !header && cursor == 8 && length == 13 => {
                if payload[..4] != width.to_be_bytes() || payload[4..8] != height.to_be_bytes() {
                    return Err(InputImageSnapshotError::Dimensions);
                }
                if payload[8..] != [8, 6, 0, 0, 0] {
                    return Err(invalid);
                }
                header = true;
            },
            b"IDAT" if header => data_bytes += length,
            b"IEND" if header && data_bytes > 0 && length == 0 && end == png.len() => {
                return validate_pixels(png, width, height);
            },
            _ => return Err(invalid),
        }
        cursor = end;
    }
    Err(invalid)
}

fn validate_pixels(png: &[u8], width: u32, height: u32) -> Result<(), InputImageSnapshotError> {
    let invalid = InputImageSnapshotError::Encoding;
    let mut input = IdatReader {
        png,
        chunk: 8,
        position: 0,
    };
    let mut decoder = Decompress::new(true);
    let row_bytes = 1 + u64::from(width) * 4;
    let expected = row_bytes * u64::from(height);
    let mut decoded = 0_u64;
    let mut output = [0_u8; 8192];
    loop {
        let previous_in = decoder.total_in();
        let previous_out = decoder.total_out();
        let capacity = output.len().min((expected - decoded + 1) as usize);
        let status = decoder
            .decompress(
                input.fill_buf().map_err(|_| invalid)?,
                &mut output[..capacity],
                FlushDecompress::None,
            )
            .map_err(|_| invalid)?;
        let consumed = (decoder.total_in() - previous_in) as usize;
        let produced = decoder.total_out() - previous_out;
        input.consume(consumed);
        if produced > expected - decoded {
            return Err(invalid);
        }
        for (index, byte) in output[..produced as usize].iter().enumerate() {
            if (decoded + index as u64).is_multiple_of(row_bytes) && *byte > 4 {
                return Err(invalid);
            }
        }
        decoded += produced;
        // Read wrappers can report EOF before zlib's checksum has completed.
        // Only explicit StreamEnd proves the complete compressed stream.
        if status == Status::StreamEnd {
            return if decoded == expected && input.fill_buf().map_err(|_| invalid)?.is_empty() {
                Ok(())
            } else {
                Err(invalid)
            };
        }
        if consumed == 0 && produced == 0 {
            return Err(invalid);
        }
    }
}

/// Borrows IDAT chunks already checked by validate_png without concatenating or
/// retaining a second compressed buffer or an unbounded list of chunk slices.
struct IdatReader<'a> {
    png: &'a [u8],
    chunk: usize,
    position: usize,
}

impl BufRead for IdatReader<'_> {
    fn fill_buf(&mut self) -> io::Result<&[u8]> {
        while self.chunk < self.png.len() {
            let length =
                u32::from_be_bytes(self.png[self.chunk..self.chunk + 4].try_into().unwrap())
                    as usize;
            let start = self.chunk + 8;
            if &self.png[self.chunk + 4..start] == b"IDAT" && self.position < length {
                return Ok(&self.png[start + self.position..start + length]);
            }
            self.chunk += length + 12;
            self.position = 0;
        }
        Ok(&[])
    }

    fn consume(&mut self, amount: usize) {
        self.position += amount;
    }
}

impl Read for IdatReader<'_> {
    fn read(&mut self, output: &mut [u8]) -> io::Result<usize> {
        let available = self.fill_buf()?;
        let count = available.len().min(output.len());
        output[..count].copy_from_slice(&available[..count]);
        self.consume(count);
        Ok(count)
    }
}
