use std::{env, process::ExitCode};

fn main() -> ExitCode {
    match xtask::run(env::args_os().skip(1)) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("xtask: {error}");
            ExitCode::FAILURE
        },
    }
}
