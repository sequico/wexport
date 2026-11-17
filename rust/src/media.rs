//! Finding the file a row points at, and the derivatives the viewer needs.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;

use sha2::{Digest, Sha256};

use crate::error::{Error, Result};

pub const PEAKS: usize = 500;

#[derive(Debug, Clone)]
pub struct Placed {
    pub sha256: String,
    pub rel_path: String,
}

#[derive(Debug, Default, Clone)]
pub struct Derived {
    pub width: Option<i64>,
    pub height: Option<i64>,
    pub duration_ms: Option<i64>,
    pub thumb: Option<String>,
    pub peaks: Option<Vec<u8>>,
}

/// The file on disk for a database path, or None when it was not copied.
///
/// The path comes from the backup, so it is a trust boundary: a row pointing at `../../etc/passwd`
/// must not be able to pull a local file into the archive.
pub fn source_for(rel_path: &str, media_root: Option<&Path>, copy_root: &Path) -> Option<PathBuf> {
    let rel = rel_path.replace('\\', "/");
    let rel = rel.trim_start_matches('/');
    let mut candidates: Vec<PathBuf> = Vec::new();
    let mut roots: Vec<PathBuf> = vec![copy_root.to_path_buf()];
    if let Some(media_root) = media_root {
        roots.push(media_root.to_path_buf());
        if let Some(tail) = rel.split_once("Media/").map(|(_, tail)| tail) {
            candidates.push(media_root.join(tail));
        }
    }
    for root in &roots {
        candidates.push(root.join(rel));
    }
    for candidate in candidates {
        let Ok(resolved) = candidate.canonicalize() else {
            continue;
        };
        if !roots.iter().any(|root| {
            root.canonicalize()
                .map(|root| resolved.starts_with(root))
                .unwrap_or(false)
        }) {
            continue;
        }
        if resolved.is_file() {
            return Some(resolved);
        }
    }
    None
}

pub fn sha256_of(path: &Path) -> Result<String> {
    let bytes = std::fs::read(path)?;
    let mut digest = Sha256::new();
    digest.update(&bytes);
    Ok(format!("{:x}", digest.finalize()))
}

/// Copy (or hardlink) the file under its content address: one file per content, ever.
pub fn place(source: &Path, archive_root: &Path, link: bool) -> Result<Placed> {
    let digest = sha256_of(source)?;
    let extension = source
        .extension()
        .and_then(|value| value.to_str())
        .map(|value| format!(".{}", value.to_lowercase()))
        .unwrap_or_default();
    let rel_path = format!("media/{}/{}{}", &digest[..2], digest, extension);
    let destination = archive_root.join(&rel_path);
    if destination.exists() {
        return Ok(Placed {
            sha256: digest,
            rel_path,
        });
    }
    std::fs::create_dir_all(destination.parent().unwrap_or(archive_root))?;
    if link && std::fs::hard_link(source, &destination).is_ok() {
        return Ok(Placed {
            sha256: digest,
            rel_path,
        });
    }
    // Write beside the target and rename, so a concurrent worker never exposes a partial file.
    let partial = destination.with_extension(format!("{}.part", std::process::id()));
    std::fs::copy(source, &partial)?;
    std::fs::rename(&partial, &destination)?;
    Ok(Placed {
        sha256: digest,
        rel_path,
    })
}

pub fn derive(kind: &str, source: &Path, archive_root: &Path, sha: &str) -> Result<Derived> {
    match kind {
        "image" => image_derivative(kind, source, archive_root, sha),
        "sticker" => Ok(Derived::default()),
        "video" | "gif" => video_derivative(source, archive_root, sha),
        "audio" => audio_derivative(source, archive_root, sha),
        _ => Ok(Derived::default()),
    }
}

fn image_derivative(kind: &str, source: &Path, archive_root: &Path, sha: &str) -> Result<Derived> {
    let _ = kind;
    let rel = format!("deriv/{sha}.webp");
    let target = archive_root.join(&rel);
    std::fs::create_dir_all(target.parent().unwrap_or(archive_root))?;
    run_ffmpeg(&[
        "-i".into(),
        source.display().to_string(),
        "-vf".into(),
        "scale='min(320,iw)':-2".into(),
        "-y".into(),
        target.display().to_string(),
    ])?;
    let (width, height) = probe_dimensions(source)?;
    Ok(Derived {
        width,
        height,
        thumb: Some(rel),
        ..Derived::default()
    })
}

