// Included in the command tests to reuse disposable audio fixtures.
#[test]
#[ignore = "manual independent verification; requires ffmpeg, ffprobe and Python mutagen"]
fn native_batch_independent_audio_roundtrip() {
    use std::process::Command;
    fn output(program: &str, args: &[&str]) -> Vec<u8> {
        let result = Command::new(program).args(args).output().unwrap();
        assert!(
            result.status.success(),
            "{program}: {}",
            String::from_utf8_lossy(&result.stderr)
        );
        result.stdout
    }
    fn packets(path: &str) -> Vec<u8> {
        output(
            "ffmpeg",
            &[
                "-v", "error", "-i", path, "-map", "0:a:0", "-c", "copy", "-f", "hash", "-hash",
                "sha256", "-",
            ],
        )
    }
    fn tags(path: &str) -> serde_json::Value {
        let read: serde_json::Value = serde_json::from_slice(&output(
            "ffprobe",
            &[
                "-v",
                "error",
                "-show_entries",
                "format_tags",
                "-of",
                "json",
                path,
            ],
        ))
        .unwrap();
        serde_json::Value::Object(
            read["format"]["tags"]
                .as_object()
                .unwrap()
                .iter()
                .map(|(key, value)| (key.to_ascii_lowercase(), value.clone()))
                .collect(),
        )
    }
    let dir = TempDir::new("native-batch-independent");
    let mut app = App::open(&dir.0, &dir.0.join("journal.sqlite")).unwrap();
    for (ext, codec) in [("mp3", "libmp3lame"), ("flac", "flac"), ("m4a", "aac")] {
        let path = dir.0.join(format!("fixture.{ext}"));
        let name = path.to_str().unwrap();
        output(
            "ffmpeg",
            &[
                "-v",
                "error",
                "-f",
                "lavfi",
                "-i",
                "sine=frequency=440:duration=0.1",
                "-c:a",
                codec,
                "-metadata",
                "artist=Original",
                "-metadata",
                "title=Original title",
                "-metadata",
                "album=Saved album",
                "-y",
                name,
            ],
        );
        let before = tags(name);
        let audio = packets(name);
        let bytes = std::fs::read(&path).unwrap();
        let recipe = [
            FieldOperation::Extract {
                source: "Björk / 日本語 / Björk - New title".into(),
                pattern: "%artist% - %title%".into(),
            },
            FieldOperation::SplitValues {
                field: "artist".into(),
                separator: " / ".into(),
            },
            FieldOperation::DeduplicateValues {
                field: "artist".into(),
            },
        ];
        let plan = app
            .preview_field_operations(std::slice::from_ref(&path), &recipe, None)
            .unwrap();
        let plan = app
            .preview_text_import(
                std::slice::from_ref(&path),
                &format!("fixture.{ext}|Imported album"),
                &TextImportOptions {
                    pattern: "%_filename_ext%|%album%".into(),
                    mode: TextImportMode::Filename,
                },
                Some(&plan),
            )
            .unwrap()
            .plan;
        std::fs::write(dir.0.join("fixture.txt"), "日本語\nRecording notes\n").unwrap();
        let plan = app
            .preview_text_sidecars(
                std::slice::from_ref(&path),
                &TextSidecarOptions {
                    field: "comment".into(),
                    filename: "%filename%.txt".into(),
                    skip_missing: false,
                },
                Some(&plan),
            )
            .unwrap();
        let plan = app
            .preview_numbering(
                std::slice::from_ref(&path),
                &NumberingOptions {
                    start: 3,
                    disc: Some(2),
                    write_total: true,
                    write_disc_total: true,
                    ..Default::default()
                },
                Some(&plan),
            )
            .unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), bytes, "preview wrote {ext}");
        let batch = app.apply(&plan).unwrap();
        let saved = tags(name);
        assert_eq!(saved["title"], "New title", "{saved}");
        assert_eq!(saved["album"], "Imported album", "{saved}");
        assert_eq!(saved["comment"], "日本語\nRecording notes\n", "{saved}");
        // Independent physical-value readback, beyond the joined table string.
        let artists=output("python3",&["-c","import json,sys,mutagen; f=mutagen.File(sys.argv[1]); t=f.tags; a=t.get('artist') if sys.argv[1].endswith('.flac') else (t.get('\\u00a9ART') if sys.argv[1].endswith('.m4a') else t.getall('TPE1')[0].text); print(json.dumps(list(a),ensure_ascii=False))",name]);
        let artists: Vec<String> = serde_json::from_slice(&artists).unwrap();
        assert_eq!(
            artists,
            vec!["Björk", "日本語"],
            "physical list values differ in {ext}"
        );
        assert_eq!(packets(name), audio, "encoded audio changed in {ext}");
        app.undo(batch.id).unwrap();
        assert_eq!(
            tags(name),
            before,
            "independent metadata differs after Undo in {ext}"
        );
        assert_eq!(packets(name), audio);
        eprintln!("Native batch {ext}: preview unchanged, extraction/list/import/numbering Apply verified, physical artists verified, packets unchanged, Undo restored");
    }
}

