use super::*;
use std::f64::consts::TAU;

fn source(rate: u32, channels: u16) -> SourceInfo {
    SourceInfo {
        sample_rate: rate,
        channels,
        frames: u64::from(rate),
        bits_per_sample: Some(24),
    }
}

fn sample(rate: u32, frame: usize, frequency: f64) -> f64 {
    (TAU * frame as f64 * frequency / f64::from(rate)).sin()
}

fn pcm(amplitude: f64) -> i32 {
    (amplitude * PCM_SCALE).round() as i32
}

fn treatment(bands: &[EqBand]) -> AudioTreatment {
    let mut eq_bands = [None; MAX_EQ_BANDS];
    for (slot, band) in eq_bands.iter_mut().zip(bands) {
        *slot = Some(*band);
    }
    AudioTreatment {
        eq_bands: Some(eq_bands),
        ..AudioTreatment::default()
    }
}

fn render(input: &[i32], source: SourceInfo, settings: AudioTreatment) -> Vec<i32> {
    let mut processor = PreparedTreatment::preview(settings, source)
        .unwrap()
        .processor(source)
        .unwrap();
    let mut output = Vec::new();
    processor.process(input, &mut output);
    output
}

fn amplitude(input: &[i32], source: SourceInfo, frequency: f64, channel: usize) -> f64 {
    let start = source.sample_rate as usize / 2;
    let (real, imag, frames) = input
        .chunks_exact(source.channels as usize)
        .enumerate()
        .skip(start)
        .fold((0.0, 0.0, 0), |(real, imag, count), (frame, values)| {
            let radians = TAU * frame as f64 * frequency / f64::from(source.sample_rate);
            let (sine, cosine) = radians.sin_cos();
            let value = f64::from(values[channel]) / PCM_SCALE;
            (real + value * cosine, imag + value * sine, count + 1)
        });
    2.0 * real.hypot(imag) / f64::from(frames)
}

fn spectral_band(channel: EqChannel, kind: EqBandKind, hz: u16, amount: f32) -> EqBand {
    EqBand {
        kind,
        channel,
        hz,
        q: 80,
        dynamic: Some(DynamicEq {
            threshold_db: -22.0,
            amount_db: amount,
            ratio: 8.0,
            spectral: true,
        }),
        ..Default::default()
    }
}

#[test]
fn spectral_eq_reduces_only_the_loud_frequency_inside_one_bell() {
    for rate in [48_000, 96_000] {
        for channels in [1, 2] {
            let source = source(rate, channels);
            let (loud, quiet, outside) = (1_125.0, 1_375.0, 5_000.0);
            let input: Vec<_> = (0..rate as usize)
                .flat_map(|frame| {
                    let tone = |hz| sample(rate, frame, hz);
                    let left = pcm(0.28 * tone(loud) + 0.030 * tone(quiet) + 0.14 * tone(outside));
                    let right =
                        pcm(0.025 * tone(loud) + 0.030 * tone(quiet) + 0.14 * tone(outside));
                    [left, right].into_iter().take(channels as usize)
                })
                .collect();
            let band = spectral_band(EqChannel::Stereo, EqBandKind::Bell, 1200, 12.0);
            let processed = render(&input, source, treatment(&[band]));
            assert_eq!(processed.len(), input.len());
            for channel in 0..channels as usize {
                let ratio = |hz| {
                    amplitude(&processed, source, hz, channel)
                        / amplitude(&input, source, hz, channel)
                };
                assert!(
                    ratio(loud) < 0.6,
                    "{rate} Hz, {channels}ch: loud {}",
                    ratio(loud)
                );
                assert!(
                    (0.93..1.07).contains(&ratio(quiet)),
                    "{rate} Hz, {channels}ch: quiet {}",
                    ratio(quiet)
                );
                assert!(
                    (0.99..1.01).contains(&ratio(outside)),
                    "{rate} Hz, {channels}ch: outside {}",
                    ratio(outside)
                );
            }
        }
    }
}

