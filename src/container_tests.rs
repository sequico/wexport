//! The parsers, byte by byte.
//!
//! The fixture proves that a real WhatsApp file reads; these tests prove what the readers do at the
//! edges — a box that ends where it says, a truncated element, a page whose packet spans two pages —
//! because mutation testing found all of them uncovered.

use super::*;

fn scratch(name: &str) -> std::path::PathBuf {
    let path = std::env::temp_dir().join(format!(
        "wexport-container-{name}-{}.bin",
        std::process::id()
    ));
    let _ = std::fs::remove_file(&path);
    path
}

fn file_with(name: &str, bytes: &[u8]) -> (File, u64) {
    let path = scratch(name);
    std::fs::write(&path, bytes).unwrap();
    let file = File::open(&path).unwrap();
    let length = bytes.len() as u64;
    let _ = std::fs::remove_file(&path);
    (file, length)
}

fn be_box(kind: &[u8; 4], payload: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(payload.len() + 8);
    out.extend_from_slice(&((payload.len() + 8) as u32).to_be_bytes());
    out.extend_from_slice(kind);
    out.extend_from_slice(payload);
    out
}

fn raw_box(kind: &[u8; 4], size: u32, payload: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(payload.len() + 8);
    out.extend_from_slice(&size.to_be_bytes());
    out.extend_from_slice(kind);
    out.extend_from_slice(payload);
    out
}

fn large_box(kind: &[u8; 4], payload: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(&1u32.to_be_bytes());
    out.extend_from_slice(kind);
    out.extend_from_slice(&((payload.len() as u64) + 16).to_be_bytes());
    out.extend_from_slice(payload);
    out
}

fn concat(parts: &[Vec<u8>]) -> Vec<u8> {
    parts.iter().flatten().copied().collect()
}

// ------------------------------------------------------------------ MP4 boxes

#[test]
fn find_box_walks_past_the_boxes_it_does_not_want() {
    let data = concat(&[
        be_box(b"ftyp", b"isom"),
        be_box(b"free", b"junk"),
        be_box(b"moov", b"payload"),
    ]);
    let (mut file, length) = file_with("find", &data);
    assert_eq!(
        find_box(&mut file, length, b"moov").unwrap(),
        Some((7, 32)),
        "the payload of the third box"
    );
    assert_eq!(find_box(&mut file, length, b"mdat").unwrap(), None);
}

#[test]
fn find_box_reads_a_64_bit_size_and_a_size_that_runs_to_the_end() {
    let wide = concat(&[be_box(b"skip", b"x"), large_box(b"moov", b"abc")]);
    let (mut file, length) = file_with("wide", &wide);
    assert_eq!(find_box(&mut file, length, b"moov").unwrap(), Some((3, 25)));

    let open = raw_box(b"moov", 0, b"to the end");
    let (mut file, length) = file_with("open", &open);
    assert_eq!(
        find_box(&mut file, length, b"moov").unwrap(),
        Some((10, 8)),
        "a zero size means everything that is left"
    );
}

#[test]
fn find_box_reads_an_open_size_after_another_box() {
    let data = concat(&[be_box(b"free", b"x"), raw_box(b"moov", 0, b"end")]);
    let (mut file, length) = file_with("open-after", &data);
    assert_eq!(
        find_box(&mut file, length, b"moov").unwrap(),
        Some((3, 17)),
        "what is left after the second header, not after the first"
    );
}

#[test]
fn find_box_finds_a_box_that_ends_exactly_at_the_file_end() {
    let data = concat(&[be_box(b"free", b"x"), be_box(b"moov", b"")]);
    let (mut file, length) = file_with("tail", &data);
    assert_eq!(
        find_box(&mut file, length, b"moov").unwrap(),
        Some((0, 17)),
        "the last eight bytes are a box too"
    );
}