#[test]
#[ignore = "manual performance measurement; creates 10000 disposable tiny files"]
fn native_batch_benchmark() {
    let dir = TempDir::new("native-batch-benchmark");
    let source = dir.tagged_flac("source.flac", "Artist", "Song");
    let app = App::open(&dir.0, &dir.0.join("journal.sqlite")).unwrap();
    let paths = (0..10000)
        .map(|i| {
            let path = dir.0.join(format!("track-{i}.flac"));
            std::fs::copy(&source, &path).unwrap();
            path
        })
        .collect::<Vec<_>>();
    for count in [100, 1000, 10000] {
        let text = (0..count)
            .rev()
            .map(|i| format!("track-{i}.flac|Title {i}"))
            .collect::<Vec<_>>()
            .join("\n");
        let mut samples = Vec::new();
        for _ in 0..6 {
            let start = Instant::now();
            let imported = app
                .preview_text_import(
                    &paths[..count],
                    &text,
                    &TextImportOptions {
                        pattern: "%_filename_ext%|%title%".into(),
                        mode: TextImportMode::Filename,
                    },
                    None,
                )
                .unwrap();
            let plan = app
                .preview_numbering(
                    &paths[..count],
                    &NumberingOptions {
                        write_total: true,
                        ..Default::default()
                    },
                    Some(&imported.plan),
                )
                .unwrap();
            assert_eq!(plan.changes.len(), count);
            assert_eq!(batch_value(&plan, &paths[0], "title"), Some("Title 0"));
            samples.push(start.elapsed());
        }
        let first = samples.remove(0);
        samples.sort();
        eprintln!(
            "Native import+number {count}: first {first:?}, warm median {:?}, warm max {:?}",
            samples[2], samples[4]
        );
    }
}
fn batch_value<'a>(plan: &'a PlanDto, path: &Path, field: &str) -> Option<&'a str> {
    plan.changes
        .iter()
        .find(|row| Path::new(&row.path) == path)
        .and_then(|row| row.tag_changes.iter().find(|edit| edit.field == field))
        .and_then(|edit| edit.new.as_deref())
}

#[test]
fn native_extract_reads_staging_before_overlapping_destinations_and_undoes() {
    let dir = TempDir::new("native-extract");
    let path = dir.tagged_flac("one.flac", "Saved artist", "Saved title");
    let other = dir.tagged_flac("two.flac", "Other", "No separator");
    let mut app = App::open(&dir.0, &dir.0.join("journal.sqlite")).unwrap();
    let before = std::fs::read(&path).unwrap();
    let working = app
        .preview_tag_edits(&[TagEditDto {
            path: path.to_string_lossy().into(),
            field: "title".into(),
            value: Some("Björk - 日本語".into()),
        }])
        .unwrap();
    let recipe = [
        FieldOperation::Extract {
            source: "%title%".into(),
            pattern: "%artist% - %title%".into(),
        },
        FieldOperation::Format {
            field: "comment".into(),
            pattern: "%artist%: %title%".into(),
        },
    ];
    let plan = app
        .preview_field_operations(&[path.clone(), other.clone()], &recipe, Some(&working))
        .unwrap();
    assert_eq!(batch_value(&plan, &path, "artist"), Some("Björk"));
    assert_eq!(batch_value(&plan, &path, "title"), Some("日本語"));
    assert_eq!(batch_value(&plan, &path, "comment"), Some("Björk: 日本語"));
    assert_eq!(plan.unrendered[0].paths, vec![other.to_string_lossy()]);
    assert_eq!(std::fs::read(&path).unwrap(), before);
    let batch = app.apply(&plan).unwrap();
    assert_eq!(
        TagEngine::read(&path).unwrap().tags[&TagField::Title],
        "日本語"
    );
    app.undo(batch.id).unwrap();
    assert_eq!(
        TagEngine::read(&path).unwrap().tags[&TagField::Title],
        "Saved title"
    );
}

#[test]
fn native_extract_preflights_and_locked_results_do_not_feed_later_steps() {
    let dir = TempDir::new("native-extract-lock");
    let path = dir.tagged_flac("one.flac", "Saved", "Björk - Track");
    let app = App::open(&dir.0, &dir.0.join("journal.sqlite")).unwrap();
    app.set_locked_fields(&["artist".into()]);
    let groups = [field_group(vec![
        field_rule("extract", "tags", "%title%", "%artist% - %title%"),
        field_rule("format", "comment", "", "%artist%"),
    ])];
    let plan = app
        .preview_field_groups(std::slice::from_ref(&path), &groups, None)
        .unwrap();
    assert_eq!(batch_value(&plan, &path, "comment"), Some("Saved"));
    assert_eq!(plan.locked_skipped[0].field, "artist");
    for pattern in [
        "%artist%%title%",
        "%_path%",
        "%skip%",
        "$upper(%title%)",
        "%title% - %title%",
        "%custom:Venue% / %custom:VENUE%",
    ] {
        assert!(
            app.preview_field_operations(
                &[],
                &[FieldOperation::Extract {
                    source: "%title%".into(),
                    pattern: pattern.into()
                }],
                None
            )
            .is_err(),
            "{pattern}"
        );
    }
}