#[test]
fn a_sub_bin_spectral_bell_moves_continuously_and_spares_the_adjacent_quiet_tone() {
    for (rate, loud, centre) in [
        (44_100, 43.06640625, 53),
        (96_000, 46.875, 58),
        (192_000, 46.875, 70),
    ] {
        let source = source(rate, 1);
        let input: Vec<_> = (0..rate as usize)
            .map(|frame| pcm(0.093 * sample(rate, frame, loud)))
            .collect();
        let gain_at = |hz| {
            let mut band = spectral_band(EqChannel::Stereo, EqBandKind::Bell, hz, 12.0);
            band.q = 1200;
            band.dynamic.as_mut().unwrap().threshold_db = -72.0;
            let output = render(&input, source, treatment(&[band]));
            20.0 * (amplitude(&output, source, loud, 0) / amplitude(&input, source, loud, 0))
                .log10()
        };
        let gain_left = gain_at(centre);
        let gain_right = gain_at(centre + 1);
        assert!(
            (gain_left - gain_right).abs() < 1.5,
            "{rate}: {gain_left} / {gain_right} dB"
        );
    }

    let (rate, loud, quiet) = (48_000, 46.875, 70.3125);
    let source = source(rate, 1);
    let input: Vec<_> = (0..rate as usize)
        .map(|frame| pcm(0.4 * sample(rate, frame, loud) + 0.005 * sample(rate, frame, quiet)))
        .collect();
    let mut band = spectral_band(EqChannel::Stereo, EqBandKind::Bell, 47, 12.0);
    band.q = 1200;
    band.dynamic.as_mut().unwrap().threshold_db = -30.0;
    let output = render(&input, source, treatment(&[band]));
    // Measure a whole number of FFT periods so the strong tone cannot leak
    // into the quiet tone's RMS estimate.
    let end = rate as usize / 2 + 11 * 2048;
    let gain = |hz| {
        20.0 * (amplitude(&output[..end], source, hz, 0) / amplitude(&input[..end], source, hz, 0))
            .log10()
    };
    assert!(
        gain(loud) < -4.0 && gain(quiet).abs() < 2.0,
        "{} / {} dB",
        gain(loud),
        gain(quiet)
    );
}

#[test]
fn spectral_lift_and_linked_detection_do_not_generate_output_for_silence() {
    let source = source(48_000, 1);
    let (loud, quiet) = (1_125.0, 1_375.0);
    let input: Vec<_> = (0..source.sample_rate as usize)
        .map(|frame| {
            pcm(0.25 * sample(source.sample_rate, frame, loud)
                + 0.025 * sample(source.sample_rate, frame, quiet))
        })
        .collect();
    let band = spectral_band(EqChannel::Stereo, EqBandKind::Bell, 1200, -12.0);
    let settings = treatment(&[band]);
    let processed = render(&input, source, settings.clone());
    let ratio = |hz| amplitude(&processed, source, hz, 0) / amplitude(&input, source, hz, 0);
    assert!((0.92..1.08).contains(&ratio(loud)), "loud {}", ratio(loud));
    assert!(ratio(quiet) > 1.7, "quiet {}", ratio(quiet));
    let silent = vec![0; 12_000];
    assert_eq!(render(&silent, source, settings), silent);
}

#[test]
fn matrix_eq_isolates_mid_and_side_and_composes_after_source_eq() {
    let source = source(48_000, 2);
    let tone = |frame| pcm(0.045 * sample(source.sample_rate, frame, 1_000.0));
    let stereo = |side| {
        (0..source.sample_rate as usize)
            .flat_map(|frame| [tone(frame), tone(frame) * side])
            .collect::<Vec<_>>()
    };
    let band = |channel| EqBand {
        channel,
        hz: 1000,
        q: 100,
        gain_db: 6.0,
        ..Default::default()
    };
    let mid = stereo(1);
    let side = stereo(-1);
    assert_eq!(
        render(&mid, source, treatment(&[band(EqChannel::Side)])),
        mid
    );
    assert_eq!(
        render(&side, source, treatment(&[band(EqChannel::Mid)])),
        side
    );
    let mid_settings = treatment(&[band(EqChannel::Mid)]);
    let boosted = render(&mid, source, mid_settings.clone());
    assert!((1.98..2.01).contains(&(amplitude(&boosted, source, 1000.0, 0) / 0.045)));
    let left_then_mid = render(
        &mid,
        source,
        treatment(&[band(EqChannel::Left), band(EqChannel::Mid)]),
    );
    let sequential = render(
        &render(&mid, source, treatment(&[band(EqChannel::Left)])),
        source,
        mid_settings.clone(),
    );
    assert!(left_then_mid
        .iter()
        .zip(sequential)
        .all(|(a, b)| a.abs_diff(b) <= 3));
    assert!(mid_settings
        .output_info(super::eq_features_tests::source(48_000, 1))
        .is_err());

    let (low_hz, high_hz) = (800, 1200);
    for (channel, dry) in [(EqChannel::Mid, &mid), (EqChannel::Side, &side)] {
        let mut processor = PreparedTreatment::preview(AudioTreatment::default(), source)
            .unwrap()
            .with_preview_monitor(
                Some(PreviewMonitor::Band {
                    low_hz,
                    high_hz,
                    channel,
                }),
                None,
            )
            .processor(source)
            .unwrap();
        let mut output = Vec::new();
        processor.process(dry, &mut output);
        assert!(output
            .as_chunks::<2>()
            .0
            .iter()
            .all(|frame| frame[0] == frame[1]));
        assert!(amplitude(&output, source, 1000.0, 1) > 0.04);
    }
}

