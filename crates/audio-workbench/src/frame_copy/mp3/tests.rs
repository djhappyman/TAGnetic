use super::*;
use std::io::ErrorKind;
use std::path::Path;
use std::process::Command;

fn ffmpeg(args: &[&str]) -> Option<Vec<u8>> {
    match Command::new("ffmpeg")
        .args(["-nostdin", "-v", "error", "-xerror"])
        .args(args)
        .output()
    {
        Ok(result) => {
            assert!(
                result.status.success(),
                "{}",
                String::from_utf8_lossy(&result.stderr)
            );
            Some(result.stdout)
        }
        Err(error)
            if error.kind() == ErrorKind::NotFound
                && std::env::var_os("TAGREX_REQUIRE_REFERENCE_CODECS").is_none() =>
        {
            eprintln!("ffmpeg unavailable, skipped independent MP3 frame-copy readback");
            None
        }
        Err(error) => panic!("ffmpeg unavailable: {error}"),
    }
}

fn decode(path: &Path) -> Vec<u8> {
    ffmpeg(&[
        "-i",
        path.to_str().unwrap(),
        "-f",
        "s16le",
        "-c:a",
        "pcm_s16le",
        "-",
    ])
    .unwrap()
}

fn packet_bytes(held: &mut File, packet: &Packet) -> Vec<u8> {
    let mut bytes = vec![0_u8; usize::from(packet.len)];
    held.seek(SeekFrom::Start(packet.offset)).unwrap();
    held.read_exact(&mut bytes).unwrap();
    bytes
}

#[test]
fn copied_mp3_keeps_frame_bytes_and_pcm_across_reservoir_cuts() {
    let root = tempfile::tempdir().unwrap();
    for (label, sample_rate, channels, setting, rate, xing) in [
        ("cbr", "44100", "2", "-b:a", "128k", "1"),
        ("vbr", "44100", "2", "-q:a", "4", "1"),
        ("vbr-no-xing", "44100", "2", "-q:a", "5", "0"),
        ("cbr-no-xing", "44100", "2", "-b:a", "96k", "0"),
        ("mpeg2", "22050", "1", "-q:a", "5", "1"),
        ("mpeg25", "8000", "1", "-b:a", "24k", "1"),
    ] {
        let source = root.path().join(format!("{label}.mp3"));
        let waveform = format!("aevalsrc=0.4*sin(2*PI*440*t)*sin(2*PI*43*t)*sin(2*PI*2*t):sample_rate={sample_rate}:duration=2.371");
        if ffmpeg(&[
            "-f",
            "lavfi",
            "-i",
            &waveform,
            "-ac",
            channels,
            "-c:a",
            "libmp3lame",
            setting,
            rate,
            "-metadata",
            "title=Reservoir proof 🎵",
            "-write_xing",
            xing,
            source.to_str().unwrap(),
        ])
        .is_none()
        {
            return;
        }
        let pcm = decode(&source);
        let stride = channels.parse::<usize>().unwrap() * 2;
        let frames = pcm.len() / stride;
        let owned = crate::inspect(&source, 4, || false).unwrap();
        assert_eq!(owned.source.frames, frames as u64, "{label} source");
        let mut held = File::open(&source).unwrap();
        let index = scan(&mut held, frames as u64, &mut || false).unwrap();
        assert!(
            index.packets.iter().any(|packet| packet.back != 0),
            "{label} reservoir"
        );
        let boundaries = index.boundaries();
        let cut = boundaries.len() * 3 / 7;
        let first = boundaries[cut];
        let next = boundaries[cut + 1];
        let cases = [
            (0, boundaries[1]),
            (0, first),
            (first, next),
            (next, frames as u64),
        ];
        for (part, (start, end)) in cases.into_iter().enumerate() {
            let result = root.path().join(format!("{label}-{part}.mp3"));
            let output = File::create(&result).unwrap();
            index
                .write(&mut held, FrameRange { start, end }, output, &mut || false)
                .unwrap();
            let native = crate::inspect(&result, 4, || false).unwrap();
            assert_eq!(
                native.source.frames,
                end - start,
                "{label} part {part} native duration"
            );
            let decoded = decode(&result);
            let expected = &pcm[start as usize * stride..end as usize * stride];
            assert_eq!(
                decoded.len(),
                expected.len(),
                "{label} part {part} FFmpeg duration"
            );
            let different = decoded.iter().zip(expected).position(|(a, b)| a != b);
            assert!(
                different.is_none(),
                "{label} part {part} first differing PCM byte at {different:?}"
            );

            let mut output = File::open(&result).unwrap();
            let copy = scan(&mut output, end - start, &mut || false).unwrap();
            let samples = index.stream.version.samples_per_frame() as u64;
            let first_audible = (index.delay + start) / samples;
            let end_audible = (index.delay + end).div_ceil(samples);
            let first_copied = copy.delay / samples;
            for (relative, original) in index.packets[first_audible as usize..end_audible as usize]
                .iter()
                .enumerate()
            {
                let copied = copy.packets[first_copied as usize + relative];
                assert_eq!(
                    packet_bytes(&mut held, original),
                    packet_bytes(&mut output, &copied),
                    "{label} part {part} encoded packet {relative}"
                );
            }
        }
    }
}