#[test]
fn find_box_stops_at_a_file_shorter_than_its_boxes() {
    let (mut file, _) = file_with("short", b"abcd");
    assert_eq!(find_box(&mut file, 100, b"moov").unwrap(), None, "read < 8");
    let (mut file, length) = file_with("tiny", b"abcd");
    assert_eq!(find_box(&mut file, length, b"moov").unwrap(), None);
    let (mut file, length) = file_with("size-zero-box", &raw_box(b"free", 4, b""));
    assert_eq!(
        find_box(&mut file, length, b"free").unwrap(),
        None,
        "a size smaller than its header is not a box"
    );
    let mut wide = vec![0u8; 8];
    wide[3] = 1;
    wide[4..8].copy_from_slice(b"moov");
    let (mut file, length) = file_with("wide-short", &wide);
    assert_eq!(
        find_box(&mut file, length, b"moov").unwrap(),
        None,
        "a 64-bit size with no room for it"
    );
}

#[test]
fn boxes_handle_a_normal_forty_bit_and_open_size() {
    let data = concat(&[
        be_box(b"aaaa", b"one"),
        large_box(b"bbbb", b"two"),
        raw_box(b"cccc", 0, b"rest"),
    ]);
    let kinds: Vec<[u8; 4]> = boxes(&data).iter().map(|(kind, _)| *kind).collect();
    assert_eq!(kinds, [*b"aaaa", *b"bbbb", *b"cccc"]);

    let empty_bytes = concat(&[be_box(b"aaaa", b"")]);
    let empty = boxes(&empty_bytes);
    assert_eq!(empty.len(), 1, "a box of exactly its header is still a box");
    assert!(
        boxes(&[0u8; 7]).is_empty(),
        "fewer than eight bytes is no box"
    );
    assert!(
        boxes(&raw_box(b"aaaa", 4, b"")).is_empty(),
        "a size smaller than the header is not a box"
    );
    assert!(
        boxes(&raw_box(b"aaaa", 20, b"short")).is_empty(),
        "a box that claims more than it has is not read"
    );

    let mut wide_header = vec![0u8; 8];
    wide_header[3] = 1;
    wide_header[4..8].copy_from_slice(b"wide");
    assert!(
        boxes(&wide_header).is_empty(),
        "a 64-bit size with no room for it"
    );

    let wide_empty = large_box(b"moov", b"");
    let parsed = boxes(&wide_empty);
    assert_eq!(parsed.len(), 1, "sixteen bytes are a whole 64-bit header");
    assert!(parsed[0].1.is_empty());
}

#[test]
fn a_track_is_video_only_when_its_handler_says_so() {
    let hdlr = |handler: &[u8; 4]| {
        let mut payload = vec![0u8; 8];
        payload.extend_from_slice(handler);
        be_box(b"mdia", &be_box(b"hdlr", &payload))
    };
    assert!(is_video_track(&hdlr(b"vide")));
    assert!(!is_video_track(&hdlr(b"soun")));
    assert!(!is_video_track(&be_box(
        b"mdia",
        &be_box(b"hdlr", b"short")
    )));
    assert!(!is_video_track(b""));
}

#[test]
fn the_movie_duration_is_read_in_both_versions() {
    let mut v0 = vec![0u8; 20];
    v0[12..16].copy_from_slice(&1000u32.to_be_bytes());
    v0[16..20].copy_from_slice(&1500u32.to_be_bytes());
    assert_eq!(movie_duration(&v0), Some(1500));

    let mut v1 = vec![0u8; 32];
    v1[0] = 1;
    v1[20..24].copy_from_slice(&1000u32.to_be_bytes());
    v1[24..32].copy_from_slice(&2500u64.to_be_bytes());
    assert_eq!(movie_duration(&v1), Some(2500));

    let mut no_scale = v0.clone();
    no_scale[12..16].copy_from_slice(&0u32.to_be_bytes());
    assert_eq!(movie_duration(&no_scale), None);
    let mut no_duration = v0.clone();
    no_duration[16..20].copy_from_slice(&0u32.to_be_bytes());
    assert_eq!(movie_duration(&no_duration), None);
    assert_eq!(movie_duration(&[]), None);
    assert_eq!(movie_duration(&v0[..12]), None, "too short to hold a time");
}

// ------------------------------------------------------------------ the first sample