#[test]
fn eq_strength_scales_only_static_bell_and_shelf_gains() {
    let source = source(48_000, 1);
    let band = |kind, hz, gain_db| EqBand {
        kind,
        hz,
        gain_db,
        q: 80,
        adaptive_q: true,
        ..Default::default()
    };
    let dynamic = spectral_band(EqChannel::Stereo, EqBandKind::Bell, 1200, 12.0);
    let settings = AudioTreatment {
        bass_db: 6.0,
        presence_db: -12.0,
        treble_db: 16.0,
        rumble: true,
        rumble_slope: 48,
        eq_output_db: -3.0,
        ..treatment(&[
            band(EqBandKind::Bass, 300, -8.0),
            band(EqBandKind::Bell, 800, 6.0),
            band(EqBandKind::Treble, 4000, 10.0),
            band(EqBandKind::High, 8500, 8.0),
            dynamic,
        ])
    };
    let input: Vec<_> = (0..source.sample_rate as usize)
        .map(|frame| {
            pcm(0.06 * sample(source.sample_rate, frame, 300.0)
                + 0.27 * sample(source.sample_rate, frame, 1_200.0))
        })
        .collect();
    for strength in [0.0, 50.0, 100.0] {
        let scaled = AudioTreatment {
            eq_strength: Some(strength),
            ..settings.clone()
        };
        let mut expected = settings.clone();
        expected.bass_db *= strength / 100.0;
        expected.presence_db *= strength / 100.0;
        expected.treble_db *= strength / 100.0;
        for band in expected.eq_bands.as_mut().unwrap().iter_mut().flatten() {
            if band.dynamic.is_none() {
                band.gain_db *= strength / 100.0;
            }
        }
        assert_eq!(
            render(&input, source, scaled),
            render(&input, source, expected)
        );
    }
    assert!(AudioTreatment {
        eq_strength: Some(101.0),
        ..settings
    }
    .output_info(source)
    .is_err());
    let neutral = AudioTreatment {
        eq_strength: Some(0.0),
        bass_db: 12.0,
        ..Default::default()
    };
    assert!(!neutral.is_active());
    assert_eq!(render(&input, source, neutral), input);
}

#[test]
fn spectral_output_is_invariant_to_blocks_and_source_aligned_seeks() {
    let source = source(48_000, 2);
    let input: Vec<_> = (0..source.sample_rate as usize * 3)
        .flat_map(|frame| {
            let tone = |hz| sample(source.sample_rate, frame, hz);
            [
                pcm(0.29 * tone(1125.0) + 0.07 * tone(3000.0)),
                pcm(-0.25 * tone(1125.0) + 0.04 * tone(3000.0)),
            ]
        })
        .collect();
    let settings = treatment(&[
        spectral_band(EqChannel::Side, EqBandKind::Bell, 1125, 9.0),
        spectral_band(EqChannel::Stereo, EqBandKind::Treble, 2000, -8.0),
    ]);
    let prepared = PreparedTreatment::preview(settings.clone(), source).unwrap();
    let whole = render(&input, source, settings);
    let mut output = Vec::new();
    let mut processor = prepared.processor(source).unwrap();
    let mut chunked = Vec::new();
    for block in input.chunks(766) {
        processor.process(block, &mut output);
        chunked.extend_from_slice(&output);
    }
    assert_eq!(chunked, whole);
    let start = 123;
    let after = 111_373;
    let mut processor = prepared.processor_at(source, start as u64).unwrap();
    processor.process(&input[start * 2..], &mut output);
    let difference = whole[after * 2..]
        .iter()
        .zip(&output[(after - start) * 2..])
        .map(|(a, b)| a.abs_diff(*b))
        .max()
        .unwrap();
    assert!(
        difference < 20_000,
        "aligned seek differed by {difference} PCM counts"
    );
}

