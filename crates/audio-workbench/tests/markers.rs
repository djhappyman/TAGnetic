use tagrex_audio_workbench::markers::{
    parse_cue, provider_markers, validate_ranges, write_cue, CueEncoding, CueExportTrack, CueTags,
    FrameRange, MarkerError, ProviderAnchor, ProviderTrack, ProviderTrackKind,
};

const CUE: &str = "REM GENRE \"Dance\"\r\nTITLE \"Édition\"\r\nPERFORMER \"Ensemble\"\r\nFILE \"master rip.flac\" WAVE\r\n  TRACK 01 AUDIO\r\n    TITLE \"Début\"\r\n    INDEX 01 00:00:00\r\n  TRACK 02 AUDIO\r\n    TITLE \"Suite\"\r\n    INDEX 00 00:00:30\r\n    INDEX 01 00:00:38\r\n    POSTGAP 00:00:04\r\n";

fn ranges(start: u64, end: u64) -> FrameRange {
    FrameRange { start, end }
}

#[test]
fn cue_roundtrip_retains_absolute_track_positions_and_metadata() {
    let sheet = parse_cue(CUE.as_bytes(), CueEncoding::Utf8).expect("supported CUE");
    assert_eq!(sheet.source_name, "master rip.flac");
    assert_eq!(sheet.tags.title.as_deref(), Some("Édition"));
    assert_eq!(sheet.tags.genre.as_deref(), Some("Dance"));
    assert_eq!(sheet.tags.performer.as_deref(), Some("Ensemble"));
    assert_eq!(sheet.tracks[1].tags.title.as_deref(), Some("Suite"));
    assert_eq!(sheet.tracks[1].postgap, Some(4));
    assert!(sheet
        .warnings
        .iter()
        .any(|warning| warning.contains("silence")));
    let sample_rate = 44_100;
    let total_frames = 48_000;
    let markers = sheet
        .markers(sample_rate, total_frames)
        .expect("bound to master");
    assert_eq!(markers[1].frame, 38 * 588);
    assert_eq!(markers[1].pregap_frame, Some(30 * 588));

    let tracks = markers
        .iter()
        .enumerate()
        .map(|(index, marker)| CueExportTrack {
            range: ranges(
                marker.frame,
                markers.get(index + 1).map_or(total_frames, |m| m.frame),
            ),
            tags: marker.tags.clone(),
            pregap_frame: marker.pregap_frame,
            index01: Some(marker.index01),
            index00: marker.index00,
            pregap: marker.pregap,
            postgap: marker.postgap,
        })
        .collect::<Vec<_>>();
    let export = write_cue(
        &sheet.source_name,
        sample_rate,
        total_frames,
        &sheet.tags,
        &tracks,
    )
    .expect("whole master");
    assert_eq!(
        parse_cue(export.text.as_bytes(), CueEncoding::Utf8).expect("written CUE"),
        sheet,
    );
}

#[test]
fn cue_decodes_boms_and_requires_an_explicit_legacy_choice() {
    let mut bom = vec![0xff, 0xfe];
    for unit in CUE.encode_utf16() {
        bom.extend(unit.to_le_bytes());
    }
    let original = parse_cue(CUE.as_bytes(), CueEncoding::Utf8).expect("UTF-8");
    assert_eq!(
        parse_cue(&bom, CueEncoding::Utf8).expect("UTF-16 with BOM"),
        original
    );
    let legacy = b"TITLE \"Caf\xe9\"\nFILE \"long.mp3\" MP3\nTRACK 01 AUDIO\nINDEX 01 00:00:00\n";
    assert_eq!(
        parse_cue(legacy, CueEncoding::Utf8),
        Err(MarkerError::CueEncoding)
    );
    let legacy = parse_cue(legacy, CueEncoding::Windows1252).expect("legacy selected");
    assert_eq!(legacy.tags.title.as_deref(), Some("Café"));
}

