use super::*;
use std::fs;
use std::path::Path;
use std::process::Command;

const FIXED_SIZES: &[usize] = &[256; 134];
const VARIABLE_SIZES: &[usize] = &[91, 256, 64, 513, 128, 31, 7];

fn fixture(frames: &[usize], variable: bool) -> (StreamInfo, Vec<u8>, Vec<u8>) {
    let samples: usize = frames.iter().sum();
    let min_block = if variable {
        frames[..frames.len() - 1]
            .iter()
            .copied()
            .min()
            .unwrap_or(16)
    } else {
        frames[0]
    };
    let max_block = *frames.iter().max().unwrap();
    let fields = (44_100_u64 << 44) | (15_u64 << 36) | samples as u64;
    let mut info = [0; STREAM_INFO_LEN];
    info[..2].copy_from_slice(&(min_block as u16).to_be_bytes());
    info[2..4].copy_from_slice(&(max_block as u16).to_be_bytes());
    info[10..18].copy_from_slice(&fields.to_be_bytes());
    let info = StreamInfo(info);

    let mut flac = info.prefix().to_vec();
    let mut position = 0;
    let mut pcm = Vec::with_capacity(samples * 2);
    for (index, count) in frames.iter().copied().enumerate() {
        let number = if variable { position } else { index };
        let mut coded = [0; 7];
        let coded_len = write_number(number as u64, &mut coded);
        let mut packet = vec![0xff, if variable { 0xf9 } else { 0xf8 }, 0x79, 8];
        packet.extend_from_slice(&coded[..coded_len]);
        packet.extend_from_slice(&((count - 1) as u16).to_be_bytes());
        packet.push(crc8(&packet));
        // Verbatim mono subframe, 16 bits per sample.
        packet.push(0b10);
        for n in position..position + count {
            let sample = ((n * 1337 + (n % 31) * 7) % 36_929) as i16;
            packet.extend_from_slice(&sample.to_be_bytes());
            pcm.extend_from_slice(&sample.to_le_bytes());
        }
        packet.extend_from_slice(&crc16(&packet));
        flac.extend_from_slice(&packet);
        position += count;
    }
    (info, flac, pcm)
}

fn compare_reference_pcm(path: &Path, expected: &[u8]) {
    let ffmpeg = Command::new("ffmpeg")
        .args(["-v", "error", "-xerror", "-i"])
        .arg(path)
        .args(["-map", "0:a:0", "-f", "s16le", "-"])
        .output();
    let flac = Command::new("flac")
        .args([
            "--silent",
            "--decode",
            "--stdout",
            "--force-raw-format",
            "--endian=little",
            "--sign=signed",
            "--",
        ])
        .arg(path)
        .output();
    for (name, result) in [("FFmpeg", ffmpeg), ("reference FLAC", flac)] {
        match result {
            Ok(output) => {
                assert!(
                    output.status.success(),
                    "{name}: {}",
                    String::from_utf8_lossy(&output.stderr)
                );
                assert_eq!(output.stdout, expected, "{name}");
            }
            Err(error)
                if error.kind() == io::ErrorKind::NotFound
                    && std::env::var_os("TAGREX_REQUIRE_REFERENCE_CODECS").is_none() => {}
            Err(error) => panic!("{name} failed: {error}"),
        }
    }
}

fn test_split(sizes: &[usize], variable: bool, first: usize, last: usize) {
    let dir = tempfile::tempdir().unwrap();
    let original = dir.path().join("source.flac");
    let destination = dir.path().join("split.flac");
    let (info, source_bytes, samples) = fixture(sizes, variable);
    let frames = (samples.len() / 2) as u64;
    fs::write(&original, &source_bytes).unwrap();
    compare_reference_pcm(&original, &samples);

    let mut source = File::open(&original).unwrap();
    let index = scan(&mut source, frames, &mut || false).unwrap();
    assert_eq!(index.boundaries().last(), Some(&frames));
    assert_eq!(index.variable_blocks, variable);

    let range = FrameRange {
        start: index.boundaries()[first],
        end: index.boundaries()[last],
    };
    index
        .write(
            &mut source,
            range,
            File::create(&destination).unwrap(),
            &mut || false,
        )
        .unwrap();
    let output = fs::read(&destination).unwrap();
    let mut target = File::open(&destination).unwrap();
    let copied = scan(&mut target, range.end - range.start, &mut || false).unwrap();
    assert_eq!(copied.variable_blocks, variable);
    for (n, src_frame) in (first..last).enumerate() {
        let old =
            &source_bytes[index.offsets[src_frame] as usize..index.offsets[src_frame + 1] as usize];
        let new = &output[copied.offsets[n] as usize..copied.offsets[n + 1] as usize];
        let old_header = frame_header(old, info).unwrap();
        let new_header = frame_header(new, copied.stream_info).unwrap();
        assert_eq!(
            &old[old_header.crc_position + 1..old.len() - 2],
            &new[new_header.crc_position + 1..new.len() - 2]
        );
        assert_eq!(
            new_header.number,
            if variable {
                copied.boundaries()[n]
            } else {
                n as u64
            }
        );
    }
    assert_eq!(
        fs::read(original).unwrap(),
        source_bytes,
        "source audio bytes changed"
    );
    compare_reference_pcm(
        &destination,
        &samples[(range.start * 2) as usize..(range.end * 2) as usize],
    );
}