#[test]
fn spectral_narrow_low_bell_stays_within_its_dynamic_range() {
    let source = source(48_000, 1);
    let mut band = spectral_band(EqChannel::Stereo, EqBandKind::Bell, 47, 12.0);
    band.q = 1200;
    band.dynamic.as_mut().unwrap().threshold_db = -72.0;
    band.dynamic.as_mut().unwrap().ratio = 20.0;
    for hz in [44.0, 46.875, 54.5, 70.3125] {
        let input: Vec<_> = (0..source.sample_rate as usize)
            .map(|frame| pcm(0.1 * sample(source.sample_rate, frame, hz)))
            .collect();
        let output = render(&input, source, treatment(&[band]));
        let db =
            20.0 * (amplitude(&output, source, hz, 0) / amplitude(&input, source, hz, 0)).log10();
        assert!(db > -12.2, "{hz} Hz turned down {db:.2} dB");
        if hz == 46.875 {
            assert!(db < -2.0, "the selected 47 Hz still needs attenuation");
        }
    }
}

#[test]
fn spectral_preview_probe_and_flac_export_share_the_processed_audio() {
    use crate::{probe_treatment, render_treated_file, FrameRange, OutputFormat, RenderSegment};
    use std::io::{Seek, Write};

    let mut source = source(48_000, 2);
    source.frames *= 4;
    source.bits_per_sample = Some(16);
    let frames = source.frames as u32;
    let length = frames * 4;
    let mut wav = tempfile::tempfile().unwrap();
    wav.write_all(b"RIFF").unwrap();
    wav.write_all(&(36 + length).to_le_bytes()).unwrap();
    wav.write_all(b"WAVEfmt ").unwrap();
    wav.write_all(&[16, 0, 0, 0, 1, 0, 2, 0]).unwrap();
    wav.write_all(&source.sample_rate.to_le_bytes()).unwrap();
    wav.write_all(&(source.sample_rate * 4).to_le_bytes())
        .unwrap();
    wav.write_all(&[4, 0, 16, 0]).unwrap();
    wav.write_all(b"data").unwrap();
    wav.write_all(&length.to_le_bytes()).unwrap();
    let dry: Vec<_> = (0..frames as usize)
        .flat_map(|frame| {
            let tone = |hz| sample(source.sample_rate, frame, hz);
            let mid = 0.27 * tone(1_125.0) + 0.03 * tone(1_375.0);
            let side = 0.07 * tone(4000.0);
            [(mid + side) * 32767.0, (mid - side) * 32767.0]
        })
        .map(|value| value as i16)
        .collect();
    let bytes: Vec<_> = dry.iter().flat_map(|sample| sample.to_le_bytes()).collect();
    wav.write_all(&bytes).unwrap();
    let mut copy = || {
        wav.rewind().unwrap();
        wav.try_clone().unwrap()
    };

    let settings = treatment(&[spectral_band(EqChannel::Mid, EqBandKind::Bell, 1200, 12.0)]);
    let prepared = prepare_treatment(copy(), settings.clone(), || false).unwrap();
    assert_eq!(prepared.gain_db(), 0.0);
    let range = FrameRange {
        start: 148_002,
        end: 166_133,
    };
    let preview = PreparedTreatment::preview(settings, source).unwrap();
    let warmup = preview.preview_start(range.start, source.sample_rate);
    assert!(warmup > 0);
    let mut processor = preview.processor_at(source, warmup).unwrap();
    let input: Vec<_> = dry[(warmup * 2) as usize..(range.end * 2) as usize]
        .iter()
        .map(|&sample| i32::from(sample) << 16)
        .collect();
    let mut audition = Vec::new();
    processor.process(&input, &mut audition);

    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("spectral.flac");
    render_treated_file(
        copy(),
        &[RenderSegment {
            range,
            output: path.clone(),
        }],
        OutputFormat::Flac,
        prepared,
        || false,
        |_| {},
        |output| {
            File::options()
                .read(true)
                .write(true)
                .create_new(true)
                .open(output)
        },
    )
    .unwrap();
    let mut export = Recording::open(File::open(path).unwrap()).unwrap();
    let mut rendered = Vec::new();
    while let Some(block) = export.next(&mut || false).unwrap() {
        rendered.extend_from_slice(block);
    }
    assert_eq!(rendered.len(), ((range.end - range.start) * 2) as usize);
    assert!(audition[((range.start - warmup) * 2) as usize..]
        .iter()
        .zip(rendered)
        .all(|(audible, exported)| audible.abs_diff(exported) < 1024));

    let probe =
        probe_treatment(copy(), source, range.start, None, preview, false, || false).unwrap();
    assert!(probe.spectral_eq && probe.dynamic_eq_mix[0] > 0.4);
    let detector = probe.dynamic_eq_detectors[0].unwrap();
    assert!((-14.0..-10.0).contains(&detector), "detector {detector}");
    let nearest = |hz: f32| {
        probe
            .frequencies
            .iter()
            .enumerate()
            .min_by(|(_, left), (_, right)| (*left - hz).abs().total_cmp(&(*right - hz).abs()))
            .unwrap()
            .0
    };
    let loud = nearest(1125.0);
    let side = nearest(4000.0);
    assert!(probe.treated[loud] < probe.original[loud] - 5.0);
    assert!((probe.treated[side] - probe.original[side]).abs() < 0.1);
}

