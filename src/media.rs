//! Finding the file a row points at, and the derivatives the viewer needs.

use std::io::{Cursor, Read};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use image::ImageDecoder;
use openh264::formats::YUVSource;
use sha2::{Digest, Sha256};

use crate::container;
use crate::error::{Error, Result};
use crate::msgstore::Media;

pub const PEAKS: usize = 500;
/// The widest a generated image thumbnail or a decoded poster gets, in pixels.
pub const THUMB_WIDTH: u32 = 320;
pub const POSTER_WIDTH: u32 = 480;

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
    contain(candidates, &roots)
}

/// The first candidate that exists and stays inside one of the roots.
pub fn contain(candidates: Vec<PathBuf>, roots: &[PathBuf]) -> Option<PathBuf> {
    let roots: Vec<PathBuf> = roots
        .iter()
        .filter_map(|root| root.canonicalize().ok())
        .collect();
    if roots.is_empty() {
        return None;
    }
    for candidate in candidates {
        let Ok(resolved) = candidate.canonicalize() else {
            continue;
        };
        if !roots.iter().any(|root| resolved.starts_with(root)) {
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

pub fn sha256_bytes(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
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

/// The derivatives of one media item: a thumbnail or a poster, a waveform, and the numbers the
/// record carries. Called with a missing file too, because the phone may have kept the preview.
pub fn derive(media: &Media, archive_root: &Path, source_sha: &str) -> Result<Derived> {
    match media.kind.as_str() {
        "image" => image_derivative(media, archive_root, source_sha),
        "video" | "gif" => video_derivative(media, archive_root, source_sha),
        "audio" => audio_derivative(media),
        // Stickers, contacts, documents and locations are shown as they are.
        _ => Ok(Derived::default()),
    }
}

fn image_derivative(media: &Media, archive_root: &Path, sha: &str) -> Result<Derived> {
    if let Some(source) = &media.source {
        let (bytes, width, height) = thumbnail_of(source)?;
        let rel = write_derived(&bytes, archive_root, &format!("{sha}.jpg"))?;
        return Ok(Derived {
            width,
            height,
            thumb: Some(rel),
            ..Derived::default()
        });
    }
    Ok(Derived {
        thumb: stored_poster(media, archive_root)?,
        ..Derived::default()
    })
}

fn video_derivative(media: &Media, archive_root: &Path, sha: &str) -> Result<Derived> {
    let mut derived = Derived::default();
    if let Some(source) = &media.source {
        match container::probe(source) {
            Ok(info) => {
                derived.width = info.width;
                derived.height = info.height;
                derived.duration_ms = info.duration_ms;
                if media.thumbnail.is_none() {
                    if let Some(frame) = info.h264.as_deref().and_then(h264_poster) {
                        let rel = write_derived(&frame, archive_root, &format!("{sha}.jpg"))?;
                        derived.thumb = Some(rel);
                    }
                }
            }
            // The file is copied either way: when the phone kept a preview, the picture is enough
            // and only the duration is unknown.
            Err(error) if media.thumbnail.is_none() => return Err(error),
            Err(_) => {}
        }
    }
    if derived.thumb.is_none() {
        derived.thumb = stored_poster(media, archive_root)?;
    }
    Ok(derived)
}

/// The preview the phone kept, written as it is, named by the hash of its own bytes. A preview in a
/// format nobody can show is left out rather than written under an extension that lies.
fn stored_poster(media: &Media, archive_root: &Path) -> Result<Option<String>> {
    let Some(bytes) = media.thumbnail.as_ref().filter(|bytes| !bytes.is_empty()) else {
        return Ok(None);
    };
    let Some(extension) = extension_of(bytes) else {
        return Ok(None);
    };
    let digest = sha256_bytes(bytes);
    Ok(Some(write_derived(
        bytes,
        archive_root,
        &format!("{digest}.{extension}"),
    )?))
}

fn audio_derivative(media: &Media) -> Result<Derived> {
    let Some(source) = &media.source else {
        return Ok(Derived::default());
    };
    let ogg = container::ogg(source)?;
    let peaks = decode_peaks(&ogg)?;
    Ok(Derived {
        duration_ms: ogg.duration_ms,
        peaks: Some(peaks),
        ..Derived::default()
    })
}

/// Write a derivative beside the archive's others, atomically, and return its relative path.
fn write_derived(bytes: &[u8], archive_root: &Path, name: &str) -> Result<String> {
    let rel = format!("deriv/{name}");
    let destination = archive_root.join(&rel);
    if destination.is_file() {
        return Ok(rel);
    }
    std::fs::create_dir_all(destination.parent().unwrap_or(archive_root))?;
    let partial = destination.with_file_name(scratch_name(name, "part"));
    std::fs::write(&partial, bytes)?;
    std::fs::rename(&partial, &destination)?;
    Ok(rel)
}

/// Decode an image, honour the camera's orientation, and scale it down to a thumbnail.
fn thumbnail_of(source: &Path) -> Result<(Vec<u8>, Option<i64>, Option<i64>)> {
    let mut reader = image::ImageReader::open(source)
        .and_then(|reader| reader.with_guessed_format())
        .map_err(|error| Error::input(format!("cannot read the image: {error}")))?;
    // A picture is untrusted input: a crafted header must not be able to ask for gigabytes.
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(16_384);
    limits.max_image_height = Some(16_384);
    limits.max_alloc = Some(256 << 20);
    reader.limits(limits);
    let mut decoder = reader
        .into_decoder()
        .map_err(|error| Error::input(format!("cannot decode the image: {error}")))?;
    let orientation = decoder
        .orientation()
        .unwrap_or(image::metadata::Orientation::NoTransforms);
    let mut picture = image::DynamicImage::from_decoder(decoder)
        .map_err(|error| Error::input(format!("cannot decode the image: {error}")))?;
    picture.apply_orientation(orientation);
    let (width, height) = (picture.width(), picture.height());
    if width > THUMB_WIDTH {
        let scaled = (height as u64 * THUMB_WIDTH as u64 / width.max(1) as u64) as u32;
        picture = picture.resize_exact(
            THUMB_WIDTH,
            scaled.max(1),
            image::imageops::FilterType::Triangle,
        );
    }
    let mut bytes = Vec::new();
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut Cursor::new(&mut bytes), 80)
        .encode_image(&picture)
        .map_err(|error| Error::input(format!("cannot encode the thumbnail: {error}")))?;
    Ok((bytes, Some(width as i64), Some(height as i64)))
}

/// The first frame of an H.264 access unit, scaled and encoded as a JPEG poster.
fn h264_poster(bitstream: &[u8]) -> Option<Vec<u8>> {
    let mut decoder = openh264::decoder::Decoder::new().ok()?;
    let yuv = decoder.decode(bitstream).ok().flatten()?;
    let (width, height) = yuv.dimensions();
    if width == 0 || height == 0 {
        return None;
    }
    let rgb = yuv_to_rgb(width, height, yuv.strides(), (yuv.y(), yuv.u(), yuv.v()));
    let frame = image::RgbImage::from_raw(width as u32, height as u32, rgb)?;
    let frame = if width as u32 > POSTER_WIDTH {
        let scaled = (height as u64 * POSTER_WIDTH as u64 / width as u64) as u32;
        image::imageops::resize(
            &frame,
            POSTER_WIDTH,
            scaled.max(1),
            image::imageops::FilterType::Triangle,
        )
    } else {
        frame
    };
    let mut bytes = Vec::new();
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut Cursor::new(&mut bytes), 80)
        .encode_image(&frame)
        .ok()?;
    Some(bytes)
}

/// BT.601 limited-range YUV to RGB, which is what H.264 streams carry.
fn yuv_to_rgb(
    width: usize,
    height: usize,
    (stride_y, stride_u, stride_v): (usize, usize, usize),
    (luma, blue, red): (&[u8], &[u8], &[u8]),
) -> Vec<u8> {
    let mut rgb = vec![0u8; width * height * 3];
    for row in 0..height {
        for column in 0..width {
            let y = luma[row * stride_y + column] as i32;
            let u = blue[(row / 2) * stride_u + column / 2] as i32 - 128;
            let v = red[(row / 2) * stride_v + column / 2] as i32 - 128;
            let c = (y - 16).max(0);
            let index = (row * width + column) * 3;
            rgb[index] = clamp((298 * c + 409 * v + 128) >> 8);
            rgb[index + 1] = clamp((298 * c - 100 * u - 208 * v + 128) >> 8);
            rgb[index + 2] = clamp((298 * c + 516 * u + 128) >> 8);
        }
    }
    rgb
}

fn clamp(value: i32) -> u8 {
    value.clamp(0, 255) as u8
}

/// The extension a picture's own bytes declare, or None for bytes no viewer could show.
pub fn extension_of(bytes: &[u8]) -> Option<&'static str> {
    if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
        Some("jpg")
    } else if bytes.starts_with(&[0x89, b'P', b'N', b'G']) {
        Some("png")
    } else if bytes.starts_with(b"RIFF") && bytes.get(8..12) == Some(b"WEBP") {
        Some("webp")
    } else {
        None
    }
}

