//! Failures that carry the message a person reads and the exit code a script reads.

use std::fmt;

pub const EXIT_INPUT: i32 = 1;
pub const EXIT_KEY: i32 = 2;
pub const EXIT_TYPE: i32 = 3;
pub const EXIT_INCOMPLETE: i32 = 4;
pub const EXIT_DEPENDENCY: i32 = 5;

#[derive(Debug)]
pub struct Error {
    pub message: String,
    pub code: i32,
}

impl Error {
    pub fn input(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            code: EXIT_INPUT,
        }
    }

    pub fn key(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            code: EXIT_KEY,
        }
    }

    pub fn wrong_type(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            code: EXIT_TYPE,
        }
    }

    pub fn dependency(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            code: EXIT_DEPENDENCY,
        }
    }

    pub fn incomplete(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            code: EXIT_INCOMPLETE,
        }
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl std::error::Error for Error {}

impl From<std::io::Error> for Error {
    fn from(error: std::io::Error) -> Self {
        Error::input(error.to_string())
    }
}

impl From<rusqlite::Error> for Error {
    fn from(error: rusqlite::Error) -> Self {
        Error::input(error.to_string())
    }
}

impl From<serde_json::Error> for Error {
    fn from(error: serde_json::Error) -> Self {
        Error::input(error.to_string())
    }
}

pub type Result<T> = std::result::Result<T, Error>;

/// The install command for the platforms the project supports, one line each.
pub fn install_hints(name: &str) -> String {
    let (debian, fedora, arch, windows) = if name == "tesseract" {
        (
            "sudo apt install tesseract-ocr",
            "sudo dnf install tesseract",
            "sudo pacman -S tesseract",
            "winget install UB-Mannheim.TesseractOCR",
        )
    } else {
        (
            "sudo apt install ffmpeg",
            "sudo dnf install ffmpeg",
            "sudo pacman -S ffmpeg",
            "winget install Gyan.FFmpeg",
        )
    };
    format!("  Debian/Ubuntu   {debian}\n  Fedora/RHEL     {fedora}\n  Arch            {arch}\n  Windows         {windows}")
}

pub fn missing_binary(name: &str) -> Error {
    Error::dependency(format!(
        "{name} not found on PATH.\n{}\nThen re-run the same command.",
        install_hints(name)
    ))
}
