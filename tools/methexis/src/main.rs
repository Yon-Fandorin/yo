use std::{
    env,
    io::{self, Write},
    process::ExitCode,
};

const IO_ERROR: &[u8] = b"\
{\"schema\":\"methexis.error/v1alpha1\",\"ok\":false,\"error\":{\"code\":\"io_error\",\"affected_ids\":[],\"next_actions\":[\"retry with writable stdout and stderr\"]}}
";

fn main() -> ExitCode {
    match methexis::run(
        env::args_os().skip(1),
        io::stdout().lock(),
        io::stderr().lock(),
    ) {
        Ok(code) => code,
        Err(_) => report_io_error(io::stderr().lock()),
    }
}

fn report_io_error(mut stderr: impl Write) -> ExitCode {
    let _ = stderr.write_all(IO_ERROR);
    ExitCode::FAILURE
}

#[cfg(test)]
mod tests {
    use std::process::ExitCode;

    use super::{IO_ERROR, report_io_error};

    // 출력 I/O 실패 뒤 main이 호출하는 fallback 함수는 정해진 io_error JSON을 stderr에 쓰고
    // FAILURE 코드를 반환한다. 여기서는 실제 스트림 실패가 아니라 fallback 함수의 결과를 검증한다.
    #[test]
    fn io_failure_has_a_structured_fallback() {
        let mut stderr = Vec::new();

        let code = report_io_error(&mut stderr);

        assert_eq!(code, ExitCode::FAILURE);
        assert_eq!(stderr, IO_ERROR);
    }
}