/// 500 peaks, one per equal window of the whole note, the loudest sample of each.
fn decode_peaks(ogg: &container::Ogg) -> Result<Vec<u8>> {
    use opus::Channels;
    let channels = if ogg.channels > 1 {
        Channels::Stereo
    } else {
        Channels::Mono
    };
    let mut decoder = opus::Decoder::new(48_000, channels)
        .map_err(|error| Error::input(format!("the Opus decoder refused the note: {error}")))?;
    // One value per 20 ms of audio, then reduced to the 500 the viewer draws: the decoded length is
    // whatever the stream says, so no total has to be predicted first.
    const WINDOW: usize = 960;
    let mut fine: Vec<u8> = Vec::new();
    let mut samples = vec![0i16; 57_600 * ogg.channels.max(1) as usize];
    let mut index = 0usize;
    let mut decoded = 0usize;
    let channels = ogg.channels.max(1) as usize;
    for packet in &ogg.packets {
        let Ok(count) = decoder.decode(&packet.bytes, &mut samples, false) else {
            continue;
        };
        decoded += 1;
        for position in 0..count {
            let peak = peak_of(&samples, position, channels);
            let bucket = index / WINDOW;
            if fine.len() <= bucket {
                fine.resize(bucket + 1, 0);
            }
            let value = ((peak.unsigned_abs() as f64) * 100.0 / 32768.0).round() as u8;
            fine[bucket] = fine[bucket].max(value.min(100));
            index += 1;
        }
    }
    if decoded == 0 {
        return Err(Error::input(
            "the Opus decoder could not read a single packet",
        ));
    }
    Ok(peaks_of(&fine))
}