fn avcc(length_size_minus_one: u8, sps: &[u8], pps: &[u8]) -> Vec<u8> {
    let mut out = vec![
        0x01,
        0x42,
        0xC0,
        0x1E,
        0xFC | length_size_minus_one,
        0xE0 | 1,
    ];
    out.extend_from_slice(&(sps.len() as u16).to_be_bytes());
    out.extend_from_slice(sps);
    out.push(1);
    out.extend_from_slice(&(pps.len() as u16).to_be_bytes());
    out.extend_from_slice(pps);
    out
}

fn stbl_of(entry: Option<&[u8; 4]>, avcc: Option<&[u8]>, chunk: u32, sample: u32) -> Vec<u8> {
    let mut stsd = vec![0, 0, 0, 0];
    stsd.extend_from_slice(&1u32.to_be_bytes());
    if let Some(kind) = entry {
        let mut body = vec![0u8; 78];
        if let Some(avcc) = avcc {
            body.extend_from_slice(&be_box(b"avcC", avcc));
        }
        stsd.extend_from_slice(&be_box(kind, &body));
    }
    let mut stbl = be_box(b"stsd", &stsd);
    let mut stco = vec![0, 0, 0, 0];
    stco.extend_from_slice(&1u32.to_be_bytes());
    stco.extend_from_slice(&chunk.to_be_bytes());
    stbl.extend_from_slice(&be_box(b"stco", &stco));
    let mut stsz = vec![0, 0, 0, 0];
    stsz.extend_from_slice(&0u32.to_be_bytes());
    stsz.extend_from_slice(&1u32.to_be_bytes());
    stsz.extend_from_slice(&sample.to_be_bytes());
    stbl.extend_from_slice(&be_box(b"stsz", &stsz));
    stbl
}

#[test]
fn a_first_sample_becomes_annex_b_with_its_headers() {
    let sps = [0x67, 0x42, 0xC0];
    let pps = [0x68, 0xCE];
    let sample = [0, 0, 0, 3, 0x65, 0x88, 0x84];
    let stbl = stbl_of(
        Some(b"avc1"),
        Some(&avcc(3, &sps, &pps)),
        0,
        sample.len() as u32,
    );
    let (mut file, _) = file_with("sample", &sample);
    let out = first_h264_sample(&mut file, &stbl).unwrap().unwrap();
    assert_eq!(
        out,
        [0, 0, 0, 1, 0x67, 0x42, 0xC0, 0, 0, 0, 1, 0x68, 0xCE, 0, 0, 0, 1, 0x65, 0x88, 0x84]
    );

    let stbl = stbl_of(
        Some(b"avc3"),
        Some(&avcc(3, &sps, &pps)),
        0,
        sample.len() as u32,
    );
    let (mut file, _) = file_with("sample-avc3", &sample);
    assert!(first_h264_sample(&mut file, &stbl).unwrap().is_some());
}

#[test]
fn a_sample_that_is_not_h264_is_not_a_poster() {
    let sps = [0x67, 0x42];
    let pps = [0x68, 0xCE];
    let sample = [0, 0, 0, 1, 0x65];
    for (entry, avcc_bytes) in [
        (Some(b"mp4v"), Some(avcc(3, &sps, &pps))),
        (Some(b"avc1"), None),
        (None, None),
    ] {
        let stbl = stbl_of(entry, avcc_bytes.as_deref(), 0, sample.len() as u32);
        let (mut file, _) = file_with("not-h264", &sample);
        assert_eq!(
            first_h264_sample(&mut file, &stbl).unwrap(),
            None,
            "entry {entry:?} does not describe an H.264 picture"
        );
    }

    // No stsd at all.
    let (mut file, _) = file_with("no-stsd", &sample);
    assert_eq!(first_h264_sample(&mut file, b"").unwrap(), None);

    // A sample entry with no avcC inside it.
    let mut body = vec![0u8; 78];
    body.extend_from_slice(&be_box(b"pasp", b""));
    let mut stsd = vec![0, 0, 0, 0];
    stsd.extend_from_slice(&1u32.to_be_bytes());
    stsd.extend_from_slice(&be_box(b"avc1", &body));
    let (mut file, _) = file_with("no-avcc", &sample);
    assert_eq!(
        first_h264_sample(&mut file, &be_box(b"stsd", &stsd)).unwrap(),
        None
    );

    // avcC with no parameter sets at all: both counts are zero.
    let no_sets = [0x01, 0x42, 0xC0, 0x1E, 0xFF, 0xE0, 0x00];
    let empty = stbl_of(Some(b"avc1"), Some(&no_sets), 0, sample.len() as u32);
    let (mut file, _) = file_with("no-sets", &sample);
    assert_eq!(first_h264_sample(&mut file, &empty).unwrap(), None);

    // A sample of zero bytes says nothing.
    let zero = stbl_of(Some(b"avc1"), Some(&avcc(3, &[0x67], &[0x68])), 0, 0);
    let (mut file, _) = file_with("zero-sample", &sample);
    assert_eq!(first_h264_sample(&mut file, &zero).unwrap(), None);
}