#[test]
fn scan_reports_cancellation_before_opening_the_mp3() {
    let mut owned = tempfile::tempfile().unwrap();
    assert!(matches!(
        scan(&mut owned, 1, &mut || true),
        Err(MediaError::Cancelled)
    ));
}

#[test]
fn protected_lame_timing_aligns_with_ffmpeg_and_unverified_timing_is_refused() {
    let root = tempfile::tempdir().unwrap();
    let waveform = root.path().join("source.wav");
    if ffmpeg(&[
        "-f",
        "lavfi",
        "-i",
        "sine=frequency=440:duration=2:sample_rate=44100",
        "-ac",
        "2",
        waveform.to_str().unwrap(),
    ])
    .is_none()
    {
        return;
    }

    let source = root.path().join("protected.mp3");
    let encoded = match Command::new("lame")
        .args(["--silent", "-p", "-b", "192"])
        .arg(&waveform)
        .arg(&source)
        .output()
    {
        Ok(output) => output,
        Err(error) if error.kind() == ErrorKind::NotFound => {
            eprintln!("Skipping independent CRC-protected fixture because LAME is unavailable");
            return;
        }
        Err(error) => panic!("LAME could not run: {error}"),
    };
    assert!(encoded.status.success());
    let original = std::fs::read(&source).unwrap();
    let first = FrameHeader::parse(original[..4].try_into().unwrap()).unwrap();
    assert!(first.crc_protected);

    let reference = decode(&source);
    let declared = crate::inspect(&source, 1, || false).unwrap().source.frames;
    assert_eq!(declared, 88_200);
    assert_eq!(declared as usize * 4, reference.len());
    let mut protected = File::open(&source).unwrap();
    let index = scan(&mut protected, declared, &mut || false).unwrap();
    let start = index.boundaries()[18];
    let end = index.boundaries()[19];
    let copied = root.path().join("protected-part.mp3");
    index
        .write(
            &mut protected,
            FrameRange { start, end },
            File::create(&copied).unwrap(),
            &mut || false,
        )
        .unwrap();
    assert_eq!(
        decode(&copied),
        reference[start as usize * 4..end as usize * 4]
    );
    assert_eq!(std::fs::read(&source).unwrap(), original);

    let invalid_timing = root.path().join("invalid-timing.mp3");
    let checksum = timing::checked_crc_offset(&original[..first.frame_size()], &first).unwrap();
    let mut invalid = original.clone();
    invalid[checksum] ^= 1;
    std::fs::write(&invalid_timing, invalid).unwrap();
    let apparent_frames = crate::inspect(&invalid_timing, 1, || false)
        .unwrap()
        .source
        .frames;
    assert!(matches!(
        scan(&mut File::open(&invalid_timing).unwrap(), apparent_frames, &mut || false),
        Err(MediaError::Unsupported(reason)) if reason.contains("CRC-protected timing")
    ));

    let headerless = root.path().join("protected-audio.mp3");
    std::fs::write(&headerless, &original[first.frame_size()..]).unwrap();
    let audio = decode(&headerless);
    let mut input = File::open(&headerless).unwrap();
    let len = (audio.len() / 4) as u64;
    let index = scan(&mut input, len, &mut || false).unwrap();
    let start = index.boundaries()[18];
    let copied = root.path().join("protected-headerless-part.mp3");
    index
        .write(
            &mut input,
            FrameRange { start, end: len },
            File::create(&copied).unwrap(),
            &mut || false,
        )
        .unwrap();
    assert!(decode(&copied) == audio[start as usize * 4..]);
}