#[test]
fn native_text_import_uses_tagnetic_path_placeholders_and_rejects_repeated_destinations() {
    let dir = TempDir::new("native-import-placeholders");
    let path = dir.tagged_flac("one.flac", "Artist", "Title");
    let app = App::open(&dir.0, &dir.0.join("journal.sqlite")).unwrap();
    for (pattern, text, mode) in [
        (
            "%filenameext%|%title%",
            "one.flac|New".to_string(),
            TextImportMode::Filename,
        ),
        (
            "%filepath%|%title%",
            format!("{}|New", path.display()),
            TextImportMode::Path,
        ),
    ] {
        let result = app
            .preview_text_import(
                std::slice::from_ref(&path),
                &text,
                &TextImportOptions {
                    pattern: pattern.into(),
                    mode,
                },
                None,
            )
            .unwrap();
        assert_eq!(batch_value(&result.plan, &path, "title"), Some("New"));
    }
    for pattern in [
        "%filenameext%",
        "%filenameext%|%title%|%title%",
        "%filenameext%|%_filename_ext%|%title%",
    ] {
        assert!(app
            .preview_text_import(
                &[],
                "",
                &TextImportOptions {
                    pattern: pattern.into(),
                    mode: TextImportMode::Filename
                },
                None
            )
            .is_err());
    }
}

#[test]
fn native_list_merge_refuses_reinterpretation_and_accepts_an_empty_joiner() {
    let dir = TempDir::new("native-list-merge");
    let path = dir.tagged_flac("one.flac", "A; B", "Title");
    let app = App::open(&dir.0, &dir.0.join("journal.sqlite")).unwrap();
    assert!(app
        .preview_field_operations(
            std::slice::from_ref(&path),
            &[FieldOperation::MergeValues {
                field: "artist".into(),
                separator: "; ".into()
            }],
            None
        )
        .is_err());
    let plan = app
        .preview_field_operations(
            std::slice::from_ref(&path),
            &[FieldOperation::MergeValues {
                field: "artist".into(),
                separator: String::new(),
            }],
            None,
        )
        .unwrap();
    assert_eq!(batch_value(&plan, &path, "artist"), Some("AB"));
}

#[cfg(unix)]
#[test]
fn native_batches_refuse_paths_that_cannot_survive_the_plan_string_boundary() {
    use std::os::unix::ffi::OsStringExt;
    let dir = TempDir::new("native-path-codec");
    let app = App::open(&dir.0, &dir.0.join("journal.sqlite")).unwrap();
    let path = PathBuf::from(std::ffi::OsString::from_vec(vec![b'/', 0xff]));
    assert!(matches!(
        app.preview_numbering(std::slice::from_ref(&path), &Default::default(), None),
        Err(AppError::Mask(MaskError::BadArgument(_)))
    ));
    assert!(matches!(
        app.preview_field_operations(&[path], &[], None),
        Err(AppError::Mask(MaskError::BadArgument(_)))
    ));
}

#[test]
fn native_text_import_matches_by_filename_not_row_order_and_preserves_staging() {
    let dir = TempDir::new("native-matched-text");
    let one = dir.tagged_flac("日本語.flac", "Old 1", "One");
    let two = dir.tagged_flac("two.flac", "Old 2", "Two");
    let app = App::open(&dir.0, &dir.0.join("journal.sqlite")).unwrap();
    let mut working = app
        .preview_tag_edits(&[TagEditDto {
            path: one.to_string_lossy().into(),
            field: "comment".into(),
            value: Some("Retain me".into()),
        }])
        .unwrap();
    working.changes[0].rename_to = Some(dir.0.join("renamed.flac").to_string_lossy().into());
    let result = app
        .preview_text_import(
            &[one.clone(), two.clone()],
            "Heading\n\ntwo.flac|New 2|Second\n日本語.flac|Björk|First\nunknown.flac|Other|Other",
            &TextImportOptions {
                pattern: "%_FILENAME_EXT%|%artist%|%title%".into(),
                mode: TextImportMode::Filename,
            },
            Some(&working),
        )
        .unwrap();
    assert_eq!(batch_value(&result.plan, &one, "title"), Some("First"));
    assert_eq!(batch_value(&result.plan, &two, "title"), Some("Second"));
    assert_eq!(
        batch_value(&result.plan, &one, "comment"),
        Some("Retain me")
    );
    assert!(result
        .plan
        .changes
        .iter()
        .any(|row| row.rename_to.is_some()));
    assert_eq!(result.rows[1].line, 3);
    assert_eq!(result.rows[0].status, TextImportStatus::PatternMismatch);
    assert_eq!(result.rows[3].status, TextImportStatus::FileNotSelected);
    assert!(result.unmatched_files.is_empty());
}