#[test]
fn a_malformed_avcc_yields_what_it_holds_and_no_more() {
    assert_eq!(parameter_sets(&[]).0.len(), 0);
    assert_eq!(
        parameter_sets(&[]).1,
        4,
        "a missing byte means four-byte lengths"
    );
    // One SPS whose length runs past the record.
    let avcc = [0x01, 0x42, 0xC0, 0x1E, 0xFF, 0xE1, 0x00, 0x40, 0x67];
    assert!(parameter_sets(&avcc).0.is_empty());
    // One PPS whose length runs past the record.
    let avcc = [0x01, 0x42, 0xC0, 0x1E, 0xFF, 0xE0, 0x01, 0x00, 0x40, 0x68];
    assert!(parameter_sets(&avcc).0.is_empty());
    // A count of zero SPS, then one PPS.
    let avcc = [
        0x01, 0x42, 0xC0, 0x1E, 0xFF, 0xE0, 0x01, 0x00, 0x02, 0x68, 0xCE,
    ];
    let (sets, length_size) = parameter_sets(&avcc);
    assert_eq!(length_size, 4);
    assert_eq!(sets, [&[0x68, 0xCE][..]]);
}

#[test]
fn every_parameter_set_of_a_record_is_read() {
    // Two SPS and two PPS: after each set the offset has to move by its length and its prefix.
    let mut avcc = vec![0x01, 0x42, 0xC0, 0x1E, 0xFF, 0xE2];
    for sps in [&[0x67u8, 0x01][..], &[0x67, 0x02, 0x03][..]] {
        avcc.extend_from_slice(&(sps.len() as u16).to_be_bytes());
        avcc.extend_from_slice(sps);
    }
    avcc.push(2);
    for pps in [&[0x68u8, 0x01][..], &[0x68, 0x02, 0x03, 0x04][..]] {
        avcc.extend_from_slice(&(pps.len() as u16).to_be_bytes());
        avcc.extend_from_slice(pps);
    }
    let (sets, size) = parameter_sets(&avcc);
    assert_eq!(size, 4);
    assert_eq!(
        sets,
        [
            &[0x67, 0x01][..],
            &[0x67, 0x02, 0x03][..],
            &[0x68, 0x01][..],
            &[0x68, 0x02, 0x03, 0x04][..]
        ]
    );
}

#[test]
fn the_first_chunk_and_sample_size_come_from_the_tables() {
    let mut stco = vec![0, 0, 0, 0];
    stco.extend_from_slice(&1u32.to_be_bytes());
    stco.extend_from_slice(&4321u32.to_be_bytes());
    assert_eq!(first_chunk(&be_box(b"stco", &stco)), Some(4321));

    let mut empty = vec![0, 0, 0, 0];
    empty.extend_from_slice(&0u32.to_be_bytes());
    assert_eq!(first_chunk(&be_box(b"stco", &empty)), None);

    let mut co64 = vec![0, 0, 0, 0];
    co64.extend_from_slice(&1u32.to_be_bytes());
    co64.extend_from_slice(&9_000_000_000u64.to_be_bytes());
    assert_eq!(first_chunk(&be_box(b"co64", &co64)), Some(9_000_000_000));
    assert_eq!(first_chunk(b""), None);

    let mut fixed = vec![0, 0, 0, 0];
    fixed.extend_from_slice(&7u32.to_be_bytes());
    fixed.extend_from_slice(&1u32.to_be_bytes());
    assert_eq!(first_sample_size(&be_box(b"stsz", &fixed)), Some(7));

    let mut variable = vec![0, 0, 0, 0];
    variable.extend_from_slice(&0u32.to_be_bytes());
    variable.extend_from_slice(&1u32.to_be_bytes());
    variable.extend_from_slice(&12u32.to_be_bytes());
    assert_eq!(first_sample_size(&be_box(b"stsz", &variable)), Some(12));

    let mut none = vec![0, 0, 0, 0];
    none.extend_from_slice(&0u32.to_be_bytes());
    none.extend_from_slice(&0u32.to_be_bytes());
    assert_eq!(first_sample_size(&be_box(b"stsz", &none)), None);
    assert_eq!(first_sample_size(b""), None);
}