#[test]
fn fixed_flac_split_rebases_large_frame_numbers_and_copies_subframes() {
    let mut sizes = FIXED_SIZES.to_vec();
    sizes.push(11);
    test_split(&sizes, false, 129, sizes.len());
}

#[test]
fn variable_flac_split_rebases_sample_numbers_and_preserves_short_final_frame() {
    test_split(VARIABLE_SIZES, true, 1, VARIABLE_SIZES.len());
}

#[test]
fn reference_flac_with_master_tags_and_seek_table_copies_an_interior_span() {
    let dir = tempfile::tempdir().unwrap();
    let original = dir.path().join("reference.flac");
    let raw = dir.path().join("audio.raw");
    let (_, _, samples) = fixture(&[4096, 4096, 701], false);
    fs::write(&raw, &samples).unwrap();
    let encoded = Command::new("flac")
        .args([
            "--silent",
            "--force-raw-format",
            "--endian=little",
            "--sign=signed",
            "--channels=1",
            "--bps=16",
            "--sample-rate=44100",
            "--blocksize=512",
            "--tag=TITLE=Reference master",
            "--seekpoint=0",
            "--output-name",
        ])
        .arg(&original)
        .arg(&raw)
        .output();
    match encoded {
        Ok(output) => assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        ),
        Err(error)
            if error.kind() == io::ErrorKind::NotFound
                && std::env::var_os("TAGREX_REQUIRE_REFERENCE_CODECS").is_none() =>
        {
            return;
        }
        Err(error) => panic!("reference FLAC failed: {error}"),
    }

    let mut source = File::open(&original).unwrap();
    let original_bytes = fs::read(&original).unwrap();
    let index = scan(&mut source, (samples.len() / 2) as u64, &mut || false).unwrap();
    assert!(index.offsets[0] > PREFIX_LEN as u64);
    assert!(index.stream_info.0[18..].iter().any(|byte| *byte != 0));
    let range = FrameRange {
        start: index.boundaries()[4],
        end: index.boundaries()[11],
    };
    let output = dir.path().join("copied.flac");
    index
        .write(
            &mut source,
            range,
            File::create(&output).unwrap(),
            &mut || false,
        )
        .unwrap();
    let output_bytes = fs::read(&output).unwrap();
    assert_eq!(&output_bytes[26..42], &[0; 16]);
    assert_eq!(fs::read(&original).unwrap(), original_bytes);
    compare_reference_pcm(
        &output,
        &samples[(range.start * 2) as usize..(range.end * 2) as usize],
    );
}

#[test]
fn invalid_frames_and_off_boundary_writes_are_refused() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("master.flac");
    let output = dir.path().join("split.flac");
    let (_, mut bytes, samples) = fixture(&[32, 32, 32], false);
    let frames = (samples.len() / 2) as u64;
    fs::write(&path, &bytes).unwrap();
    let mut source = File::open(&path).unwrap();
    let index = scan(&mut source, frames, &mut || false).unwrap();

    let file = File::create(&output).unwrap();
    assert!(matches!(
        index.write(
            &mut source,
            FrameRange { start: 1, end: 32 },
            file,
            &mut || false
        ),
        Err(MediaError::Range(_))
    ));
    assert_eq!(fs::metadata(output).unwrap().len(), 0);
    assert!(matches!(
        scan(&mut source, frames, &mut || true),
        Err(MediaError::Cancelled)
    ));

    bytes[index.offsets[1] as usize - 4] ^= 1;
    fs::write(&path, &bytes).unwrap();
    assert!(scan(&mut source, frames, &mut || false).is_err());

    bytes[index.offsets[1] as usize - 4] ^= 1;
    bytes.pop();
    fs::write(&path, &bytes).unwrap();
    assert!(scan(&mut source, frames, &mut || false).is_err());
}

#[test]
fn flac_coded_numbers_use_canonical_widths() {
    let transitions = [
        0,
        0x7f,
        0x80,
        0x7ff,
        0x800,
        0xffff,
        0x1_0000,
        0x1f_ffff,
        0x20_0000,
        0x3ff_ffff,
        0x400_0000,
        0x7fff_ffff,
        0x8000_0000,
        SAMPLE_COUNT_MASK,
    ];
    for number in transitions {
        let mut bytes = [0; 7];
        let width = write_number(number, &mut bytes);
        assert_eq!(coded_number(&bytes[..width]), Some((number, width)));
    }
    assert_eq!(coded_number(&[0xc0, 0x80]), None);
    assert_eq!(coded_number(&[0xff, 0x80]), None);
    assert_eq!(coded_number(&[0xe2, 0x82]), None);
}
