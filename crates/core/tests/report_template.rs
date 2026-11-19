use std::{cell::Cell, path::Path};
use tagrex_core::{
    mask::FileContext,
    model::{TagField, TagMap},
    report_template::{
        Aggregate, CompiledTemplate, Error, Escape, GroupOrder, Node, Row, Template,
    },
    resources::{Budget, Charge, Limits},
    text_codec::{decode, TextEncoding},
};
fn budget() -> Budget {
    Budget::new(Limits {
        retained: 128 << 20,
        transient: 384 << 20,
        total: 512 << 20,
        jobs: 2,
    })
}
fn literal(text: &str) -> Node {
    Node::Literal(text.into())
}
fn mask(pattern: &str) -> Node {
    Node::Mask {
        pattern: pattern.into(),
        escape: Escape::Plain,
    }
}
fn tags(album: &str, title: &str) -> TagMap {
    [
        (TagField::Album, album.into()),
        (TagField::Title, title.into()),
        (TagField::TrackNumber, "7".into()),
    ]
    .into_iter()
    .collect()
}
fn row(tags: &TagMap, duration: Option<u64>) -> Row<'_> {
    Row {
        tags,
        file: FileContext::default(),
        duration_seconds: duration,
    }
}
fn compile(nodes: Vec<Node>, budget: Budget) -> CompiledTemplate {
    CompiledTemplate::compile_in(&Template { version: 1, nodes }, budget).unwrap()
}
#[test]
fn encounter_grouping_counts_unknowns_and_unsanitized_rows_have_explicit_scopes() {
    let budget = budget();
    let template = compile(
        vec![
            literal("Library "),
            Node::Aggregate(Aggregate::Count),
            literal("\n"),
            Node::GroupBy {
                key_pattern: "%album%".into(),
                order: GroupOrder::Encounter,
                body: vec![
                    Node::GroupKey(Escape::Plain),
                    literal(":"),
                    Node::Aggregate(Aggregate::Count),
                    literal("/"),
                    Node::Aggregate(Aggregate::KnownDurationSeconds),
                    literal("/unknown="),
                    Node::Aggregate(Aggregate::UnknownDurations),
                    literal("\n"),
                    Node::EachTrack(vec![mask("%track% %title%"), literal("\n")]),
                ],
            },
        ],
        budget.clone(),
    );
    let a = tags("B", "A/B;尾");
    let b = tags("A", "Second");
    let c = tags("B", "Last");
    let rows = [row(&a, Some(3)), row(&b, None), row(&c, Some(0))];
    let output = template
        .render(&rows, TextEncoding::Utf8, false, || false)
        .unwrap();
    assert_eq!(
        std::str::from_utf8(output.bytes()).unwrap(),
        "Library 3\nB:2/3/unknown=0\n7 A/B;尾\n7 Last\nA:1/0/unknown=1\n7 Second\n"
    );
    assert!(output.policy_revision() != u64::MAX);
    drop(template);
    assert!(budget.usage().0.retained > 0);
    drop(output);
    assert_eq!(budget.usage().0, Charge::default());
}
#[test]
fn lexical_nested_groups_restore_outer_key_and_track_iteration_does_not_expand_scope() {
    let template = compile(
        vec![Node::GroupBy {
            key_pattern: "%album%".into(),
            order: GroupOrder::Lexical,
            body: vec![
                Node::GroupKey(Escape::Plain),
                literal("["),
                Node::GroupBy {
                    key_pattern: "%title%".into(),
                    order: GroupOrder::Lexical,
                    body: vec![
                        Node::GroupKey(Escape::Plain),
                        Node::EachTrack(vec![Node::Aggregate(Aggregate::Count)]),
                    ],
                },
                literal("]"),
                Node::GroupKey(Escape::Plain),
            ],
        }],
        budget(),
    );
    let a = tags("B", "z");
    let b = tags("A", "y");
    let c = tags("A", "x");
    let output = template
        .render(
            &[row(&a, None), row(&b, None), row(&c, None)],
            TextEncoding::Utf8,
            false,
            || false,
        )
        .unwrap();
    assert_eq!(
        std::str::from_utf8(output.bytes()).unwrap(),
        "A[x1y1]AB[z1]B"
    );
}
#[test]
fn escaping_and_strict_encodings_preserve_literal_text_and_invalid_xml_refuses() {
    let data = tags("", "<&\"'尾,\n");
    let template = compile(
        vec![
            Node::Mask {
                pattern: "%title%".into(),
                escape: Escape::HtmlText,
            },
            literal("\n"),
            Node::Mask {
                pattern: "%title%".into(),
                escape: Escape::CsvCell,
            },
        ],
        budget(),
    );
    for encoding in [
        TextEncoding::Utf8,
        TextEncoding::Utf16Le,
        TextEncoding::Utf16Be,
    ] {
        let result = template
            .render(&[row(&data, None)], encoding, true, || false)
            .unwrap();
        assert_eq!(
            decode(result.bytes(), encoding, 1 << 20).unwrap(),
            "&lt;&amp;&quot;&apos;尾,\n\n\"<&\"\"'尾,\n\""
        );
    }
    assert!(template
        .render(
            &[row(&data, None)],
            TextEncoding::from_label("windows-1252").unwrap(),
            false,
            || false
        )
        .is_err());
    let xml = compile(
        vec![Node::Mask {
            pattern: "%title%".into(),
            escape: Escape::XmlText,
        }],
        budget(),
    );
    let bad = tags("", "nul\0");
    assert!(matches!(
        xml.render(&[row(&bad, None)], TextEncoding::Utf8, false, || false),
        Err(Error::Invalid(_))
    ));
}
#[test]
fn supplied_context_is_used_without_opening_paths_and_missing_tags_render_empty() {
    let data = tags("", "Title");
    let template = compile(vec![mask("%artist%/%filenameext%/%_samplerate%")], budget());
    let result = template
        .render(
            &[Row {
                tags: &data,
                file: FileContext {
                    path: Some(Path::new("/does-not-exist/report-fixture.flac")),
                    props: Some(tagrex_core::model::AudioProps {
                        sample_rate_hz: Some(48000),
                        duration_secs: 0,
                        bitrate_kbps: None,
                        channels: None,
                    }),
                    ..Default::default()
                },
                duration_seconds: None,
            }],
            TextEncoding::Utf8,
            false,
            || false,
        )
        .unwrap();
    assert_eq!(
        std::str::from_utf8(result.bytes()).unwrap(),
        "/report-fixture.flac/48000"
    );
}
#[test]
fn future_invalid_and_oversized_templates_refuse_before_retained_admission() {
    let budget = budget();
    for template in [
        Template {
            version: 2,
            nodes: vec![],
        },
        Template {
            version: 1,
            nodes: vec![Node::GroupKey(Escape::Plain)],
        },
        Template {
            version: 1,
            nodes: (0..129).map(|_| literal("a")).collect(),
        },
        Template {
            version: 1,
            nodes: vec![literal(&"x".repeat(65537))],
        },
    ] {
        assert!(CompiledTemplate::compile_in(&template, budget.clone()).is_err());
        assert_eq!(budget.usage().0, Charge::default());
    }
    let mut nested = vec![literal("deep")];
    for _ in 0..10 {
        nested = vec![Node::EachTrack(nested)];
    }
    assert!(CompiledTemplate::compile_in(
        &Template {
            version: 1,
            nodes: nested
        },
        budget.clone()
    )
    .is_err());
    assert_eq!(budget.usage().0, Charge::default());
}
#[test]
fn cancellation_at_every_checkpoint_and_panics_release_work_without_partial_output() {
    let budget = budget();
    let template = compile(
        vec![Node::EachTrack(vec![mask("%title%"), literal("\n")])],
        budget.clone(),
    );
    let data = tags("", "A&B");
    let rows = vec![row(&data, None); 8];
    let calls = Cell::new(0);
    drop(
        template
            .render(&rows, TextEncoding::Utf8, false, || {
                calls.set(calls.get() + 1);
                false
            })
            .unwrap(),
    );
    let expected = budget.usage().0;
    for stop in 1..=calls.get() {
        let count = Cell::new(0);
        let result = template.render(&rows, TextEncoding::Utf8, false, || {
            count.set(count.get() + 1);
            count.get() >= stop
        });
        assert!(matches!(result, Err(Error::Cancelled)), "checkpoint {stop}");
        assert_eq!(budget.usage().0, expected);
    }
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        template.render(&rows, TextEncoding::Utf8, false, || {
            panic!("fixture cancellation callback panic")
        })
    }));
    assert!(result.is_err());
    assert_eq!(budget.usage().0, expected);
}
#[test]
fn group_fanout_output_and_duration_overflow_are_bounded_and_retryable() {
    let budget = budget();
    let template = compile(
        vec![Node::GroupBy {
            key_pattern: "%title%".into(),
            order: GroupOrder::Encounter,
            body: vec![Node::Aggregate(Aggregate::Count)],
        }],
        budget.clone(),
    );
    let source: Vec<_> = (0..4097).map(|i| tags("", &i.to_string())).collect();
    let rows: Vec<_> = source.iter().map(|tags| row(tags, None)).collect();
    let before = budget.usage().0;
    assert!(matches!(
        template.render(&rows, TextEncoding::Utf8, false, || false),
        Err(Error::Limit)
    ));
    assert_eq!(budget.usage().0, before);
    assert!(template
        .render(&rows[..1], TextEncoding::Utf8, false, || false)
        .is_ok());
    let overflow = compile(
        vec![Node::Aggregate(Aggregate::KnownDurationSeconds)],
        budget.clone(),
    );
    let data = tags("", "");
    assert!(matches!(
        overflow.render(
            &[row(&data, Some(u64::MAX)), row(&data, Some(1))],
            TextEncoding::Utf8,
            false,
            || false
        ),
        Err(Error::Limit)
    ));
    let large = compile(vec![Node::EachTrack(vec![mask("%title%")])], budget.clone());
    let data = tags("", &"x".repeat(100000));
    let before = budget.usage().0;
    assert!(matches!(
        large.render(
            &vec![row(&data, None); 64],
            TextEncoding::Utf8,
            false,
            || false
        ),
        Err(Error::Limit)
    ));
    assert_eq!(budget.usage().0, before);
}