#[test]
fn spectral_eq_processes_all_band_slots_across_all_stereo_domains() {
    let source = source(96_000, 2);
    let routes = [
        EqChannel::Stereo,
        EqChannel::Left,
        EqChannel::Right,
        EqChannel::Mid,
        EqChannel::Side,
    ];
    let kinds = [EqBandKind::Bass, EqBandKind::Bell, EqBandKind::Treble];
    let bands: Vec<_> = (0..MAX_EQ_BANDS)
        .map(|i| {
            let hz = 700 + i as u16 * 80;
            spectral_band(
                routes[i % routes.len()],
                kinds[i % kinds.len()],
                hz,
                if i % 2 != 0 { -6.0 } else { 6.0 },
            )
        })
        .collect();
    let input: Vec<_> = (0..source.sample_rate as usize)
        .flat_map(|frame| {
            let tone = |hz| sample(source.sample_rate, frame, hz);
            [
                pcm(0.18 * tone(1125.0) + 0.07 * tone(3000.0)),
                pcm(-0.2 * tone(1125.0) + 0.04 * tone(3000.0)),
            ]
        })
        .collect();
    let settings = treatment(&bands);
    let mut processor = PreparedTreatment::preview(settings, source)
        .unwrap()
        .processor(source)
        .unwrap();
    let started = std::time::Instant::now();
    let mut output = Vec::new();
    let readings = processor.process_for_playback(std::hint::black_box(&input), &mut output);
    eprintln!(
        "96 kHz stereo, {MAX_EQ_BANDS} spectral bands across 5 domains, 1 second in {:?}",
        started.elapsed()
    );
    assert_eq!(output.len(), input.len());
    assert_ne!(output, input);
    assert_eq!(readings.mix.len(), MAX_EQ_BANDS);
    assert!(readings.mix.iter().all(|mix| mix.is_finite()));
}

#[test]
fn eq_wire_defaults_keep_existing_drafts_and_lowercase_channels() {
    use serde::de::value::{Error, MapDeserializer, StrDeserializer};

    let old = AudioTreatment::deserialize(MapDeserializer::<_, Error>::new(std::iter::empty::<(
        &str,
        bool,
    )>()))
    .unwrap();
    assert_eq!(old.eq_strength, None);
    assert_eq!(old.eq_weight(), 1.0);
    let dynamic = DynamicEq::deserialize(MapDeserializer::<_, Error>::new(
        [("thresholdDb", -22.0_f32), ("amountDb", 6.0)].into_iter(),
    ))
    .unwrap();
    assert_eq!(dynamic.ratio, 2.0);
    assert!(!dynamic.spectral);

    for (name, channel) in [("mid", EqChannel::Mid), ("side", EqChannel::Side)] {
        assert_eq!(
            EqChannel::deserialize(StrDeserializer::<Error>::new(name)).unwrap(),
            channel
        );
    }
}