#[test]
fn native_text_import_refuses_ambiguous_keys_and_duplicate_assignments() {
    let dir = TempDir::new("native-text-ambiguity");
    let one = dir.tagged_flac_at("a/one.flac", "A", "One");
    let two = dir.tagged_flac_at("b/one.flac", "B", "Two");
    let app = App::open(&dir.0, &dir.0.join("journal.sqlite")).unwrap();
    let filename = TextImportOptions {
        pattern: "%_filename_ext%|%title%".into(),
        mode: TextImportMode::Filename,
    };
    assert!(app
        .preview_text_import(&[one.clone(), two.clone()], "one.flac|New", &filename, None)
        .is_err());
    assert!(app
        .preview_text_import(
            std::slice::from_ref(&one),
            "one.flac|New\none.flac|Again",
            &filename,
            None
        )
        .is_err());
    let result = app
        .preview_text_import(
            &[one.clone(), two.clone()],
            &format!("{}|New two\n{}|New one", two.display(), one.display()),
            &TextImportOptions {
                pattern: "%_path%|%title%".into(),
                mode: TextImportMode::Path,
            },
            None,
        )
        .unwrap();
    assert_eq!(batch_value(&result.plan, &one, "title"), Some("New one"));
    assert_eq!(batch_value(&result.plan, &two, "title"), Some("New two"));
}

#[test]
fn native_text_import_reports_unpaired_rows_and_files_and_checks_empty_batches() {
    let dir = TempDir::new("native-text-report");
    let one = dir.tagged_flac("one.flac", "A", "One");
    let app = App::open(&dir.0, &dir.0.join("journal.sqlite")).unwrap();
    let options = TextImportOptions {
        pattern: "%track% - %title%".into(),
        mode: TextImportMode::Sequential,
    };
    let result = app
        .preview_text_import(
            &[one.clone(), one.clone()],
            "Heading\n1 - New\n2 - Extra",
            &options,
            None,
        )
        .unwrap();
    assert_eq!(result.rows[2].status, TextImportStatus::NoRemainingFile);
    assert_eq!(result.plan.changes.len(), 1);
    assert!(app
        .preview_text_import(
            &[],
            "",
            &TextImportOptions {
                pattern: "%title%".into(),
                mode: TextImportMode::Filename
            },
            None
        )
        .is_err());
    assert!(app
        .preview_text_import(
            &[],
            "",
            &TextImportOptions {
                pattern: "%artist%%title%".into(),
                mode: TextImportMode::Sequential
            },
            None
        )
        .is_err());
    assert!(app
        .preview_text_import(&[], &"x".repeat(1024 * 1024 + 1), &options, None)
        .is_err());
    assert!(app
        .preview_text_import_with_cancel(&[], "", &options, None, || true)
        .is_err());
    let result = app
        .preview_text_import(std::slice::from_ref(&one), "Heading", &options, None)
        .unwrap();
    assert_eq!(result.unmatched_files, vec![one.to_string_lossy()]);
}

#[test]
fn native_numbering_handles_disc_boundaries_release_totals_and_staged_groups() {
    let dir = TempDir::new("native-discs");
    let paths: Vec<_> = [
        "r1/d1/one.flac",
        "r1/d1/two.flac",
        "r1/d2/three.flac",
        "r2/d1/four.flac",
    ]
    .iter()
    .map(|path| dir.tagged_flac_at(path, "Artist", "Title"))
    .collect();
    let mut app = App::open(&dir.0, &dir.0.join("journal.sqlite")).unwrap();
    let working = app
        .preview_tag_edits(
            &paths
                .iter()
                .enumerate()
                .map(|(i, path)| TagEditDto {
                    path: path.to_string_lossy().into(),
                    field: "album".into(),
                    value: Some(if i < 3 { "Release 1" } else { "Release 2" }.into()),
                })
                .collect::<Vec<_>>(),
        )
        .unwrap();
    let options = NumberingOptions {
        disc: Some(1),
        width: 2,
        disc_width: 2,
        write_total: true,
        write_disc_total: true,
        next_disc_on: Some(NumberBoundary::Folder),
        reset_disc_on: Some(NumberBoundary::Field {
            field: "album".into(),
        }),
        ..Default::default()
    };
    let plan = app
        .preview_numbering(&paths, &options, Some(&working))
        .unwrap();
    for (i, (track, total, disc, discs)) in [
        ("01", "2", "01", "2"),
        ("02", "2", "01", "2"),
        ("01", "1", "02", "2"),
        ("01", "1", "01", "1"),
    ]
    .iter()
    .enumerate()
    {
        assert_eq!(batch_value(&plan, &paths[i], "track"), Some(*track));
        assert_eq!(batch_value(&plan, &paths[i], "tracktotal"), Some(*total));
        assert_eq!(batch_value(&plan, &paths[i], "disc"), Some(*disc));
        assert_eq!(batch_value(&plan, &paths[i], "disctotal"), Some(*discs));
    }
    let batch = app.apply(&plan).unwrap();
    assert_eq!(
        TagEngine::read(&paths[2]).unwrap().tags[&TagField::DiscNumber],
        "02"
    );
    app.undo(batch.id).unwrap();
    assert!(!TagEngine::read(&paths[2])
        .unwrap()
        .tags
        .contains_key(&TagField::DiscNumber));
}