#[test]
fn shared_pressure_refuses_without_retained_result_then_retries() {
    let budget = budget();
    let template = compile(vec![literal("ready")], budget.clone());
    let before = budget.usage().0;
    let pressure = budget
        .try_acquire(Charge {
            transient: 384 << 20,
            ..Default::default()
        })
        .unwrap();
    assert!(matches!(
        template.render(&[], TextEncoding::Utf8, false, || false),
        Err(Error::Resource(_))
    ));
    drop(pressure);
    assert_eq!(budget.usage().0, before);
    assert_eq!(
        template
            .render(&[], TextEncoding::Utf8, false, || false)
            .unwrap()
            .bytes(),
        b"ready"
    );
}

#[test]
#[ignore = "Independent Python CSV/XML parser readback; run by backend stress gates"]
fn independent_csv_and_xml_consumers_read_utf8_and_utf16_reports() {
    use std::{
        io::Write,
        process::{Command, Stdio},
    };
    let value = "Quoted \" & <tail> 尾\nSecond line";
    let data = tags("", value);
    for (escape,nodes,script) in [
        (Escape::CsvCell,vec![Node::EachTrack(vec![Node::Mask{pattern:"%title%".into(),escape:Escape::CsvCell},literal("\r\n")])],"import csv,sys,io; values=list(csv.reader(io.StringIO(sys.stdin.buffer.read().decode('utf-8'),newline=''))); assert values==[[sys.argv[1]]]; print('csv readback passed')"),
        (Escape::XmlText,vec![literal("<library>"),Node::EachTrack(vec![literal("<title>"),Node::Mask{pattern:"%title%".into(),escape:Escape::XmlText},literal("</title>")]),literal("</library>")],"import sys,xml.etree.ElementTree as ET; root=ET.fromstring(sys.stdin.buffer.read()); assert [x.text for x in root]==[sys.argv[1]]; print('xml readback passed')"),
    ] {
        let template=compile(nodes,budget());
        for encoding in [TextEncoding::Utf8,TextEncoding::Utf16Le,TextEncoding::Utf16Be] {
            let result=template.render(&[row(&data,None)],encoding,!matches!(escape,Escape::CsvCell),||false).unwrap();
            let bytes=if matches!(escape,Escape::CsvCell) {decode(result.bytes(),encoding,1<<20).unwrap().into_bytes()} else {result.bytes().to_vec()};
            let python=std::env::var_os("TAGREX_TEST_PYTHON").unwrap_or_else(||"python3".into());
            let mut child=Command::new(python).args(["-c",script,value]).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn().unwrap();
            child.stdin.take().unwrap().write_all(&bytes).unwrap();
            let output=child.wait_with_output().unwrap();
            assert!(output.status.success(),"{}",String::from_utf8_lossy(&output.stderr));
        }
    }
}