#[test]
fn annex_b_stops_at_a_length_that_runs_past_the_sample() {
    assert_eq!(annex_b(&[], 4), Vec::<u8>::new());
    let mut truncated = 5u32.to_be_bytes().to_vec();
    truncated.extend_from_slice(&[1, 2, 3]);
    assert_eq!(annex_b(&truncated, 4), Vec::<u8>::new());
    let mut exact = 3u32.to_be_bytes().to_vec();
    exact.extend_from_slice(&[9, 9, 9]);
    assert_eq!(annex_b(&exact, 4), [0, 0, 0, 1, 9, 9, 9]);
    // A two-byte length of 256 does not become 257.
    let mut wide = 256u16.to_be_bytes().to_vec();
    wide.extend(std::iter::repeat_n(0x42, 256));
    let out = annex_b(&wide, 2);
    assert_eq!(out.len(), 260);
    assert_eq!(&out[..4], &[0, 0, 0, 1]);
}

// ------------------------------------------------------------------ Matroska

fn vint(size: usize) -> Vec<u8> {
    if size < 128 {
        vec![0x80 | size as u8]
    } else if size < 16_384 {
        vec![0x40 | (size >> 8) as u8, size as u8]
    } else {
        panic!("the tests never build that big an element");
    }
}

fn element(id: &[u8], payload: &[u8]) -> Vec<u8> {
    let mut out = id.to_vec();
    out.extend_from_slice(&vint(payload.len()));
    out.extend_from_slice(payload);
    out
}

fn webm_document(scale: Option<&[u8]>, duration: f32) -> Vec<u8> {
    let mut info = Vec::new();
    if let Some(scale) = scale {
        info.extend_from_slice(&element(&[0x2A, 0xD7, 0xB1], scale));
    }
    info.extend_from_slice(&element(&[0x44, 0x89], &duration.to_be_bytes()));
    let segment = element(
        &[0x18, 0x53, 0x80, 0x67],
        &element(&[0x15, 0x49, 0xA9, 0x66], &info),
    );
    concat(&[element(&[0x1A, 0x45, 0xDF, 0xA3], b"webm"), segment])
}

#[test]
fn a_webm_gives_its_duration_and_size() {
    let mut video = element(&[0xB0], &[0x02, 0x80]); // 640
    video.extend_from_slice(&element(&[0xBA], &[0x01, 0xE0])); // 480
    let tracks = element(
        &[0x16, 0x54, 0xAE, 0x6B],
        &element(&[0xAE], &element(&[0xE0], &video)),
    );
    let mut document = webm_document(Some(&[0x0F, 0x42, 0x40]), 1500.0);
    // The tracks element goes beside the info element, inside the Segment.
    let segment = element(&[0x18, 0x53, 0x80, 0x67], &tracks);
    document.extend_from_slice(&segment);
    let path = scratch("webm");
    std::fs::write(&path, document).unwrap();
    let info = probe(&path).unwrap();
    assert_eq!(info.duration_ms, Some(1500));
    assert_eq!(info.width, Some(640));
    assert_eq!(info.height, Some(480));
    assert!(info.h264.is_none());
    let _ = std::fs::remove_file(&path);
}

