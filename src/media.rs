//! Finding the file a row points at, and the derivatives the viewer needs.

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

use sha2::{Digest, Sha256};

use crate::error::{Error, Result};

pub const PEAKS: usize = 500;

/// Distinguishes the scratch files of two workers that place the same content at the same moment.
static SCRATCH: AtomicU64 = AtomicU64::new(0);

fn scratch_name(prefix: &str, suffix: &str) -> String {
    format!(
        "{prefix}.{}.{}.{suffix}",
        std::process::id(),
        SCRATCH.fetch_add(1, Ordering::Relaxed)
    )
}

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

/// Hash a file in blocks: a 2 GB video must not have to fit in memory to be placed.
pub fn sha256_of(path: &Path) -> Result<String> {
    let mut reader = std::io::BufReader::new(std::fs::File::open(path)?);
    let mut digest = Sha256::new();
    let mut buffer = vec![0u8; 1 << 20];
    loop {
        let read = reader.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        digest.update(&buffer[..read]);
    }
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
    // Write beside the target and rename, so a concurrent worker never exposes a partial file. The
    // scratch name carries a counter: two workers placing the same content must not share it.
    let partial = destination.with_file_name(scratch_name(
        destination
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("media"),
        "part",
    ));
    std::fs::copy(source, &partial)?;
    std::fs::rename(&partial, &destination)?;
    Ok(Placed {
        sha256: digest,
        rel_path,
    })
}

pub fn derive(kind: &str, source: &Path, archive_root: &Path, sha: &str) -> Result<Derived> {
    match kind {
        "image" => image_derivative(source, archive_root, sha),
        "video" | "gif" => video_derivative(source, archive_root, sha),
        "audio" => audio_derivative(source, archive_root, sha),
        // Stickers, contacts, documents and locations are shown as they are.
        _ => Ok(Derived::default()),
    }
}

fn image_derivative(source: &Path, archive_root: &Path, sha: &str) -> Result<Derived> {
    let rel = format!("deriv/{sha}.webp");
    let target = archive_root.join(&rel);
    std::fs::create_dir_all(target.parent().unwrap_or(archive_root))?;
    run_ffmpeg(&[
        "-i".into(),
        source.display().to_string(),
        // One frame: an animated WebP would otherwise become an animated "thumbnail" of any size.
        "-frames:v".into(),
        "1".into(),
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
    let raw = archive_root.join(format!("deriv/{}", scratch_name(sha, "pcm")));
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
        ..Derived::default()
    })
}