#[test]
fn cue_rejects_an_unsafe_or_ambiguous_structure() {
    for malformed in [
        "FILE \"a.wav\" WAVE\nFILE \"b.wav\" WAVE\nTRACK 01 AUDIO\nINDEX 01 00:00:00",
        "FILE \"a.wav\" WAVE\nTRACK 01 MODE1/2352\nINDEX 01 00:00:00",
        "FILE \"a.wav\" WAVE\nTRACK 01 AUDIO\nINDEX 01 00:60:00",
        "FILE \"a.wav\" WAVE\nTRACK 01 AUDIO\nINDEX 01 00:00:75",
        "FILE \"a.wav\" WAVE\nTRACK 01 AUDIO\nINDEX 01 99999999999999999:00:00",
        "FILE \"a.wav\" WAVE\nTRACK 01 AUDIO\nINDEX 01 00:00:00\nINDEX 00 00:00:00",
        "FILE \"a.wav\" WAVE\nTRACK 01 AUDIO\nINDEX 01 00:00:04\nTRACK 02 AUDIO\nINDEX 01 00:00:02",
        "FILE \"a.wav\" WAVE\nTRACK 01 AUDIO\nINDEX 00 00:00:00",
    ] {
        assert!(
            parse_cue(malformed.as_bytes(), CueEncoding::Utf8).is_err(),
            "{malformed}"
        );
    }
    let sheet = parse_cue(CUE.as_bytes(), CueEncoding::Utf8).expect("sheet reads");
    assert_eq!(
        sheet.markers(44_100, 38 * 588),
        Err(MarkerError::CueBoundary { track: 2 })
    );
    let external = CUE.replace("master rip.flac", "../../other/master.flac");
    assert_eq!(
        parse_cue(external.as_bytes(), CueEncoding::Utf8)
            .expect("path is text")
            .source_name,
        "../../other/master.flac",
    );
}

#[test]
fn cue_rounding_uses_absolute_positions_and_refuses_reviewed_exclusions() {
    fn export_track(start: u64, end: u64) -> CueExportTrack {
        CueExportTrack {
            range: ranges(start, end),
            ..CueExportTrack::default()
        }
    }
    let tags = CueTags::default();
    let trimmed = write_cue(
        "master.mp3",
        44_100,
        10_000,
        &tags,
        &[export_track(5_880, 10_000)],
    )
    .expect("a nonzero first INDEX 01 leaves the opening unassigned");
    assert!(trimmed
        .text
        .contains("TRACK 01 AUDIO\n    INDEX 01 00:00:10"));
    assert_eq!(
        write_cue(
            "master.mp3",
            44_100,
            10_000,
            &tags,
            &[export_track(0, 6_000), export_track(6_010, 10_000)]
        ),
        Err(MarkerError::CueExclusions)
    );
    assert_eq!(
        write_cue(
            "master.mp3",
            44_100,
            10_000,
            &tags,
            &[export_track(0, 1), export_track(1, 10_000)]
        ),
        Err(MarkerError::CueRounding { track: 2 })
    );
    for sample_rate in [44_100, 48_000, 96_000] {
        let original = u64::from(sample_rate) * 60 + u64::from(sample_rate) / 75 / 2;
        let total = u64::from(sample_rate) * 61;
        let export = write_cue(
            "master.mp3",
            sample_rate,
            total,
            &tags,
            &[export_track(0, original), export_track(original, total)],
        )
        .expect("second boundary fits");
        assert!(export.text.contains("FILE \"master.mp3\" MP3"));
        assert!(export.text.contains("INDEX 01 01:00:01"));
        assert!(export
            .warnings
            .iter()
            .any(|warning| warning.contains("rounded")));
    }
    let injected = CueTags {
        title: Some("First\nFILE \"other.wav\" WAVE".to_owned()),
        ..CueTags::default()
    };
    assert_eq!(
        write_cue(
            "master.mp3",
            44_100,
            10_000,
            &injected,
            &[export_track(0, 10_000)]
        ),
        Err(MarkerError::CueMetadata)
    );
}

#[test]
fn generic_validation_allows_gaps_and_refuses_bad_ranges() {
    assert_eq!(
        validate_ranges(&[ranges(0, 4), ranges(5, u64::MAX)], u64::MAX),
        Ok(())
    );
    assert_eq!(
        validate_ranges(&[ranges(0, 6), ranges(5, 8)], 8),
        Err(MarkerError::OverlappingRange { track: 2 })
    );
    assert_eq!(
        validate_ranges(&[ranges(8, 8)], 10),
        Err(MarkerError::EmptyRange { track: 1 })
    );
    assert_eq!(
        validate_ranges(&[ranges(0, 11)], 10),
        Err(MarkerError::RangeOutsideSource { track: 1 })
    );
}