#[test]
#[ignore = "Manual 100,000-row grouped report cold/warm native compute probe"]
fn native_grouped_report_scale_probe() {
    use std::time::Instant;
    let budget = budget();
    let start = Instant::now();
    let template = compile(
        vec![Node::GroupBy {
            key_pattern: "%album%".into(),
            order: GroupOrder::Encounter,
            body: vec![
                Node::GroupKey(Escape::Plain),
                literal("\n"),
                Node::EachTrack(vec![mask("%title%"), literal("\n")]),
            ],
        }],
        budget.clone(),
    );
    let compile_time = start.elapsed();
    let source: Vec<_> = (0..100_000)
        .map(|i| tags(&format!("Group {}", i % 100), &format!("Track {i:06} 尾")))
        .collect();
    let rows: Vec<_> = source.iter().map(|tags| row(tags, Some(1))).collect();
    let mut samples = Vec::new();
    let mut expected = None;
    for _ in 0..4 {
        let start = Instant::now();
        let output = template
            .render(&rows, TextEncoding::Utf8, false, || false)
            .unwrap();
        samples.push(start.elapsed());
        assert!(output.bytes().starts_with(b"Group 0\nTrack 000000"));
        let hash = output.bytes().iter().fold(0u64, |sum, byte| {
            sum.wrapping_mul(31).wrapping_add(u64::from(*byte))
        });
        assert_eq!(
            *expected.get_or_insert((output.bytes().len(), hash)),
            (output.bytes().len(), hash)
        );
    }
    eprintln!("REPORT_SCALE rows=100000 groups=100 compile={compile_time:?} samples={samples:?} bytes={} highwater={:?}",expected.unwrap().0,budget.usage().1);
    drop(template);
    assert_eq!(budget.usage().0, Charge::default());
}

