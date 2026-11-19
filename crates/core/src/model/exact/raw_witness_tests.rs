//! Disposable-source probes for equal-stamp metadata and bounded byte proofs.
use super::*;
use crate::model::TagEngine;
use std::{
    cell::Cell,
    fs::{self, File, OpenOptions},
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicU64, Ordering},
        Mutex,
    },
};

static INPUTS: Mutex<()> = Mutex::new(());

struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "tagrex-exact-witness-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).unwrap();
        Self(path)
    }
    fn put(&self, name: &str, bytes: &[u8]) -> PathBuf {
        let path = self.0.join(name);
        fs::write(&path, bytes).unwrap();
        path
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).unwrap();
    }
}

fn block(out: &mut Vec<u8>, kind: u8, body: &[u8]) {
    let len = body.len();
    out.extend([kind, (len >> 16) as u8, (len >> 8) as u8, len as u8]);
    out.extend(body);
}

fn flac(picture: bool) -> (Vec<u8>, usize, usize, usize) {
    let mut bytes = b"fLaC".to_vec();
    let mut info = [0; 34];
    info[..4].copy_from_slice(&[0x10, 0, 0x10, 0]);
    info[10..18].copy_from_slice(&[0x0a, 0xc4, 0x42, 0xf0, 0, 0, 0, 0]);
    block(&mut bytes, 0, &info);
    let mut text = 0u32.to_le_bytes().to_vec();
    text.extend(1u32.to_le_bytes());
    text.extend(12u32.to_le_bytes());
    text.extend(b"TITLE=Before");
    let header = bytes.len();
    block(&mut bytes, if picture { 4 } else { 0x84 }, &text);
    let title = bytes.windows(6).position(|b| b == b"Before").unwrap();
    let mut picture_byte = 0;
    if picture {
        let mut body = 3u32.to_be_bytes().to_vec();
        body.extend(9u32.to_be_bytes());
        body.extend(b"image/png");
        body.extend(0u32.to_be_bytes());
        for dimension in [1u32, 1, 24, 0] {
            body.extend(dimension.to_be_bytes());
        }
        body.extend(4u32.to_be_bytes());
        body.extend(b"PNG!");
        block(&mut bytes, 0x86, &body);
        picture_byte = bytes.len() - 1;
    }
    let metadata_end = bytes.len();
    bytes.extend([
        0xff, 0xf8, 0xc9, 0x18, 0, 0xc2, 0, 0, 0, 0, 0, 0, 0xb8, 0xee,
    ]);
    assert!(title < metadata_end && header < metadata_end);
    (bytes, title, header, picture_byte)
}

#[test]
fn equal_stamp_text_header_and_picture_changes_refuse_named_and_held_sources() {
    let _guard = INPUTS.lock().unwrap_or_else(|p| p.into_inner());
    let fixture = Fixture::new();
    let (original, text, header, picture) = flac(true);
    for held in [false, true] {
        for (kind, at) in [("text", text), ("header", header), ("picture", picture)] {
            let path = fixture.put(&format!("{kind}-{held}.flac"), &original);
            let mut input = if held {
                Input::prepare_file(File::open(&path).unwrap(), &|| false).unwrap()
            } else {
                Input::prepare(&path, &|| false).unwrap()
            };
            input.ensure_unchanged().unwrap();
            let mut changed = original.clone();
            changed[at] ^= 1;
            fs::write(&path, &changed).unwrap();
            assert!(!input.witness.matches(&input.source).unwrap());
            // Deterministically emulate a filesystem reporting the same stamp
            // for two byte states. The original Windows public-API canaries
            // supply the actual platform failure; this is collision emulation.
            input.stamp = Stamp::from_file(&input.source).unwrap();
            assert!(matches!(
                input.ensure_unchanged(),
                Err(ExactReadError::Changed)
            ));
            assert_eq!(fs::read(path).unwrap(), changed);
        }
    }
}

fn named_or_held(
    path: &Path,
    held: bool,
    cancelled: impl Fn() -> bool,
) -> Result<ExactTrackRead, ExactReadError> {
    if held {
        TagEngine::read_exact_file_with_cancel(File::open(path).unwrap(), path, cancelled)
    } else {
        TagEngine::read_exact_with_cancel(path, cancelled)
    }
}