fn peaks_of(pcm: &[u8]) -> Vec<u8> {
    let samples: Vec<i16> = pcm
        .as_chunks::<2>()
        .0
        .iter()
        .map(|pair| i16::from_le_bytes(*pair))
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

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn scratch(name: &str) -> PathBuf {
        let path =
            std::env::temp_dir().join(format!("wexport-media-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).unwrap();
        path
    }

    #[test]
    fn the_hash_is_the_file_content() {
        let root = scratch("hash");
        let file = root.join("abc.bin");
        std::fs::write(&file, b"abc").unwrap();
        assert_eq!(
            sha256_of(&file).unwrap(),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn peaks_are_the_loudest_sample_of_each_window() {
        let loud: Vec<u8> = vec![-32768i16; PEAKS]
            .iter()
            .flat_map(|sample| sample.to_le_bytes())
            .collect();
        assert!(peaks_of(&loud).iter().all(|peak| *peak == 100));
        assert_eq!(peaks_of(&[0, 0, 0, 0]), vec![0; PEAKS]);
    }

    proptest! {
        #[test]
        fn peaks_are_always_five_hundred_values_in_range(bytes in proptest::collection::vec(any::<u8>(), 0..8000)) {
            let peaks = peaks_of(&bytes);
            prop_assert_eq!(peaks.len(), PEAKS);
            prop_assert!(peaks.iter().all(|peak| *peak <= 100));
        }
    }

    #[test]
    fn the_same_content_is_placed_once() {
        let root = scratch("place");
        let source = root.join("photo.jpg");
        std::fs::write(&source, b"same bytes").unwrap();
        let archive = root.join("archive");
        std::fs::create_dir_all(&archive).unwrap();
        let first = place(&source, &archive, false).unwrap();
        let second = place(&source, &archive, false).unwrap();
        assert_eq!(first.rel_path, second.rel_path);
        assert!(archive.join(&first.rel_path).is_file());
        assert_eq!(
            std::fs::read_dir(archive.join(format!("media/{}", &first.sha256[..2])))
                .unwrap()
                .count(),
            1,
            "one file per content"
        );
        let _ = std::fs::remove_dir_all(root);
    }
}

#[cfg(test)]
mod fixture_tests {
    use super::*;
    use std::os::unix::fs::MetadataExt;

    fn media(name: &str) -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/synthetic/phone/Media")
            .join(name)
    }

    fn scratch(name: &str) -> PathBuf {
        let path =
            std::env::temp_dir().join(format!("wexport-derive-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).unwrap();
        path
    }

    #[test]
    fn a_sticker_needs_nothing_derived() {
        let derived = derive(
            "sticker",
            &media("WhatsApp Stickers/sticker.webp"),
            Path::new("/nonexistent"),
            "sha",
        )
        .unwrap();
        assert!(derived.thumb.is_none());
        assert!(derived.peaks.is_none());
    }

    #[test]
    fn a_voice_note_gives_peaks_and_no_poster() {
        let root = scratch("audio");
        let derived = derive(
            "audio",
            &media("WhatsApp Voice Notes/PTT-20261201-WA0002.opus"),
            &root,
            "sha",
        )
        .unwrap();
        assert_eq!(derived.peaks.as_ref().map(|peaks| peaks.len()), Some(PEAKS));
        assert!(derived.thumb.is_none());
        assert!(
            std::fs::read_dir(root.join("deriv")).unwrap().count() == 0,
            "no leftovers"
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn a_video_gives_a_poster_and_its_duration() {
        let root = scratch("video");
        let derived = derive(
            "video",
            &media("WhatsApp Video/VID-20261201-WA0003.mp4"),
            &root,
            "sha",
        )
        .unwrap();
        assert!(derived.thumb.is_some());
        // The fixture is exactly one second long, in milliseconds.
        assert_eq!(derived.duration_ms, Some(1000));
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn an_animated_image_gives_a_readable_single_frame_thumbnail() {
        let root = scratch("animated");
        let source = root.join("anim.gif");
        let status = Command::new("ffmpeg")
            .args([
                "-nostdin",
                "-v",
                "error",
                "-f",
                "lavfi",
                "-i",
                "testsrc=size=64x64:rate=2",
                "-frames:v",
                "4",
                "-y",
            ])
            .arg(&source)
            .status()
            .unwrap();
        assert!(status.success());

        let derived = derive("image", &source, &root, "animated").unwrap();
        let thumb = root.join(derived.thumb.expect("an image has a thumbnail"));
        let probe = Command::new("ffprobe")
            .args([
                "-v",
                "error",
                "-count_frames",
                "-select_streams",
                "v:0",
                "-show_entries",
                "stream=nb_read_frames",
                "-of",
                "default=nw=1:nk=1",
            ])
            .arg(&thumb)
            .output()
            .unwrap();
        assert_eq!(
            String::from_utf8_lossy(&probe.stdout).trim(),
            "1",
            "a multi-frame source used to write a thumbnail no browser could read"
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn place_copies_without_link_and_links_when_asked() {
        let root = scratch("place");
        let source = root.join("photo.png");
        std::fs::write(&source, b"some bytes").unwrap();
        let archive = root.join("archive");
        std::fs::create_dir_all(&archive).unwrap();

        let copied = place(&source, &archive, false).unwrap();
        let copied_meta = std::fs::metadata(archive.join(&copied.rel_path)).unwrap();
        assert_ne!(copied_meta.ino(), std::fs::metadata(&source).unwrap().ino());

        let linked = place(&source, &archive.join("other"), true).unwrap();
        assert_eq!(
            std::fs::metadata(archive.join("other").join(&linked.rel_path))
                .unwrap()
                .ino(),
            std::fs::metadata(&source).unwrap().ino()
        );
        let _ = std::fs::remove_dir_all(root);
    }
}
