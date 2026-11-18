//! The binary: one command, a numbered picker, archives that open by double-clicking.

use std::process::ExitCode;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match wexport::ui::run(&args) {
        Ok(code) => ExitCode::from(code as u8),
        Err(error) => {
            eprintln!("wexport: {error}");
            ExitCode::from(error.code as u8)
        }
    }
}