#[test]
fn native_numbering_preserves_order_skips_vinyl_without_consuming_numbers_and_locks() {
    let dir = TempDir::new("native-vinyl-numbers");
    let one = dir.tagged_flac("one.flac", "Artist", "One");
    let two = dir.tagged_flac("two.flac", "Artist", "Two");
    let three = dir.tagged_flac("three.flac", "Artist", "Three");
    let app = App::open(&dir.0, &dir.0.join("journal.sqlite")).unwrap();
    let working = app
        .preview_tag_edits(&[TagEditDto {
            path: two.to_string_lossy().into(),
            field: "track".into(),
            value: Some("A1".into()),
        }])
        .unwrap();
    let options = NumberingOptions {
        start: 5,
        step: 2,
        write_total: true,
        ..Default::default()
    };
    let plan = app
        .preview_numbering(
            &[three.clone(), two.clone(), one.clone(), three.clone()],
            &options,
            Some(&working),
        )
        .unwrap();
    assert_eq!(batch_value(&plan, &three, "track"), Some("5"));
    assert_eq!(batch_value(&plan, &two, "track"), Some("A1"));
    assert_eq!(batch_value(&plan, &one, "track"), Some("7"));
    assert_eq!(batch_value(&plan, &one, "tracktotal"), Some("7"));
    app.set_locked_fields(&["track".into()]);
    let locked = app
        .preview_numbering(std::slice::from_ref(&one), &options, None)
        .unwrap();
    assert!(batch_value(&locked, &one, "track").is_none());
    assert_eq!(locked.locked_skipped[0].field, "track");
}

#[test]
fn native_numbering_rejects_overflow_bad_totals_and_configuration_before_publication() {
    let dir = TempDir::new("native-number-limits");
    let paths = [
        dir.tagged_flac("one.flac", "A", "One"),
        dir.tagged_flac("two.flac", "A", "Two"),
    ];
    let app = App::open(&dir.0, &dir.0.join("journal.sqlite")).unwrap();
    for options in [
        NumberingOptions {
            start: u32::MAX,
            ..Default::default()
        },
        NumberingOptions {
            step: 0,
            ..Default::default()
        },
        NumberingOptions {
            width: 13,
            ..Default::default()
        },
        NumberingOptions {
            write_total: true,
            total: Some(1),
            ..Default::default()
        },
        NumberingOptions {
            next_disc_on: Some(NumberBoundary::Folder),
            ..Default::default()
        },
        NumberingOptions {
            reset_track_on: Some(NumberBoundary::Field {
                field: "_path".into(),
            }),
            ..Default::default()
        },
    ] {
        assert!(app.preview_numbering(&paths, &options, None).is_err());
    }
    assert!(app
        .preview_numbering_with_cancel(&paths, &Default::default(), None, || true)
        .is_err());
}

#[test]
fn native_text_sidecars_use_staged_names_preserve_unicode_and_undo() {
    let dir = TempDir::new("native-sidecar-text");
    let path = dir.tagged_flac("one.flac", "Artist", "Old");
    let text = "第一行\nBjörk / AC/DC\n";
    let utf16: Vec<_> = [0xff, 0xfe]
        .into_iter()
        .chain(text.encode_utf16().flat_map(u16::to_le_bytes))
        .collect();
    std::fs::write(dir.0.join("New.txt"), utf16).unwrap();
    let mut app = App::open(&dir.0, &dir.0.join("journal.sqlite")).unwrap();
    let before = std::fs::read(&path).unwrap();
    let working = app
        .preview_tag_edits(&[TagEditDto {
            path: path.to_string_lossy().into(),
            field: "title".into(),
            value: Some("New".into()),
        }])
        .unwrap();
    let plan = app
        .preview_text_sidecars(
            std::slice::from_ref(&path),
            &TextSidecarOptions {
                field: "custom:LYRICS".into(),
                filename: "%title%.txt".into(),
                skip_missing: false,
            },
            Some(&working),
        )
        .unwrap();
    assert_eq!(batch_value(&plan, &path, "custom:LYRICS"), Some(text));
    assert_eq!(std::fs::read(&path).unwrap(), before);
    let batch = app.apply(&plan).unwrap();
    assert_eq!(
        TagEngine::read(&path).unwrap().tags[&TagField::Custom("LYRICS".into())],
        text
    );
    app.undo(batch.id).unwrap();
    assert!(!TagEngine::read(&path)
        .unwrap()
        .tags
        .contains_key(&TagField::Custom("LYRICS".into())));
}

