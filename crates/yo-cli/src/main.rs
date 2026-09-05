#[cfg(not(unix))]
use std::io::Write;
use std::process::ExitCode;

#[cfg(unix)]
fn main() -> ExitCode {
    yo_cli::run()
}

#[cfg(not(unix))]
fn main() -> ExitCode {
    let _ = writeln!(
        std::io::stderr().lock(),
        "yo: this build currently supports macOS and Linux only"
    );
    ExitCode::FAILURE
}
