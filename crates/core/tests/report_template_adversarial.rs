//! Independent public-interface report rendering probes. No filesystem publisher.
use std::{
    alloc::{GlobalAlloc, Layout, System},
    cell::{Cell, RefCell},
    collections::BTreeSet,
    process::Command,
    sync::atomic::{AtomicUsize, Ordering},
};
use tagrex_core::{
    mask::FileContext,
    model::{TagField, TagMap},
    report_template::{
        Aggregate, CompiledTemplate, Error, Escape, GroupOrder, Node, Row, Template,
    },
    resources::{Budget, Charge, Limits},
    text_codec::TextEncoding,
};

struct Observed;
static LIVE: AtomicUsize = AtomicUsize::new(0);
static PEAK: AtomicUsize = AtomicUsize::new(0);
fn add(bytes: usize) {
    let next = LIVE.fetch_add(bytes, Ordering::Relaxed) + bytes;
    PEAK.fetch_max(next, Ordering::Relaxed);
}
// SAFETY: each operation delegates unchanged pointer/layout arguments to System;
// observation never dereferences user allocations or changes allocation choices.
unsafe impl GlobalAlloc for Observed {
    unsafe fn alloc(&self, l: Layout) -> *mut u8 {
        let p = unsafe { System.alloc(l) };
        if !p.is_null() {
            add(l.size());
        }
        p
    }
    unsafe fn alloc_zeroed(&self, l: Layout) -> *mut u8 {
        let p = unsafe { System.alloc_zeroed(l) };
        if !p.is_null() {
            add(l.size());
        }
        p
    }
    unsafe fn dealloc(&self, p: *mut u8, l: Layout) {
        unsafe { System.dealloc(p, l) };
        LIVE.fetch_sub(l.size(), Ordering::Relaxed);
    }
    unsafe fn realloc(&self, p: *mut u8, l: Layout, n: usize) -> *mut u8 {
        let q = unsafe { System.realloc(p, l, n) };
        if !q.is_null() {
            if n >= l.size() {
                add(n - l.size());
            } else {
                LIVE.fetch_sub(l.size() - n, Ordering::Relaxed);
            }
        }
        q
    }
}
#[global_allocator]
static ALLOCATOR: Observed = Observed;
fn isolated(name: &str) -> bool {
    if std::env::var("TAGREX_REPORT_REVIEW_CASE").as_deref() == Ok(name) {
        return false;
    }
    let output = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", name, "--nocapture"])
        .env("TAGREX_REPORT_REVIEW_CASE", name)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    true
}
fn budget() -> Budget {
    Budget::new(Limits {
        retained: 64 << 20,
        transient: 128 << 20,
        total: 192 << 20,
        jobs: 2,
    })
}
fn lit(s: &str) -> Node {
    Node::Literal(s.into())
}
fn mask(s: &str) -> Node {
    Node::Mask {
        pattern: s.into(),
        escape: Escape::Plain,
    }
}
fn compile(nodes: Vec<Node>, b: Budget) -> CompiledTemplate {
    CompiledTemplate::compile_in(&Template { version: 1, nodes }, b).unwrap()
}
fn row(tags: &TagMap, duration: Option<u64>) -> Row<'_> {
    Row {
        tags,
        file: FileContext::default(),
        duration_seconds: duration,
    }
}

#[test]
fn xml_text_does_not_emit_raw_carriage_returns_that_xml_readers_normalize() {
    let b = budget();
    let template = compile(
        vec![
            lit("<title>"),
            Node::Mask {
                pattern: "%title%".into(),
                escape: Escape::XmlText,
            },
            lit("</title>"),
        ],
        b.clone(),
    );
    let tags = [(TagField::Title, "first\rsecond\r\nthird\nfourth".into())]
        .into_iter()
        .collect();
    let output = template
        .render(&[row(&tags, None)], TextEncoding::Utf8, false, || false)
        .unwrap();
    assert!(!output.bytes().contains(&b'\r'),"raw CR in XML character data is normalized to LF by independent consumers, so this output changes the supplied value: {:?}",String::from_utf8_lossy(output.bytes()));
    drop(output);
    drop(template);
    assert_eq!(b.usage().0, Charge::default());
}

