use std::{
    io::{self, Write},
    process::ExitCode,
};

use serde::Serialize;

pub(super) fn write_text(
    writer: &mut impl Write,
    text: &str,
    exit_code: ExitCode,
) -> io::Result<ExitCode> {
    writer.write_all(text.as_bytes())?;
    Ok(exit_code)
}

pub(super) fn write_json(
    writer: &mut impl Write,
    value: &impl Serialize,
    exit_code: ExitCode,
) -> io::Result<ExitCode> {
    write_json_with(writer, value, exit_code, false)
}

pub(super) fn write_json_pretty(
    writer: &mut impl Write,
    value: &impl Serialize,
    exit_code: ExitCode,
) -> io::Result<ExitCode> {
    write_json_with(writer, value, exit_code, true)
}

fn write_json_with(
    writer: &mut impl Write,
    value: &impl Serialize,
    exit_code: ExitCode,
    pretty: bool,
) -> io::Result<ExitCode> {
    if pretty {
        serde_json::to_writer_pretty(&mut *writer, value).map_err(io::Error::other)?;
    } else {
        serde_json::to_writer(&mut *writer, value).map_err(io::Error::other)?;
    }
    writer.write_all(b"\n")?;
    Ok(exit_code)
}