#[test]
fn spectral_live_retune_carries_detector_state_and_keeps_the_source_grid() {
    let source = source(48_000, 1);
    let rate = source.sample_rate;
    let frames = rate as usize;
    let original = spectral_band(EqChannel::Stereo, EqBandKind::Bell, 1125, 12.0);
    let mut next = original;
    next.dynamic.as_mut().unwrap().threshold_db = -33.0;
    let initial = PreparedTreatment::preview(treatment(&[original]), source).unwrap();
    let target = PreparedTreatment::preview(treatment(&[next]), source).unwrap();
    assert!(initial.can_retune_eq(&target, rate));
    next.dynamic.as_mut().unwrap().spectral = false;
    let envelope_target = PreparedTreatment::preview(treatment(&[next]), source).unwrap();
    assert!(!initial.can_retune_eq(&envelope_target, rate));

    let input: Vec<_> = (0..frames)
        .map(|frame| {
            let amplitude = if frame < 10_000 { 0.30 } else { 0.06 };
            pcm(amplitude * sample(rate, frame, 1125.0))
        })
        .collect();
    let edit = 21_004;
    let history_start = edit - frames / 4;
    let mut active = initial.processor(source).unwrap();
    let mut restarted = target.processor_at(source, history_start as u64).unwrap();
    let mut uninterrupted = target.processor(source).unwrap();
    let mut scratch = Vec::new();
    let mut expected = Vec::new();
    active.process(&input[..edit], &mut scratch);
    restarted.process(&input[history_start..edit], &mut scratch);
    restarted.carry_eq_state(&active);
    restarted.process(&input[edit..], &mut scratch);
    uninterrupted.process(&input, &mut expected);
    // Repeat the retained-history handoff at the worker's absolute positions.
    // Two FFT hops replace both interpolated kernels with the new settings.
    let overlap = 512;
    assert_eq!(scratch[overlap..], expected[edit + overlap..]);
}

#[test]
fn eq_input_drives_band_detection_independently_of_output_gain() {
    let source = source(48_000, 1);
    let input: Vec<_> = (0..source.frames as usize)
        .map(|frame| pcm(0.055 * sample(source.sample_rate, frame, 1_000.0)))
        .collect();
    let settings = treatment(&[spectral_band(
        EqChannel::Stereo,
        EqBandKind::Bell,
        1_000,
        -8.0,
    )]);
    let before_eq = AudioTreatment {
        eq_input_db: 6.0,
        ..settings.clone()
    };
    let after_eq = AudioTreatment {
        eq_output_db: 6.0,
        ..settings.clone()
    };
    let level = |settings| amplitude(&render(&input, source, settings), source, 1_000.0, 0);
    assert!(level(before_eq) < level(after_eq.clone()) * 0.85);
    assert!((level(after_eq) / level(settings) - db_gain(6.0)).abs() < 0.01);
}

#[test]
fn playback_reports_separate_spectral_cuts_for_two_loud_tones() {
    let source = source(48_000, 1);
    let tones = [819.0, 1289.0];
    let input: Vec<_> = (0..source.frames as usize)
        .map(|frame| {
            pcm(tones
                .iter()
                .map(|&hz| 0.2 * sample(source.sample_rate, frame, hz))
                .sum())
        })
        .collect();
    let settings = treatment(&[spectral_band(
        EqChannel::Stereo,
        EqBandKind::Bell,
        1_000,
        8.0,
    )]);
    let mut processor = PreparedTreatment::preview(settings, source)
        .unwrap()
        .processor(source)
        .unwrap();
    let readings = processor.process_for_playback(&input, &mut Vec::new());
    let [response] = readings.spectral_response.as_slice() else {
        panic!("missing spectral response")
    };
    let gain = |hz: f32| {
        let bin = (hz / response.bin_hz).round() as usize;
        bin.checked_sub(response.first_bin)
            .and_then(|index| response.gains_db.get(index))
            .copied()
            .unwrap_or(0.0)
    };
    assert!(gain(819.0) < -2.0 && gain(1289.0) < -2.0);
    assert!(gain(1054.0).abs() < 0.1);
}
