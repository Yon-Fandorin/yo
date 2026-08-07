use std::{env, process::ExitCode};

fn main() -> ExitCode {
    match xtask::run(env::args_os().skip(1)) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            if error.trim_start().starts_with('{') {
                eprintln!("{error}");
            } else {
                eprintln!("xtask: {error}");
            }
            ExitCode::FAILURE
        },
    }
}