#[test]
fn final_cancellation_callback_policy_change_never_publishes_a_mixed_generation() {
    use std::process::Command;
    let name = "final_cancellation_callback_policy_change_never_publishes_a_mixed_generation";
    if std::env::var("TAGREX_REPORT_POLICY_CHILD").as_deref() != Ok("1") {
        let output = Command::new(std::env::current_exe().unwrap())
            .args(["--exact", name, "--nocapture"])
            .env("TAGREX_REPORT_POLICY_CHILD", "1")
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        return;
    }
    let budget = budget();
    let template = compile(vec![literal("policy")], budget.clone());
    let calls = Cell::new(0);
    drop(
        template
            .render(&[], TextEncoding::Utf8, false, || {
                calls.set(calls.get() + 1);
                false
            })
            .unwrap(),
    );
    let final_call = calls.get();
    let before = budget.usage().0;
    let current = tagrex_core::model::multi_value_separator();
    let counter = Cell::new(0);
    let result = template.render(&[], TextEncoding::Utf8, false, || {
        counter.set(counter.get() + 1);
        if counter.get() == final_call {
            tagrex_core::model::set_multi_value_separator("report-test-separator");
        }
        false
    });
    tagrex_core::model::set_multi_value_separator(&current);
    assert!(matches!(result, Err(Error::Changed)));
    assert_eq!(budget.usage().0, before);
}

#[test]
fn invalid_large_unicode_numeric_argument_returns_a_bounded_diagnostic() {
    let budget = budget();
    let template = compile(vec![mask("$left(x,%title%)")], budget.clone());
    let data = tags("", &"尾".repeat(100_000));
    let before = budget.usage().0;
    let error = template
        .render(&[row(&data, None)], TextEncoding::Utf8, false, || false)
        .unwrap_err();
    assert!(matches!(error, Error::Mask(_)));
    assert!(error.to_string().len() < 1200);
    assert_eq!(budget.usage().0, before);
}

#[test]
fn extract_only_masks_are_refused_at_compilation_before_any_rows_are_read() {
    let budget = budget();
    let template = Template {
        version: 1,
        nodes: vec![mask("%skip%")],
    };
    assert!(matches!(
        CompiledTemplate::compile_in(&template, budget.clone()),
        Err(Error::Invalid(_))
    ));
    assert_eq!(budget.usage().0, Charge::default());
}
