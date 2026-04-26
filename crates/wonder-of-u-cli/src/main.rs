use std::process::ExitCode;

fn main() -> ExitCode {
    let mut stdout = std::io::stdout();
    match wonder_of_u_cli::run_from(std::env::args_os(), &mut stdout) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("error: {error}");
            ExitCode::FAILURE
        }
    }
}