#[test]
fn native_text_sidecars_confine_paths_and_report_only_explicitly_skipped_missing_files() {
    let dir = TempDir::new("native-sidecar-boundary");
    let path = dir.tagged_flac("one.flac", "Artist", "Title");
    let app = App::open(&dir.0, &dir.0.join("journal.sqlite")).unwrap();
    let mut options = TextSidecarOptions {
        field: "comment".into(),
        filename: "missing.txt".into(),
        skip_missing: false,
    };
    assert!(app
        .preview_text_sidecars(std::slice::from_ref(&path), &options, None)
        .is_err());
    options.skip_missing = true;
    let plan = app
        .preview_text_sidecars(std::slice::from_ref(&path), &options, None)
        .unwrap();
    assert_eq!(plan.unrendered[0].paths, vec![path.to_string_lossy()]);
    for filename in [
        "../escape.txt",
        "/etc/passwd",
        "C:\\Windows\\file.txt",
        "..\\escape.txt",
        "",
    ] {
        options.filename = filename.into();
        assert!(
            app.preview_text_sidecars(std::slice::from_ref(&path), &options, None)
                .is_err(),
            "{filename}"
        );
    }
    for bytes in [
        vec![0xff],
        vec![0xff, 0xfe, 0x00],
        vec![0xff, 0xfe, 0x00, 0xd8],
        vec![0x00],
        vec![0xff, 0xfe, 0x00, 0x00],
    ] {
        std::fs::write(dir.0.join("invalid.txt"), bytes).unwrap();
        options.filename = "invalid.txt".into();
        assert!(app
            .preview_text_sidecars(std::slice::from_ref(&path), &options, None)
            .is_err());
    }
    assert!(app
        .preview_text_sidecars_with_cancel(&[], &options, None, || true)
        .is_err());
}

#[cfg(unix)]
#[test]
fn native_text_sidecars_refuse_symlink_files_and_escaping_parent_links() {
    let dir = TempDir::new("native-sidecar-links");
    let outside = TempDir::new("native-sidecar-outside");
    let path = dir.tagged_flac("one.flac", "Artist", "Title");
    std::fs::write(outside.0.join("secret.txt"), "Do not import").unwrap();
    std::os::unix::fs::symlink(outside.0.join("secret.txt"), dir.0.join("link.txt")).unwrap();
    std::os::unix::fs::symlink(&outside.0, dir.0.join("folder")).unwrap();
    let app = App::open(&dir.0, &dir.0.join("journal.sqlite")).unwrap();
    for filename in ["link.txt", "folder/secret.txt"] {
        assert!(app
            .preview_text_sidecars(
                std::slice::from_ref(&path),
                &TextSidecarOptions {
                    field: "comment".into(),
                    filename: filename.into(),
                    skip_missing: true
                },
                None
            )
            .is_err());
    }
}

#[test]
fn native_list_operations_are_ordered_explicit_and_do_not_split_titles() {
    let dir = TempDir::new("native-list-ops");
    let path = dir.tagged_flac("one.flac", "Björk / 日本語 / Björk", "Hello; Goodbye");
    let mut app = App::open(&dir.0, &dir.0.join("journal.sqlite")).unwrap();
    let recipe = [
        FieldOperation::SplitValues {
            field: "artist".into(),
            separator: " / ".into(),
        },
        FieldOperation::DeduplicateValues {
            field: "artist".into(),
        },
        FieldOperation::Format {
            field: "comment".into(),
            pattern: "%artist%".into(),
        },
    ];
    let plan = app
        .preview_field_operations(std::slice::from_ref(&path), &recipe, None)
        .unwrap();
    assert_eq!(batch_value(&plan, &path, "artist"), Some("Björk; 日本語"));
    let batch = app.apply(&plan).unwrap();
    assert_eq!(
        TagEngine::read(&path).unwrap().tags[&TagField::Title],
        "Hello; Goodbye"
    );
    let merged = app
        .preview_field_operations(
            std::slice::from_ref(&path),
            &[FieldOperation::MergeValues {
                field: "artist".into(),
                separator: " & ".into(),
            }],
            None,
        )
        .unwrap();
    assert_eq!(
        batch_value(&merged, &path, "artist"),
        Some("Björk & 日本語")
    );
    assert!(app
        .preview_field_operations(
            std::slice::from_ref(&path),
            &[FieldOperation::MergeValues {
                field: "artist".into(),
                separator: "; ".into()
            }],
            None
        )
        .is_err());
    app.undo(batch.id).unwrap();
    assert_eq!(
        TagEngine::read(&path).unwrap().tags[&TagField::Artist],
        "Björk / 日本語 / Björk"
    );
    for field in ["title", "custom:Credits"] {
        assert!(app
            .preview_field_operations(
                &[],
                &[FieldOperation::SplitValues {
                    field: field.into(),
                    separator: ";".into()
                }],
                None
            )
            .is_err());
    }
}