fn video_derivative(source: &Path, archive_root: &Path, sha: &str) -> Result<Derived> {
    let rel = format!("deriv/{sha}.jpg");
    let target = archive_root.join(&rel);
    std::fs::create_dir_all(target.parent().unwrap_or(archive_root))?;
    run_ffmpeg(&[
        "-i".into(),
        source.display().to_string(),
        "-frames:v".into(),
        "1".into(),
        "-vf".into(),
        "scale=480:-2".into(),
        "-pix_fmt".into(),
        "yuvj420p".into(),
        "-q:v".into(),
        "3".into(),
        "-y".into(),
        target.display().to_string(),
    ])?;
    let probe = probe_json(source)?;
    let stream = probe.get("streams").and_then(|streams| streams.get(0));
    let duration = probe
        .get("format")
        .and_then(|format| format.get("duration"))
        .and_then(|value| value.as_str())
        .and_then(|value| value.parse::<f64>().ok())
        .or_else(|| {
            stream
                .and_then(|stream| stream.get("duration"))
                .and_then(|value| value.as_str())
                .and_then(|value| value.parse::<f64>().ok())
        });
    Ok(Derived {
        width: stream
            .and_then(|stream| stream.get("width"))
            .and_then(|value| value.as_i64()),
        height: stream
            .and_then(|stream| stream.get("height"))
            .and_then(|value| value.as_i64()),
        duration_ms: duration.map(|seconds| (seconds * 1000.0) as i64),
        thumb: Some(rel),
        peaks: None,
    })
}

fn audio_derivative(source: &Path, archive_root: &Path, sha: &str) -> Result<Derived> {
    let raw = archive_root.join(format!("deriv/{sha}.{}.pcm", std::process::id()));
    std::fs::create_dir_all(raw.parent().unwrap_or(archive_root))?;
    let status = Command::new("ffmpeg")
        .args([
            "-nostdin",
            "-v",
            "error",
            "-i",
            &source.display().to_string(),
            "-ac",
            "1",
            "-ar",
            "4000",
            "-f",
            "s16le",
            "-y",
            &raw.display().to_string(),
        ])
        .status();
    if !status.map(|status| status.success()).unwrap_or(false) {
        let _ = std::fs::remove_file(&raw);
        return Err(Error::input("ffmpeg could not decode the voice note"));
    }
    let pcm = std::fs::read(&raw)?;
    let _ = std::fs::remove_file(&raw);
    let peaks = peaks_of(&pcm);
    Ok(Derived {
        peaks: Some(peaks),
        thumb: None,
        ..Derived::default()
    })
}

fn peaks_of(pcm: &[u8]) -> Vec<u8> {
    let samples: Vec<i16> = pcm
        .chunks_exact(2)
        .map(|pair| i16::from_le_bytes([pair[0], pair[1]]))
        .collect();
    if samples.len() < PEAKS {
        return vec![0; PEAKS];
    }
    let step = samples.len() / PEAKS;
    (0..PEAKS)
        .map(|index| {
            let window = &samples[index * step..(index + 1) * step];
            let peak = window
                .iter()
                .map(|value| value.unsigned_abs())
                .max()
                .unwrap_or(0) as f64;
            ((peak * 100.0 / 32768.0).round() as u32).min(100) as u8
        })
        .collect()
}

pub fn probe_json(path: &Path) -> Result<serde_json::Value> {
    let output = Command::new("ffprobe")
        .args([
            "-v",
            "error",
            "-select_streams",
            "v:0",
            "-show_entries",
            "stream=width,height,duration:format=duration",
            "-of",
            "json",
            &path.display().to_string(),
        ])
        .output()
        .map_err(|_| Error::dependency("ffprobe not found on PATH: install ffmpeg"))?;
    if !output.status.success() {
        return Err(Error::input(format!(
            "ffprobe could not read {}",
            path.display()
        )));
    }
    serde_json::from_slice(&output.stdout)
        .map_err(|error| Error::input(format!("ffprobe output unreadable: {error}")))
}

fn probe_dimensions(path: &Path) -> Result<(Option<i64>, Option<i64>)> {
    let probe = probe_json(path)?;
    let stream = probe.get("streams").and_then(|streams| streams.get(0));
    Ok((
        stream
            .and_then(|stream| stream.get("width"))
            .and_then(|value| value.as_i64()),
        stream
            .and_then(|stream| stream.get("height"))
            .and_then(|value| value.as_i64()),
    ))
}

fn run_ffmpeg(args: &[String]) -> Result<()> {
    let mut command = Command::new("ffmpeg");
    command.arg("-nostdin").arg("-v").arg("error");
    command.args(args);
    let output = command
        .output()
        .map_err(|_| Error::dependency("ffmpeg not found on PATH"))?;
    if !output.status.success() {
        let mut stderr = std::io::stderr();
        let _ = stderr.flush();
        return Err(Error::input(format!(
            "ffmpeg failed: {}",
            String::from_utf8_lossy(&output.stderr)
                .lines()
                .next()
                .unwrap_or("no message")
        )));
    }
    Ok(())
}