#[test]
fn final_callback_same_length_write_with_restored_mtime_is_refused() {
    let _guard = INPUTS.lock().unwrap_or_else(|p| p.into_inner());
    let fixture = Fixture::new();
    let (original, title, _, _) = flac(false);
    for held in [false, true] {
        let path = fixture.put(&format!("public-{held}.flac"), &original);
        let calls = Cell::new(0);
        named_or_held(&path, held, || {
            calls.set(calls.get() + 1);
            false
        })
        .unwrap();
        let modified = fs::metadata(&path).unwrap().modified().unwrap();
        let mut changed = original.clone();
        changed[title..title + 6].copy_from_slice(b"After!");
        let seen = Cell::new(0);
        let result = named_or_held(&path, held, || {
            seen.set(seen.get() + 1);
            if seen.get() == calls.get() {
                fs::write(&path, &changed).unwrap();
                OpenOptions::new()
                    .write(true)
                    .open(&path)
                    .unwrap()
                    .set_times(std::fs::FileTimes::new().set_modified(modified))
                    .unwrap();
            }
            false
        });
        assert_eq!(seen.get(), calls.get());
        assert!(matches!(result, Err(ExactReadError::Changed)));
        assert_eq!(fs::read(path).unwrap(), changed);
    }
}

fn mpeg() -> Vec<u8> {
    let mut bytes = b"ID3\x04\0\0\0\0\0\0".to_vec();
    let mut frame = vec![0xff, 0xfb, 0x90, 0];
    frame.resize(417, 0);
    bytes.extend(frame.repeat(3));
    bytes
}

fn ape(header: bool) -> Vec<u8> {
    let mut item = 6u32.to_le_bytes().to_vec();
    item.extend(0u32.to_le_bytes());
    item.extend(b"Title\0Before");
    let mut footer = b"APETAGEX".to_vec();
    footer.extend(2000u32.to_le_bytes());
    footer.extend(((item.len() + 32) as u32).to_le_bytes());
    footer.extend(1u32.to_le_bytes());
    let flags = if header { 1u32 << 31 } else { 0 };
    footer.extend(flags.to_le_bytes());
    footer.extend([0; 8]);
    let mut bytes = Vec::new();
    if header {
        let mut opening = footer.clone();
        opening[20..24].copy_from_slice(&((1u32 << 31) | (1 << 29)).to_le_bytes());
        bytes.extend(opening);
    }
    bytes.extend(item);
    bytes.extend(footer);
    bytes
}

#[test]
fn equal_stamp_id3v1_and_ape_tail_changes_are_refused() {
    let _guard = INPUTS.lock().unwrap_or_else(|p| p.into_inner());
    let fixture = Fixture::new();
    for id3v1 in [false, true] {
        let mut original = mpeg();
        let ape_value = original.len() + 14;
        original.extend(ape(false));
        let id3_value = original.len() + 3;
        if id3v1 {
            let mut tail = b"TAGBefore".to_vec();
            tail.resize(128, 0);
            original.extend(tail);
        }
        for held in [false, true] {
            for at in [Some(ape_value), id3v1.then_some(id3_value)]
                .into_iter()
                .flatten()
            {
                let path = fixture.put("tail.mp3", &original);
                let mut input = if held {
                    Input::prepare_file(File::open(&path).unwrap(), &|| false).unwrap()
                } else {
                    Input::prepare(&path, &|| false).unwrap()
                };
                let mut changed = original.clone();
                changed[at] ^= 1;
                fs::write(&path, &changed).unwrap();
                assert!(!input.witness.matches(&input.source).unwrap());
                input.stamp = Stamp::from_file(&input.source).unwrap();
                assert!(matches!(
                    input.ensure_unchanged(),
                    Err(ExactReadError::Changed)
                ));
                assert_eq!(fs::read(path).unwrap(), changed);
            }
        }
    }
}