#[test]
fn a_webm_says_which_scale_its_duration_uses() {
    let path = scratch("webm-scale");
    std::fs::write(&path, webm_document(Some(&[0x07, 0xA1, 0x20]), 1500.0)).unwrap();
    assert_eq!(
        ebml_info(&path).unwrap().duration_ms,
        Some(750),
        "a 500 000 scale halves the microseconds"
    );
    let _ = std::fs::remove_file(&path);

    let path = scratch("webm-default-scale");
    std::fs::write(&path, webm_document(None, 1500.0)).unwrap();
    assert_eq!(
        ebml_info(&path).unwrap().duration_ms,
        Some(1500),
        "with no scale the default is a millisecond"
    );
    let _ = std::fs::remove_file(&path);
}

#[test]
fn a_value_five_segments_deep_is_still_read() {
    let mut payload = element(&[0x44, 0x89], &1000.0f32.to_be_bytes());
    for _ in 0..5 {
        payload = element(&[0x18, 0x53, 0x80, 0x67], &payload);
    }
    let path = scratch("webm-depth-five");
    std::fs::write(&path, payload).unwrap();
    assert_eq!(
        ebml_info(&path).unwrap().duration_ms,
        Some(1000),
        "the fifth level of nesting is still inside the walk"
    );
    let _ = std::fs::remove_file(&path);
}

#[test]
fn the_ebml_primitives_read_what_they_should() {
    assert_eq!(ebml_vint(&[0x85], false), Some((5, 1)));
    assert_eq!(ebml_vint(&[0x85], true), Some((0x85, 1)));
    assert_eq!(ebml_vint(&[0x40, 0x05], false), Some((5, 2)));
    assert_eq!(ebml_vint(&[0x20, 0xA0, 0xB0], false), Some((0xA0B0, 3)));
    assert_eq!(
        ebml_vint(&[0x01, 0, 0, 0, 0, 0, 0, 0], false),
        Some((0, 8)),
        "eight bytes of length, the widest an element can use"
    );
    assert_eq!(
        ebml_vint(&[0x00], false),
        None,
        "a length wider than eight bytes"
    );
    assert_eq!(
        ebml_vint(&[0x40], false),
        None,
        "a length with no second byte"
    );
    assert_eq!(ebml_vint(&[], false), None);

    assert_eq!(
        ebml_element(&[0x44, 0x89, 0x84, 1, 2, 3, 4]),
        Some((0x4489, 4, 3))
    );
    assert_eq!(ebml_element(&[0x44]), None);

    assert_eq!(uint(&[0x01, 0x01]), Some(0x0101));
    assert_eq!(
        uint(&[0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08]),
        Some(0x0102_0304_0506_0708),
        "eight bytes are still an unsigned integer"
    );
    assert_eq!(uint(&[]), None);
    assert_eq!(uint(&[0; 9]), None);

    assert_eq!(float(&1.0f32.to_be_bytes()), Some(1.0));
    assert_eq!(float(&2.0f64.to_be_bytes()), Some(2.0));
    assert_eq!(float(&[0, 0, 0]), None);
    assert_eq!(float(&[]), None);
}

#[test]
fn an_ebml_element_that_lies_about_its_size_ends_the_walk() {
    let mut broken = vec![0x44, 0x89];
    broken.extend_from_slice(&vint(1000));
    broken.extend_from_slice(&1.0f32.to_be_bytes());
    let path = scratch("broken-ebml");
    std::fs::write(&path, broken).unwrap();
    assert_eq!(ebml_info(&path).unwrap().duration_ms, None);
    let _ = std::fs::remove_file(&path);

    // A value nested deeper than the walk goes: it is not read, and nothing panics.
    let mut payload = element(&[0x44, 0x89], &1.0f64.to_be_bytes());
    for _ in 0..8 {
        payload = element(&[0x18, 0x53, 0x80, 0x67], &payload);
    }
    let path = scratch("deep-ebml");
    std::fs::write(&path, &payload).unwrap();
    assert_eq!(ebml_info(&path).unwrap().duration_ms, None);
    let _ = std::fs::remove_file(&path);
}

// ------------------------------------------------------------------ Ogg

