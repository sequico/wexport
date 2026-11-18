//! The binary: one command, a list of chats with a code each, archives that open by double-clicking.

use std::process::ExitCode;

fn main() -> ExitCode {
    // A listing piped into `head` or `grep -q` ends with a closed pipe. Every other tool on the
    // machine dies quietly there; Rust turns the failed write into a panic instead.
    #[cfg(unix)]
    // SAFETY: installing the default disposition for SIGPIPE touches no memory of ours.
    unsafe {
        libc::signal(libc::SIGPIPE, libc::SIG_DFL);
    }
    let args: Vec<String> = std::env::args().skip(1).collect();
    match wexport::ui::run(&args) {
        Ok(code) => ExitCode::from(code as u8),
        Err(error) => {
            eprintln!("wexport: {error}");
            ExitCode::from(error.code as u8)
        }
    }
}