#[test]
fn ape_before_audio_is_witnessed_and_lyrics3_is_refused() {
    let _guard = INPUTS.lock().unwrap_or_else(|p| p.into_inner());
    let fixture = Fixture::new();
    let mut bytes = b"ID3\x04\0\0\0\0\0\0".to_vec();
    let value = bytes.len() + 32 + 14;
    bytes.extend(ape(true));
    bytes.extend(&mpeg()[10..]);
    let path = fixture.put("leading.mp3", &bytes);
    let mut input = Input::prepare(&path, &|| false).unwrap();
    bytes[value] ^= 1;
    fs::write(&path, &bytes).unwrap();
    input.stamp = Stamp::from_file(&input.source).unwrap();
    assert!(!input.witness.matches(&input.source).unwrap());
    assert!(matches!(
        input.ensure_unchanged(),
        Err(ExactReadError::Changed)
    ));
    drop(input);

    let mut lyrics = mpeg();
    lyrics.extend(b"000011LYRICS200");
    let path = fixture.put("lyrics.mp3", &lyrics);
    assert!(matches!(
        Input::prepare(&path, &|| false),
        Err(ExactReadError::Unavailable(
            UnavailableReason::UnsupportedLayout
        ))
    ));
}

#[test]
fn contiguous_observations_coalesce_and_replay_the_same_bytes() {
    let fixture = Fixture::new();
    let path = fixture.put("observed.bin", b"abcd----ef");
    let mut builder = WitnessBuilder::new();
    builder.observe(0, b"ab").unwrap();
    builder.observe(2, b"cd").unwrap();
    builder.observe(8, b"ef").unwrap();
    let witness = builder.finish().unwrap();
    assert_eq!(witness.spans.len(), 2);
    assert!(witness.matches(&File::open(path).unwrap()).unwrap());
}

#[test]
fn fragmented_observations_cannot_exceed_the_range_memory_bound() {
    let mut builder = WitnessBuilder::new();
    for at in 0..WITNESS_RANGE_LIMIT {
        builder.observe((at * 2) as u64, &[1]).unwrap();
    }
    assert!(matches!(
        builder.observe((WITNESS_RANGE_LIMIT * 2) as u64, &[1]),
        Err(ExactReadError::Limit)
    ));
    assert!(builder.spans.capacity() * std::mem::size_of::<ReadSpan>() <= WITNESS_RANGE_BYTES);
}

#[test]
fn observed_work_is_admitted_before_reading_past_its_limit() {
    let mut builder = WitnessBuilder::new();
    let chunk = [0; WITNESS_CHUNK];
    let mut at = 0;
    while at < WITNESS_IO_LIMIT {
        builder.observe(at, &chunk).unwrap();
        at += WITNESS_CHUNK as u64;
    }
    assert!(matches!(builder.admit(1), Err(ExactReadError::Limit)));
    assert!(matches!(
        builder.observe(at, &[0]),
        Err(ExactReadError::Limit)
    ));
    assert_eq!(builder.finish().unwrap().spans.len(), 1);
}

fn atom(kind: &[u8; 4], body: &[u8]) -> Vec<u8> {
    let mut bytes = ((body.len() + 8) as u32).to_be_bytes().to_vec();
    bytes.extend(kind);
    bytes.extend(body);
    bytes
}

#[test]
fn nested_metadata_does_not_repeat_complete_parent_payload_proofs() {
    let _guard = INPUTS.lock().unwrap_or_else(|p| p.into_inner());
    let fixture = Fixture::new();
    let mut body = atom(b"free", &vec![0; 64 * 1024]);
    body.extend(atom(b"ilst", &[]));
    for _ in 0..4 {
        body = atom(b"moov", &body);
    }
    let path = fixture.put("nested.m4a", &body);
    let input = Input::prepare(&path, &|| false).unwrap();
    let observed: u64 = input.witness.spans.iter().map(|span| span.len).sum();
    assert!(observed < body.len() as u64 * 2);
    input.ensure_unchanged().unwrap();
}

#[test]
fn witness_ranges_stop_before_flac_audio() {
    let _guard = INPUTS.lock().unwrap_or_else(|p| p.into_inner());
    let fixture = Fixture::new();
    let (mut bytes, _, _, _) = flac(true);
    let metadata_end = bytes.len() - 14;
    bytes.extend([0; 64 * 1024]);
    let path = fixture.put("audio.flac", &bytes);
    let input = Input::prepare(&path, &|| false).unwrap();
    for span in &input.witness.spans {
        assert!(span.at + span.len <= metadata_end as u64);
    }
    input.ensure_unchanged().unwrap();
}