fn ogg_page(header_type: u8, granule: u64, lacing: &[u8], body: &[u8]) -> Vec<u8> {
    let mut page = Vec::new();
    page.extend_from_slice(b"OggS");
    page.push(0);
    page.push(header_type);
    page.extend_from_slice(&granule.to_le_bytes());
    page.extend_from_slice(&0u32.to_le_bytes());
    page.extend_from_slice(&0u32.to_le_bytes());
    page.extend_from_slice(&0u32.to_le_bytes());
    page.push(lacing.len() as u8);
    page.extend_from_slice(lacing);
    page.extend_from_slice(body);
    page
}

fn opus_head() -> Vec<u8> {
    let mut head = b"OpusHead".to_vec();
    head.extend_from_slice(&[1, 2, 0x38, 0x01, 0x80, 0xBB, 0, 0, 0, 0, 0]);
    head
}

fn opus_file(name: &str, pages: &[Vec<u8>]) -> std::path::PathBuf {
    let path = scratch(name);
    std::fs::write(&path, concat(pages)).unwrap();
    path
}

#[test]
fn an_ogg_gives_its_channels_a_duration_and_its_audio_packets() {
    let head = opus_head();
    let tags = b"OpusTags\0\0\0\0".to_vec();
    let pages = [
        ogg_page(0x02, 0, &[head.len() as u8], &head),
        ogg_page(0, 0, &[tags.len() as u8], &tags),
        ogg_page(0x04, 96_000, &[3], &[1, 2, 3]),
    ];
    let path = opus_file("ogg", &pages);
    let ogg = ogg(&path).unwrap();
    assert_eq!(ogg.channels, 2);
    assert_eq!(ogg.duration_ms, Some(2000));
    assert_eq!(ogg.packets.len(), 1);
    assert_eq!(ogg.packets[0].bytes, [1, 2, 3]);
    let _ = std::fs::remove_file(path);
}

#[test]
fn a_packet_that_spans_two_pages_is_one_packet() {
    let head = opus_head();
    let mut first = ogg_page(0x02, 0, &[head.len() as u8], &head);
    // A page that begins with the previous packet's tail, then a 255-byte segment that does not end.
    let mut tail = ogg_page(0x01, 0, &[255], &[7u8; 255]);
    // A second page closes the packet with three bytes.
    let last = ogg_page(0x04, 0, &[3], &[8, 8, 8]);
    first.append(&mut tail);
    first.extend_from_slice(&last);
    let path = opus_file("ogg-span", &[first]);
    let ogg = ogg(&path).unwrap();
    assert_eq!(ogg.packets.len(), 1, "one packet, assembled from two pages");
    assert_eq!(ogg.packets[0].bytes.len(), 258);
    assert_eq!(&ogg.packets[0].bytes[..255], &[7u8; 255]);
    let _ = std::fs::remove_file(path);
}

#[test]
fn an_ogg_without_audio_or_without_a_header_is_an_error() {
    let path = scratch("not-ogg");
    std::fs::write(&path, b"this file carries no OggS page at all").unwrap();
    let error = ogg(&path).err().expect("a bad header is an error");
    assert!(error.message.contains("without a page header"));
    let _ = std::fs::remove_file(path);

    let head = opus_head();
    let path = opus_file(
        "ogg-no-audio",
        &[ogg_page(0x04, 10, &[head.len() as u8], &head)],
    );
    let error = ogg(&path).err().expect("no audio packet is an error");
    assert!(error.message.contains("without audio packets"));
    let _ = std::fs::remove_file(path);
}

#[test]
fn a_granule_of_all_ones_is_not_a_time() {
    let head = opus_head();
    let pages = [
        ogg_page(0x02, 0, &[head.len() as u8], &head),
        ogg_page(0, 48_000, &[1], &[1]),
        ogg_page(0x04, u64::MAX, &[1], &[2]),
    ];
    let path = opus_file("ogg-granule", &pages);
    assert_eq!(ogg(&path).unwrap().duration_ms, Some(1000));
    let _ = std::fs::remove_file(path);
}

// ------------------------------------------------------------------ end to end