#[test]
fn native_batches_reject_stale_working_tags_and_cancellation_without_disk_writes() {
    let dir = TempDir::new("native-batch-stale");
    let path = dir.tagged_flac("one.flac", "Artist", "Title");
    let app = App::open(&dir.0, &dir.0.join("journal.sqlite")).unwrap();
    let mut working = app
        .preview_tag_edits(&[TagEditDto {
            path: path.to_string_lossy().into(),
            field: "title".into(),
            value: Some("Staged".into()),
        }])
        .unwrap();
    working.changes[0].tag_changes[0].old = Some("Wrong baseline".into());
    assert!(app
        .preview_numbering(
            std::slice::from_ref(&path),
            &Default::default(),
            Some(&working)
        )
        .is_err());
    let before = std::fs::read(&path).unwrap();
    let calls = std::cell::Cell::new(0);
    assert!(app
        .preview_numbering_with_cancel(
            std::slice::from_ref(&path),
            &Default::default(),
            None,
            || {
                calls.set(calls.get() + 1);
                calls.get() > 3
            }
        )
        .is_err());
    assert_eq!(std::fs::read(&path).unwrap(), before);
}

#[test]
fn desktop_numbering_keeps_track_disc_scopes_and_staged_album_boundaries() {
    let dir = TempDir::new("desktop-numbering-scopes");
    let paths: Vec<_> = (0..6)
        .map(|i| dir.tagged_flac(&format!("{i}.flac"), "Artist", "Title"))
        .collect();
    let app = App::open(&dir.0, &dir.0.join("journal.sqlite")).unwrap();
    let groups = ["one", "one", "two", "one", "one", "two"].map(String::from);
    let working = app
        .preview_tag_edits(
            &paths
                .iter()
                .enumerate()
                .flat_map(|(i, path)| {
                    [
                        TagEditDto {
                            path: path.to_string_lossy().into(),
                            field: "album".into(),
                            value: Some(if i < 3 { "A" } else { "B" }.into()),
                        },
                        TagEditDto {
                            path: path.to_string_lossy().into(),
                            field: "comment".into(),
                            value: Some("Pending".into()),
                        },
                    ]
                })
                .collect::<Vec<_>>(),
        )
        .unwrap();
    let options = NumberingDialogOptions::Disc {
        start: "1".into(),
        disc_total: true,
        advance_group: true,
        reset_album: true,
    };
    let preview = app
        .preview_numbering_dialog_with_cancel(&paths, &groups, &options, Some(&working), || false)
        .unwrap();
    for (i, number) in ["1", "1", "2", "1", "1", "2"].iter().enumerate() {
        assert_eq!(batch_value(&preview.plan, &paths[i], "disc"), Some(*number));
        assert_eq!(
            batch_value(&preview.plan, &paths[i], "disctotal"),
            Some("2")
        );
    }
    assert!(preview
        .edits
        .iter()
        .all(|edit| ["disc", "disctotal"].contains(&edit.field.as_str())));
    assert!(preview
        .plan
        .changes
        .iter()
        .all(|change| change
            .tag_changes
            .iter()
            .all(|edit| ["disc", "disctotal"].contains(&edit.field.as_str()))));
    let options = NumberingDialogOptions::Tracks {
        start: "7".into(),
        track_total: true,
        restart_group: true,
    };
    let preview = app
        .preview_numbering_dialog_with_cancel(&paths, &groups, &options, Some(&working), || false)
        .unwrap();
    for (i, number) in ["7", "8", "7", "7", "8", "7"].iter().enumerate() {
        assert_eq!(
            batch_value(&preview.plan, &paths[i], "track"),
            Some(*number)
        );
    }
    assert!(preview
        .edits
        .iter()
        .all(|edit| ["track", "tracktotal"].contains(&edit.field.as_str())));
}

#[test]
fn desktop_vinyl_reads_staged_disc_and_counts_each_side_and_album() {
    let dir = TempDir::new("desktop-vinyl-source");
    let paths: Vec<_> = (0..7)
        .map(|i| dir.tagged_flac(&format!("{i}.flac"), "Artist", "Title"))
        .collect();
    let app = App::open(&dir.0, &dir.0.join("journal.sqlite")).unwrap();
    let raw = ["A1", "A2", "3b", "A1", "B", "B2", "12"];
    let working = app
        .preview_tag_edits(
            &paths
                .iter()
                .enumerate()
                .flat_map(|(i, path)| {
                    [
                        TagEditDto {
                            path: path.to_string_lossy().into(),
                            field: "disc".into(),
                            value: Some(raw[i].into()),
                        },
                        TagEditDto {
                            path: path.to_string_lossy().into(),
                            field: "album".into(),
                            value: Some(if i < 3 { "A" } else { "B" }.into()),
                        },
                    ]
                })
                .collect::<Vec<_>>(),
        )
        .unwrap();
    let options = NumberingDialogOptions::Vinyl {
        source: "disc".into(),
        keep_position: true,
        track_total: true,
        disc_total: true,
    };
    let preview = app
        .preview_numbering_dialog_with_cancel(
            &paths,
            &vec![String::new(); paths.len()],
            &options,
            Some(&working),
            || false,
        )
        .unwrap();
    assert_eq!((preview.matched, preview.skipped), (6, 1));
    for (i, (track, total, disc)) in [
        ("1", "2", "1"),
        ("2", "2", "1"),
        ("3", "3", "2"),
        ("1", "1", "1"),
        ("1", "2", "2"),
        ("2", "2", "2"),
    ]
    .iter()
    .enumerate()
    {
        assert_eq!(batch_value(&preview.plan, &paths[i], "track"), Some(*track));
        assert_eq!(
            batch_value(&preview.plan, &paths[i], "tracktotal"),
            Some(*total)
        );
        assert_eq!(batch_value(&preview.plan, &paths[i], "disc"), Some(*disc));
        assert_eq!(
            batch_value(&preview.plan, &paths[i], "disctotal"),
            Some("2")
        );
        assert_eq!(
            batch_value(&preview.plan, &paths[i], "custom:POSITION"),
            Some(raw[i])
        );
    }
    assert!(preview
        .edits
        .iter()
        .all(|edit| edit.path != paths[6].to_string_lossy()));
}