/// The loudest of the interleaved channels at one frame position.
fn peak_of(samples: &[i16], position: usize, channels: usize) -> i16 {
    let mut peak = 0i16;
    for channel in 0..channels {
        peak = peak.max(samples[position * channels + channel]);
    }
    peak
}

/// Squeeze the fine windows into the 500 the record carries.
fn peaks_of(fine: &[u8]) -> Vec<u8> {
    if fine.is_empty() {
        return vec![0; PEAKS];
    }
    (0..PEAKS)
        .map(|bucket| {
            let start = bucket * fine.len() / PEAKS;
            let end = ((bucket + 1) * fine.len() / PEAKS)
                .max(start + 1)
                .min(fine.len());
            fine[start..end].iter().copied().max().unwrap_or(0)
        })
        .collect()
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

    fn media(name: &str) -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/synthetic/phone/Media")
            .join(name)
    }

    fn item(kind: &str, source: Option<PathBuf>) -> Media {
        Media {
            rel_path: "Media/x".into(),
            kind: kind.into(),
            mime: None,
            bytes: None,
            width: None,
            height: None,
            duration_ms: None,
            source,
            thumbnail: None,
        }
    }

    #[test]
    fn a_path_that_climbs_out_of_the_copy_is_refused() {
        let root = scratch("escape");
        let media = root.join("Media");
        std::fs::create_dir_all(&media).unwrap();
        std::fs::write(media.join("photo.jpg"), b"bytes").unwrap();
        let secret = std::env::temp_dir().join(format!(
            "wexport-outside-{}-{}.txt",
            std::process::id(),
            "escape"
        ));
        std::fs::write(&secret, b"secret").unwrap();

        assert_eq!(
            source_for("photo.jpg", Some(&media), &root),
            Some(media.join("photo.jpg").canonicalize().unwrap())
        );
        let climb = format!("../../{}", secret.file_name().unwrap().to_string_lossy());
        assert_eq!(
            source_for(&climb, Some(&media), &root),
            None,
            "a row cannot pull a file from outside the copy"
        );
        let _ = std::fs::remove_file(&secret);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn an_image_that_claims_a_huge_size_is_refused_not_allocated() {
        let root = scratch("bomb");
        let source = root.join("bomb.png");
        // A PNG header that declares 60 000 x 60 000 and then stops: the limit answers before any
        // decoder tries to reserve the pixels.
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A]);
        bytes.extend_from_slice(&[0, 0, 0, 13]);
        bytes.extend_from_slice(b"IHDR");
        bytes.extend_from_slice(&60_000u32.to_be_bytes());
        bytes.extend_from_slice(&60_000u32.to_be_bytes());
        bytes.extend_from_slice(&[8, 6, 0, 0, 0]);
        std::fs::write(&source, bytes).unwrap();
        let error = derive(&item("image", Some(source)), &root, "sha").unwrap_err();
        assert!(
            error.message.contains("cannot decode the image"),
            "{}",
            error.message
        );
        let _ = std::fs::remove_dir_all(root);
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

    #[test]
    fn an_image_becomes_a_jpeg_thumbnail_no_wider_than_three_hundred_and_twenty() {
        let root = scratch("image");
        let derived = derive(
            &item(
                "image",
                Some(media("WhatsApp Images/IMG-20261201-WA0001.png")),
            ),
            &root,
            "sha",
        )
        .unwrap();
        let thumb = root.join(derived.thumb.expect("an image has a thumbnail"));
        let decoded = image::open(&thumb).unwrap();
        assert_eq!(decoded.width(), 16, "a small image is not upscaled");
        assert_eq!(thumb.extension().and_then(|e| e.to_str()), Some("jpg"));
        assert_eq!((derived.width, derived.height), (Some(16), Some(16)));
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn a_large_image_is_scaled_down_to_three_hundred_and_twenty() {
        let root = scratch("scale");
        let source = root.join("big.png");
        let picture = image::RgbImage::from_fn(800, 400, |x, y| {
            image::Rgb([(x % 256) as u8, (y % 256) as u8, 90])
        });
        picture.save(&source).unwrap();
        let derived = derive(&item("image", Some(source.clone())), &root, "big").unwrap();
        let thumb = image::open(root.join(derived.thumb.unwrap())).unwrap();
        assert_eq!(thumb.width(), 320);
        assert_eq!(thumb.height(), 160, "the shape is kept");
        assert_eq!(derived.width, Some(800), "the record keeps the original");
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn a_video_gives_a_poster_without_a_stored_thumbnail() {
        let root = scratch("video");
        let derived = derive(
            &item(
                "video",
                Some(media("WhatsApp Video/VID-20261201-WA0003.mp4")),
            ),
            &root,
            "sha",
        )
        .unwrap();
        assert_eq!(derived.duration_ms, Some(1000));
        assert_eq!((derived.width, derived.height), (Some(16), Some(16)));
        let poster = root.join(derived.thumb.expect("a decoded poster"));
        let decoded = image::open(&poster).unwrap();
        assert_eq!((decoded.width(), decoded.height()), (16, 16));
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn a_stored_thumbnail_is_the_poster_and_is_named_by_its_own_bytes() {
        let root = scratch("stored");
        let mut video = item("video", None);
        video.thumbnail = Some(vec![0xFF, 0xD8, 0xFF, 1, 2, 3]);
        let derived = derive(&video, &root, "ignored").unwrap();
        let rel = derived.thumb.expect("the stored preview is the poster");
        assert!(rel.starts_with("deriv/"));
        assert!(rel.ends_with(".jpg"));
        assert!(rel.contains(&sha256_bytes(&[0xFF, 0xD8, 0xFF, 1, 2, 3])[..8]));
        assert!(root.join(rel).is_file());
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn a_preview_no_browser_could_show_is_left_out_rather_than_mislabelled() {
        let root = scratch("unknown-preview");
        let mut video = item("video", None);
        video.thumbnail = Some(b"not a picture".to_vec());
        let derived = derive(&video, &root, "ignored").unwrap();
        assert!(derived.thumb.is_none(), "no file, no lying extension");
        assert!(!root.join("deriv").exists());
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn a_video_with_a_corrupt_container_but_a_stored_preview_still_shows_it() {
        let root = scratch("corrupt-with-preview");
        let broken = root.join("broken.mp4");
        std::fs::write(&broken, b"not a container").unwrap();
        let mut video = item("video", Some(broken));
        video.thumbnail = Some(vec![0xFF, 0xD8, 0xFF, 9, 9, 9]);
        let derived = derive(&video, &root, "sha").unwrap();
        assert!(derived.thumb.is_some(), "the preview is the poster");
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn a_voice_note_gives_a_waveform_and_a_duration() {
        let root = scratch("audio");
        let derived = derive(
            &item(
                "audio",
                Some(media("WhatsApp Voice Notes/PTT-20261201-WA0002.opus")),
            ),
            &root,
            "sha",
        )
        .unwrap();
        assert_eq!(derived.peaks.as_ref().map(|peaks| peaks.len()), Some(PEAKS));
        assert!((200..=500).contains(&derived.duration_ms.unwrap_or(0)));
        assert!(derived.thumb.is_none());
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn a_missing_file_gets_nothing_rather_than_an_error() {
        let root = scratch("missing");
        let derived = derive(&item("image", None), &root, "sha").unwrap();
        assert!(derived.thumb.is_none());
        assert!(!root.join("deriv").exists());
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn fine_windows_squeeze_into_five_hundred_without_gaps() {
        let flat = vec![10u8; 15];
        let peaks = peaks_of(&flat);
        assert_eq!(peaks.len(), PEAKS);
        assert!(peaks.iter().all(|peak| *peak == 10), "a short note repeats");

        let varied: Vec<u8> = (0..1500).map(|index| (index % 50) as u8 + 1).collect();
        let peaks = peaks_of(&varied);
        assert_eq!(peaks.len(), PEAKS);
        assert!(peaks.iter().all(|peak| *peak > 0), "no window is skipped");
        // Three windows per bucket, and the loud one is never at the bucket's first sample: a
        // boundary that slips shows a quiet window instead.
        let alternating: Vec<u8> = (0..1500)
            .map(|index| if index % 2 == 0 { 1 } else { 3 })
            .collect();
        assert!(peaks_of(&alternating).iter().all(|peak| *peak == 3));
        assert_eq!(peaks_of(&[]), vec![0; PEAKS]);
    }

    #[test]
    fn a_sticker_needs_nothing_derived() {
        let derived = derive(
            &item("sticker", Some(media("WhatsApp Stickers/sticker.webp"))),
            Path::new("/nonexistent"),
            "sha",
        )
        .unwrap();
        assert!(derived.thumb.is_none());
        assert!(derived.peaks.is_none());
    }

    #[test]
    fn a_wide_video_poster_is_scaled_to_four_hundred_and_eighty() {
        let root = scratch("wide");
        let derived = derive(
            &item(
                "video",
                Some(media("WhatsApp Video/VID-20261201-WA0005.mp4")),
            ),
            &root,
            "sha",
        )
        .unwrap();
        assert_eq!((derived.width, derived.height), (Some(960), Some(540)));
        let poster = image::open(root.join(derived.thumb.expect("a decoded poster"))).unwrap();
        assert_eq!(
            (poster.width(), poster.height()),
            (480, 270),
            "the shape is kept"
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn a_frame_peak_is_the_loudest_of_its_interleaved_channels() {
        // Frame 1 of two channels is samples 2 and 3; every other way of reading an index lands on
        // a louder pair.
        let samples = [100i16, 100, 5, 5, 50, 50];
        assert_eq!(peak_of(&samples, 1, 2), 5);
    }

    #[test]
    fn a_pixel_comes_out_of_the_bt601_matrix() {
        // A 3x6 frame: row 5, column 2 reads luma[5 * 4 + 2] and the chroma at row 2, column 1.
        let mut luma = vec![100u8; 30];
        luma[22] = 128;
        luma[11] = 30;
        luma[3] = 200;
        let mut blue = vec![100u8; 16];
        blue[7] = 64;
        blue[6] = 10;
        blue[1] = 20;
        blue[10] = 200;
        let mut red = vec![100u8; 16];
        red[7] = 192;
        red[6] = 40;
        red[1] = 60;
        red[10] = 50;
        let rgb = yuv_to_rgb(3, 6, (4, 3, 3), (&luma, &blue, &red));
        let index = (5 * 3 + 2) * 3;
        assert_eq!(
            &rgb[index..index + 3],
            &[233, 103, 1],
            "y=128, u=64, v=192 through BT.601 limited range"
        );
    }

    #[test]
    fn a_stored_preview_is_the_thumbnail_of_an_image_that_was_not_copied() {
        let root = scratch("stored-image");
        let mut image = item("image", None);
        image.thumbnail = Some(vec![0x89, b'P', b'N', b'G', 1, 2, 3]);
        let derived = derive(&image, &root, "ignored").unwrap();
        let rel = derived.thumb.expect("the preview the phone kept");
        assert!(rel.ends_with(".png"), "{rel}");
        assert!(root.join(&rel).is_file());
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn a_video_that_cannot_be_read_and_has_no_preview_is_an_error() {
        let root = scratch("corrupt-no-preview");
        let broken = root.join("broken.mp4");
        std::fs::write(&broken, b"not a container").unwrap();
        let error = derive(&item("video", Some(broken)), &root, "sha").unwrap_err();
        assert!(!error.message.is_empty(), "the reason is reported");
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn a_webp_is_a_webp_and_a_riff_that_is_not_one_is_nothing() {
        assert_eq!(extension_of(b"RIFF\x00\x00\x00\x00WEBPVP8 "), Some("webp"));
        assert_eq!(extension_of(b"RIFF\x00\x00\x00\x00WAVEfmt "), None);
        assert_eq!(extension_of(&[0xFF, 0xD8, 0xFF, 0xE0]), Some("jpg"));
        assert_eq!(extension_of(&[0x89, b'P', b'N', b'G', 0x0D]), Some("png"));
    }

    #[test]
    fn placing_without_a_link_copies_the_file() {
        let root = scratch("copy");
        let source = root.join("photo.jpg");
        std::fs::write(&source, b"same bytes").unwrap();
        let archive = root.join("archive");
        std::fs::create_dir_all(&archive).unwrap();
        let placed = place(&source, &archive, false).unwrap();
        let copied = archive.join(&placed.rel_path);
        std::fs::write(&copied, b"changed").unwrap();
        assert_eq!(
            std::fs::read(&source).unwrap(),
            b"same bytes",
            "a copy can be edited without touching the phone's file"
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn a_stereo_note_is_decoded_as_two_channels() {
        let mut encoder =
            opus::Encoder::new(48_000, opus::Channels::Stereo, opus::Application::Audio).unwrap();
        let mut packets = Vec::new();
        for _ in 0..10 {
            // A loud left channel and a silent right one: a decoder opened in mono either refuses
            // the packet or hears half the level.
            let frame: Vec<i16> = (0..960).flat_map(|_| [24_000i16, 0]).collect();
            let mut bytes = vec![0u8; 4096];
            let written = encoder.encode(&frame, &mut bytes).unwrap();
            bytes.truncate(written);
            packets.push(container::OggPacket { bytes, audio: true });
        }
        let ogg = container::Ogg {
            channels: 2,
            duration_ms: None,
            packets,
        };
        let peaks = decode_peaks(&ogg).unwrap();
        assert_eq!(peaks.len(), PEAKS);
        assert!(
            peaks.iter().any(|peak| *peak > 50),
            "the loud channel reaches the waveform: {:?}",
            peaks.iter().max()
        );
    }

    proptest! {
        #[test]
        fn a_picture_extension_is_known_or_absent(bytes in proptest::collection::vec(any::<u8>(), 0..20)) {
            if let Some(extension) = extension_of(&bytes) {
                prop_assert!(["jpg", "png", "webp"].contains(&extension));
            }
        }
    }
}