fn provider_track(position: &str, duration: &str) -> ProviderTrack {
    ProviderTrack {
        position: position.to_owned(),
        title: format!("Track {position}"),
        duration: duration.to_owned(),
        ..ProviderTrack::default()
    }
}

fn anchor(start_index: usize, end_index: usize, frame: u64) -> ProviderAnchor {
    ProviderAnchor {
        start_index,
        end_index,
        frame,
    }
}

#[test]
fn provider_projection_ignores_headings_and_aggregate_lengths() {
    let tracks = [
        ProviderTrack {
            kind: ProviderTrackKind::Heading,
            title: "Side A".to_owned(),
            ..ProviderTrack::default()
        },
        ProviderTrack {
            kind: ProviderTrackKind::Index,
            title: "Medley".to_owned(),
            duration: "1:30".to_owned(),
            performer: "Band".to_owned(),
            children: vec![provider_track("A1", "0:45"), provider_track("A2", "0.45")],
            ..ProviderTrack::default()
        },
        provider_track("A3", "0:30"),
        provider_track("B1", "3:05"),
    ];
    let sample_rate = 48_000;
    let sec = u64::from(sample_rate);
    let view = provider_markers(&tracks, &[], sample_rate, sec * 600).expect("candidates");
    assert_eq!(
        view.tracks
            .iter()
            .map(|track| track.position.as_str())
            .collect::<Vec<_>>(),
        ["A1", "A2", "A3", "B1"]
    );
    assert_eq!(view.tracks[0].performer, "Band");
    assert_eq!(view.tracks[0].heading.as_deref(), Some("Medley"));
    assert_eq!(view.tracks[2].heading.as_deref(), Some("Side A"));

    let projected = provider_markers(
        &tracks,
        &[anchor(0, 3, sec * 4), anchor(3, 4, sec * 130)],
        sample_rate,
        sec * 600,
    )
    .expect("independent side anchors");
    assert_eq!(
        projected
            .tracks
            .iter()
            .map(|track| track.frame)
            .collect::<Vec<_>>(),
        [
            Some(sec * 4),
            Some(sec * 49),
            Some(sec * 94),
            Some(sec * 130)
        ]
    );
    assert_eq!(projected.spans[0].expected_end_frame, Some(sec * 124));
    assert_eq!(projected.spans[0].available_end_frame, sec * 130);
    assert!(projected
        .warnings
        .iter()
        .any(|warning| warning.contains("group duration")));
}

#[test]
fn provider_unknown_durations_and_order_require_new_anchors() {
    let tracks = [
        provider_track("A1", "0:40"),
        provider_track("A2", "1:60"),
        provider_track("A3", "0:30"),
        provider_track("A1", "0:30"),
        provider_track("B1", "1:02:03"),
    ];
    let sec = 44_100_u64;
    let projected = provider_markers(
        &tracks,
        &[anchor(0, 4, 0), anchor(4, 5, sec * 120)],
        sec as u32,
        sec * 500,
    )
    .expect("independent side B");
    assert_eq!(
        projected
            .tracks
            .iter()
            .map(|track| track.frame)
            .collect::<Vec<_>>(),
        [Some(0), Some(sec * 40), None, None, Some(sec * 120)]
    );
    assert_eq!(projected.tracks[1].duration_seconds, None);
    assert_eq!(projected.tracks[4].duration_seconds, Some(3723));
    assert_eq!(projected.spans[0].expected_end_frame, None);
    assert!(projected
        .warnings
        .iter()
        .any(|warning| warning.contains("out of order")));
    assert!(projected
        .warnings
        .iter()
        .any(|warning| warning.contains("extend beyond")));
    assert_eq!(
        provider_markers(
            &tracks,
            &[anchor(0, 3, 0), anchor(2, 4, sec)],
            sec as u32,
            sec * 500
        ),
        Err(MarkerError::ProviderAnchor { anchor: 2 })
    );
}

#[test]
fn provider_does_not_assume_the_start_of_the_next_side() {
    let tracks = [provider_track("1A", "1:00"), provider_track("1B", "1:00")];
    let proposal = provider_markers(&tracks, &[anchor(0, 2, 512)], 8_000, 1_000_000)
        .expect("a draft with a diagnostic");
    assert_eq!(proposal.tracks[0].frame, Some(512));
    assert_eq!(proposal.tracks[1].frame, None);
    assert!(proposal
        .warnings
        .iter()
        .any(|warning| warning.contains("side or disc")));
}