#[test]
fn desktop_numbering_refuses_duplicates_misalignment_overflow_locks_and_cancellation() {
    let dir = TempDir::new("desktop-numbering-refusal");
    let one = dir.tagged_flac("one.flac", "Artist", "Title");
    let two = dir.tagged_flac("two.flac", "Artist", "Title");
    let app = App::open(&dir.0, &dir.0.join("journal.sqlite")).unwrap();
    let options = NumberingDialogOptions::Tracks {
        start: u32::MAX.to_string(),
        track_total: false,
        restart_group: false,
    };
    let paths = [one.clone(), two];
    let groups = ["a".into(), "b".into()];
    assert!(app
        .preview_numbering_dialog_with_cancel(&paths, &groups, &options, None, || false)
        .is_err());
    assert!(app
        .preview_numbering_dialog_with_cancel(&paths, &[], &options, None, || false)
        .is_err());
    assert!(app
        .preview_numbering_dialog_with_cancel(&[one.clone(), one], &groups, &options, None, || {
            false
        })
        .is_err());
    assert!(app
        .preview_numbering_dialog_with_cancel(&paths, &groups, &options, None, || true)
        .is_err());
    // Disc-only numbering never advances an unused track counter.
    let disc = NumberingDialogOptions::Disc {
        start: u32::MAX.to_string(),
        disc_total: false,
        advance_group: false,
        reset_album: false,
    };
    assert!(app
        .preview_numbering_dialog_with_cancel(&paths, &groups, &disc, None, || false)
        .is_ok());
    app.set_locked_fields(&["disc".into()]);
    assert!(app
        .preview_numbering_dialog_with_cancel(&paths, &groups, &disc, None, || false)
        .is_err());
}

#[test]
fn desktop_assignments_include_disk_equal_values_to_replace_pending_values() {
    let dir = TempDir::new("desktop-numbering-unchanged");
    let path = dir.tagged_flac("one.flac", "Artist", "Title");
    let mut app = App::open(&dir.0, &dir.0.join("journal.sqlite")).unwrap();
    let initial = app
        .preview_tag_edits(&[TagEditDto {
            path: path.to_string_lossy().into(),
            field: "disc".into(),
            value: Some("2".into()),
        }])
        .unwrap();
    app.apply(&initial).unwrap();
    let options = NumberingDialogOptions::Disc {
        start: "2".into(),
        disc_total: false,
        advance_group: false,
        reset_album: false,
    };
    let preview = app
        .preview_numbering_dialog_with_cancel(
            std::slice::from_ref(&path),
            &[String::new()],
            &options,
            None,
            || false,
        )
        .unwrap();
    assert_eq!(preview.matched, 1);
    assert_eq!(preview.edits[0].value.as_deref(), Some("2"));
    assert!(preview.plan.changes.is_empty());
}

#[test]
fn desktop_numbering_rejects_stale_source_baselines_without_writing() {
    let dir = TempDir::new("desktop-numbering-stale-source");
    let path = dir.tagged_flac("one.flac", "Artist", "Title");
    let app = App::open(&dir.0, &dir.0.join("journal.sqlite")).unwrap();
    let mut working = app
        .preview_tag_edits(&[TagEditDto {
            path: path.to_string_lossy().into(),
            field: "album".into(),
            value: Some("Staged".into()),
        }])
        .unwrap();
    working.changes[0].tag_changes[0].old = Some("Obsolete album".into());
    let options = NumberingDialogOptions::Disc {
        start: "1".into(),
        disc_total: true,
        advance_group: true,
        reset_album: true,
    };
    let before = std::fs::read(&path).unwrap();
    assert!(app
        .preview_numbering_dialog_with_cancel(
            std::slice::from_ref(&path),
            &[String::new()],
            &options,
            Some(&working),
            || false
        )
        .is_err());
    assert_eq!(std::fs::read(&path).unwrap(), before);
}