#[test]
#[ignore = "Independent Python XML consumer roundtrip; explicit backend review probe"]
fn xml_consumer_retains_cr_lf_tabs_and_unicode_across_supported_unicode_encodings() {
    use std::{io::Write, process::Stdio};
    let values = ["\r", "A\rB", "A\r\nB\nC\tD 尾", "<&\"'\r尾"];
    for value in values {
        let template = compile(
            vec![
                lit("<title>"),
                Node::Mask {
                    pattern: "%title%".into(),
                    escape: Escape::XmlText,
                },
                lit("</title>"),
            ],
            budget(),
        );
        let tags = [(TagField::Title, value.into())].into_iter().collect();
        for encoding in [
            TextEncoding::Utf8,
            TextEncoding::Utf16Le,
            TextEncoding::Utf16Be,
        ] {
            let output = template
                .render(&[row(&tags, None)], encoding, true, || false)
                .unwrap();
            let script="import sys,xml.etree.ElementTree as E; actual=E.fromstring(sys.stdin.buffer.read()).text or ''; assert actual==sys.argv[1],(repr(actual),repr(sys.argv[1]))";
            let mut child = Command::new(
                std::env::var_os("TAGREX_TEST_PYTHON").unwrap_or_else(|| "python3".into()),
            )
            .args(["-c", script, value])
            .stdin(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
            child
                .stdin
                .take()
                .unwrap()
                .write_all(output.bytes())
                .unwrap();
            let result = child.wait_with_output().unwrap();
            assert!(
                result.status.success(),
                "{}",
                String::from_utf8_lossy(&result.stderr)
            );
        }
    }
}

#[test]
fn deterministic_nested_group_reports_match_a_separate_selection_oracle() {
    let mut seed = 0x2026_1118u64;
    for case in 0..128 {
        let count = case % 23;
        let mut data = Vec::new();
        let mut durations = Vec::new();
        for index in 0..count {
            seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
            let album = ["Z", "a", "尾", ""][(seed >> 16) as usize % 4];
            let genre = ["g2", "g1", ""][(seed >> 24) as usize % 3];
            let duration = match (seed >> 32) % 3 {
                0 => None,
                1 => Some(0),
                _ => Some((index % 7) as u64),
            };
            data.push(
                [
                    (TagField::Album, album.into()),
                    (TagField::Genre, genre.into()),
                    (TagField::Title, format!("{case}:{index}; 尾")),
                ]
                .into_iter()
                .collect::<TagMap>(),
            );
            durations.push(duration);
        }
        let rows: Vec<_> = data
            .iter()
            .zip(&durations)
            .map(|(tags, d)| row(tags, *d))
            .collect();
        let template = compile(
            vec![Node::GroupBy {
                key_pattern: "%album%".into(),
                order: GroupOrder::Lexical,
                body: vec![
                    Node::GroupKey(Escape::Plain),
                    lit("["),
                    Node::GroupBy {
                        key_pattern: "%genre%".into(),
                        order: GroupOrder::Encounter,
                        body: vec![
                            Node::GroupKey(Escape::Plain),
                            lit(":"),
                            Node::Aggregate(Aggregate::Count),
                            lit("/"),
                            Node::Aggregate(Aggregate::KnownDurationSeconds),
                            lit("/"),
                            Node::Aggregate(Aggregate::UnknownDurations),
                            Node::EachTrack(vec![lit("{"), mask("%title%"), lit("}")]),
                        ],
                    },
                    lit("]"),
                    Node::GroupKey(Escape::Plain),
                ],
            }],
            budget(),
        );
        let mut expected = String::new();
        let albums: BTreeSet<_> = data
            .iter()
            .map(|tags| tags[&TagField::Album].as_str())
            .collect();
        for album in albums {
            expected.push_str(album);
            expected.push('[');
            let album_indices: Vec<_> = (0..count)
                .filter(|&i| data[i][&TagField::Album] == album)
                .collect();
            let mut genres = Vec::new();
            for &i in &album_indices {
                let genre = &data[i][&TagField::Genre];
                if !genres.contains(&genre.as_str()) {
                    genres.push(genre.as_str());
                }
            }
            for genre in genres {
                let members: Vec<_> = album_indices
                    .iter()
                    .copied()
                    .filter(|&i| data[i][&TagField::Genre] == genre)
                    .collect();
                let known: u64 = members.iter().filter_map(|&i| durations[i]).sum();
                let unknown = members.iter().filter(|&&i| durations[i].is_none()).count();
                expected.push_str(&format!("{genre}:{}/{known}/{unknown}", members.len()));
                for i in members {
                    expected.push('{');
                    expected.push_str(&data[i][&TagField::Title]);
                    expected.push('}');
                }
            }
            expected.push(']');
            expected.push_str(album);
        }
        let output = template
            .render(&rows, TextEncoding::Utf8, false, || false)
            .unwrap();
        assert_eq!(
            std::str::from_utf8(output.bytes()).unwrap(),
            expected,
            "case {case}"
        );
    }
}

#[test]
fn compiled_and_rendered_owners_cover_observed_peak_and_reentrant_cancellation_preserves_inner_result(
) {
    let name="compiled_and_rendered_owners_cover_observed_peak_and_reentrant_cancellation_preserves_inner_result";
    if isolated(name) {
        return;
    }
    let input = Template {
        version: 1,
        nodes: vec![Node::EachTrack(vec![
            Node::Mask {
                pattern: "$upper(%title%)".into(),
                escape: Escape::CsvCell,
            },
            lit("\n"),
        ])],
    };
    let tags = [(TagField::Title, "\"İ / 尾 & ".repeat(4000))]
        .into_iter()
        .collect::<TagMap>();
    let rows = vec![row(&tags, None); 8];
    let b = budget();
    let baseline = LIVE.load(Ordering::Relaxed);
    PEAK.store(baseline, Ordering::Relaxed);
    let compiled = CompiledTemplate::compile_in(&input, b.clone()).unwrap();
    let output = compiled
        .render(&rows, TextEncoding::Utf16Be, true, || false)
        .unwrap();
    let high = b.usage().1;
    let peak = PEAK.load(Ordering::Relaxed).saturating_sub(baseline);
    assert!(
        peak < high.retained.saturating_add(high.transient) + 65536,
        "peak={peak}, admitted={high:?}"
    );
    drop(output);
    let base = b.usage().0;
    let entered = Cell::new(false);
    let calls = Cell::new(0);
    let kept = RefCell::new(None);
    let outer = compiled.render(&rows, TextEncoding::Utf8, false, || {
        calls.set(calls.get() + 1);
        if !entered.replace(true) {
            *kept.borrow_mut() = Some(
                compiled
                    .render(&rows[..1], TextEncoding::Utf8, false, || false)
                    .unwrap(),
            );
        }
        calls.get() > 4
    });
    assert!(matches!(outer, Err(Error::Cancelled)));
    assert_eq!(b.usage().0.transient, 0);
    assert_eq!(b.usage().0.jobs, 0);
    assert!(b.usage().0.retained > base.retained);
    assert!(kept.borrow().as_ref().unwrap().bytes().starts_with(b"\""));
    drop(compiled);
    assert!(b.usage().0.retained > 0);
    kept.borrow_mut().take();
    assert_eq!(b.usage().0, Charge::default());
}

#[test]
fn cancellation_during_escape_and_group_build_preserves_previous_retained_result() {
    let b = budget();
    let escaped = compile(
        vec![Node::Mask {
            pattern: "%title%".into(),
            escape: Escape::XmlText,
        }],
        b.clone(),
    );
    let tags = [(TagField::Title, "<&尾\r".repeat(10000))]
        .into_iter()
        .collect::<TagMap>();
    let rows = [row(&tags, None)];
    let retained = escaped
        .render(&rows, TextEncoding::Utf8, false, || false)
        .unwrap();
    let before = b.usage().0;
    let calls = Cell::new(0);
    let cancelled = escaped.render(&rows, TextEncoding::Utf8, false, || {
        calls.set(calls.get() + 1);
        calls.get() >= 8
    });
    assert!(matches!(cancelled, Err(Error::Cancelled)));
    assert_eq!(b.usage().0, before);
    assert!(retained.bytes().starts_with(b"&lt;&amp;"));
    let grouped = compile(
        vec![Node::GroupBy {
            key_pattern: "%title%".into(),
            order: GroupOrder::Encounter,
            body: vec![Node::Aggregate(Aggregate::Count)],
        }],
        b.clone(),
    );
    let before = b.usage().0;
    let calls = Cell::new(0);
    let data: Vec<_> = (0..32)
        .map(|i| {
            [(TagField::Title, format!("group-{i}"))]
                .into_iter()
                .collect::<TagMap>()
        })
        .collect();
    let rows: Vec<_> = data.iter().map(|tags| row(tags, None)).collect();
    assert!(matches!(
        grouped.render(&rows, TextEncoding::Utf8, false, || {
            calls.set(calls.get() + 1);
            calls.get() >= 8
        }),
        Err(Error::Cancelled)
    ));
    assert_eq!(b.usage().0, before);
    drop(grouped);
    drop(escaped);
    drop(retained);
    assert_eq!(b.usage().0, Charge::default());
}

#[test]
#[ignore = "Independent Python CSV consumer across UTF8/UTF16 and newline forms"]
fn csv_consumer_preserves_embedded_delimiters_newlines_and_control_scalars() {
    use std::{io::Write, process::Stdio};
    let values = ["", "\r", "A\r\nB\nC\tD 尾", "comma,quote\"\0tail"];
    for value in values {
        let template = compile(
            vec![
                Node::Mask {
                    pattern: "%title%".into(),
                    escape: Escape::CsvCell,
                },
                lit("\r\n"),
            ],
            budget(),
        );
        let tags = [(TagField::Title, value.into())]
            .into_iter()
            .collect::<TagMap>();
        for (encoding, label) in [
            (TextEncoding::Utf8, "utf-8-sig"),
            (TextEncoding::Utf16Le, "utf-16"),
            (TextEncoding::Utf16Be, "utf-16"),
        ] {
            let output = template
                .render(&[row(&tags, None)], encoding, true, || false)
                .unwrap();
            // Arguments cannot carry NUL, so a separate expected byte sequence
            // is supplied as its Unicode scalar values rather than shell code.
            let expected = value
                .chars()
                .map(|c| u32::from(c).to_string())
                .collect::<Vec<_>>()
                .join(",");
            let script="import sys,csv,io; text=sys.stdin.buffer.read().decode(sys.argv[2]); rows=list(csv.reader(io.StringIO(text,newline=''))); expected=''.join(chr(int(x)) for x in sys.argv[1].split(',') if x); assert rows==[[expected]],repr(rows)";
            let mut child = Command::new(
                std::env::var_os("TAGREX_TEST_PYTHON").unwrap_or_else(|| "python3".into()),
            )
            .args(["-c", script, &expected, label])
            .stdin(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
            child
                .stdin
                .take()
                .unwrap()
                .write_all(output.bytes())
                .unwrap();
            let result = child.wait_with_output().unwrap();
            assert!(
                result.status.success(),
                "{}",
                String::from_utf8_lossy(&result.stderr)
            );
        }
    }
}

#[test]
fn markup_data_cr_references_do_not_rewrite_literals_or_csv_cells() {
    let template = compile(
        vec![
            lit("literal\r|"),
            Node::Mask {
                pattern: "%title%".into(),
                escape: Escape::XmlText,
            },
            lit("|"),
            Node::Mask {
                pattern: "%title%".into(),
                escape: Escape::HtmlText,
            },
            lit("|"),
            Node::Mask {
                pattern: "%title%".into(),
                escape: Escape::CsvCell,
            },
        ],
        budget(),
    );
    let tags = [(TagField::Title, "\r尾".into())]
        .into_iter()
        .collect::<TagMap>();
    let output = template
        .render(&[row(&tags, None)], TextEncoding::Utf8, false, || false)
        .unwrap();
    assert_eq!(
        std::str::from_utf8(output.bytes()).unwrap(),
        "literal\r|&#13;尾|&#13;尾|\"\r尾\""
    );
}