#[test]
fn a_whole_mp4_built_here_reads_back() {
    let sps = [0x67, 0x42, 0xC0];
    let pps = [0x68, 0xCE];
    let sample = [0u8, 0, 0, 3, 0x65, 0x88, 0x84];

    let mut mvhd = vec![0u8; 20];
    mvhd[12..16].copy_from_slice(&1000u32.to_be_bytes());
    mvhd[16..20].copy_from_slice(&1500u32.to_be_bytes());

    let mut tkhd = vec![0u8; 84];
    tkhd[76..80].copy_from_slice(&(640u32 << 16).to_be_bytes());
    tkhd[80..84].copy_from_slice(&(480u32 << 16).to_be_bytes());

    let mut hdlr = vec![0u8; 8];
    hdlr.extend_from_slice(b"vide");

    // An audio track before the video one: the first track is not the picture.
    let audio_tkhd = vec![0u8; 84];
    let mut audio_hdlr = vec![0u8; 8];
    audio_hdlr.extend_from_slice(b"soun");
    let audio_trak = be_box(
        b"trak",
        &concat(&[
            be_box(b"tkhd", &audio_tkhd),
            be_box(b"mdia", &be_box(b"hdlr", &audio_hdlr)),
        ]),
    );

    let build_moov = |chunk: u32| {
        let stbl = stbl_of(
            Some(b"avc1"),
            Some(&avcc(3, &sps, &pps)),
            chunk,
            sample.len() as u32,
        );
        let minf = be_box(b"minf", &be_box(b"stbl", &stbl));
        let mdia = be_box(b"mdia", &concat(&[be_box(b"hdlr", &hdlr), minf]));
        let trak = be_box(b"trak", &concat(&[be_box(b"tkhd", &tkhd), mdia]));
        be_box(
            b"moov",
            &concat(&[be_box(b"mvhd", &mvhd), audio_trak.clone(), trak]),
        )
    };

    let ftyp = be_box(b"ftyp", b"isom\0\0\0\0");
    let provisional = build_moov(0);
    let chunk = ftyp.len() as u32 + provisional.len() as u32 + 8;
    let moov = build_moov(chunk);
    let mdat = be_box(b"mdat", &sample);
    let path = scratch("whole-mp4");
    std::fs::write(&path, concat(&[ftyp, moov, mdat])).unwrap();

    let info = probe(&path).unwrap();
    assert_eq!(info.duration_ms, Some(1500));
    assert_eq!(info.width, Some(640));
    assert_eq!(info.height, Some(480));
    let h264 = info.h264.expect("the first sample");
    assert!(h264.ends_with(&[0, 0, 0, 1, 0x65, 0x88, 0x84]));
    let _ = std::fs::remove_file(path);
}

#[test]
fn an_mp4_without_a_video_track_is_an_error() {
    let ftyp = be_box(b"ftyp", b"isom\0\0\0\0");
    let moov = be_box(b"moov", &be_box(b"mvhd", &[0u8; 20]));
    let path = scratch("no-track");
    std::fs::write(&path, concat(&[ftyp.clone(), moov])).unwrap();
    let error = probe(&path).unwrap_err();
    assert!(error.message.contains("without a video track"));
    let _ = std::fs::remove_file(path);

    let path = scratch("no-moov");
    std::fs::write(&path, be_box(b"ftyp", b"isom\0\0\0\0")).unwrap();
    let error = probe(&path).unwrap_err();
    assert!(error.message.contains("without a moov box"));
    let _ = std::fs::remove_file(path);

    // A movie with only a sound track: the track exists, but it is not a picture.
    let audio_hdlr = {
        let mut payload = vec![0u8; 8];
        payload.extend_from_slice(b"soun");
        payload
    };
    let audio_trak = be_box(
        b"trak",
        &concat(&[
            be_box(b"tkhd", &[0u8; 84]),
            be_box(b"mdia", &be_box(b"hdlr", &audio_hdlr)),
        ]),
    );
    let moov = be_box(b"moov", &concat(&[be_box(b"mvhd", &[0u8; 20]), audio_trak]));
    let path = scratch("audio-only");
    std::fs::write(&path, concat(&[ftyp.clone(), moov])).unwrap();
    let error = probe(&path).unwrap_err();
    assert!(
        error.message.contains("without a video track"),
        "a sound track is not a picture: {}",
        error.message
    );
    let _ = std::fs::remove_file(path);
}
