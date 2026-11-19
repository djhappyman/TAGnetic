//! Independent public bounded-render allocation and semantic probes.
use std::{
    alloc::{GlobalAlloc, Layout, System},
    sync::atomic::{AtomicUsize, Ordering},
};
use tagrex_core::{
    mask::{BoundedRenderError, FileContext, Mask, MaskError},
    model::{TagField, TagMap},
};
struct Observed;
static LIVE: AtomicUsize = AtomicUsize::new(0);
static PEAK: AtomicUsize = AtomicUsize::new(0);
fn grow(bytes: usize) {
    let now = LIVE.fetch_add(bytes, Ordering::Relaxed) + bytes;
    PEAK.fetch_max(now, Ordering::Relaxed);
}
// SAFETY: all pointer/layout operations delegate unchanged to System. Counters
// observe successful allocations and never read or modify the allocation data.
unsafe impl GlobalAlloc for Observed {
    unsafe fn alloc(&self, l: Layout) -> *mut u8 {
        let p = unsafe { System.alloc(l) };
        if !p.is_null() {
            grow(l.size());
        }
        p
    }
    unsafe fn alloc_zeroed(&self, l: Layout) -> *mut u8 {
        let p = unsafe { System.alloc_zeroed(l) };
        if !p.is_null() {
            grow(l.size());
        }
        p
    }
    unsafe fn dealloc(&self, p: *mut u8, l: Layout) {
        unsafe { System.dealloc(p, l) };
        LIVE.fetch_sub(l.size(), Ordering::Relaxed);
    }
    unsafe fn realloc(&self, p: *mut u8, l: Layout, n: usize) -> *mut u8 {
        let p = unsafe { System.realloc(p, l, n) };
        if !p.is_null() {
            if n >= l.size() {
                grow(n - l.size());
            } else {
                LIVE.fetch_sub(l.size() - n, Ordering::Relaxed);
            }
        }
        p
    }
}
#[global_allocator]
static ALLOCATOR: Observed = Observed;
fn isolated(name: &str) -> bool {
    if std::env::var("TAGREX_MASK_ADVERSARIAL_CASE").as_deref() == Ok(name) {
        return false;
    }
    let out = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", name, "--nocapture"])
        .env("TAGREX_MASK_ADVERSARIAL_CASE", name)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    true
}
#[test]
fn every_prefix_candidate_is_considered_after_an_unusable_earlier_candidate() {
    for (title, pattern, expected) in [
        (
            "The Artist",
            "$stripprefix(%title%,UnreasonablyLongFirstCandidate,The)",
            "Artist",
        ),
        ("尾 Artist", "$stripprefix(%title%,xx,尾)", "Artist"),
        ("尾 Artist", "$swapprefix(%title%,xx,尾)", "Artist, 尾"),
    ] {
        let tags: TagMap = [(TagField::Title, title.into())].into();
        let mask = Mask::parse_for_render(pattern).unwrap();
        let bound = mask
            .render_work_bytes(&tags, &FileContext::default())
            .unwrap();
        assert_eq!(
            mask.render_value_with_budget(&tags, &FileContext::default(), bound)
                .unwrap(),
            expected,
            "{pattern}"
        );
    }
}
#[test]
fn deterministic_nested_render_allocations_and_errors_fit_the_admitted_allowance() {
    if isolated("deterministic_nested_render_allocations_and_errors_fit_the_admitted_allowance") {
        return;
    }
    let sources = [
        "",
        "尾",
        "İßﬃ  /; ",
        "\0\t\r\n",
        "The Artist (Original Mix)",
    ];
    let functions = [
        "$reverse(%title%)",
        "$caps($lower(%title%))",
        "$padleft(%title%,262144,尾)",
        "$padright($reverse(%title%),262144,😀)",
        "$insert(%title%,%title%,1)",
        "$replace(%title%,尾,İİİ)",
        "$left(%title%,%title%)",
        "$cutmix($trim(%title%))",
        "$if($and(%title%,x),[$upper(%title%)],%title%)",
        "$round(%title%,%title%)",
    ];
    let mut checked = 0;
    for source in sources {
        for repeat in [1, 32, 4096] {
            let tags: TagMap = [(TagField::Title, source.repeat(repeat))].into();
            for pattern in functions {
                let mask = Mask::parse_for_render(pattern).unwrap();
                let bound = mask
                    .render_work_bytes(&tags, &FileContext::default())
                    .unwrap();
                let before = LIVE.load(Ordering::Relaxed);
                PEAK.store(before, Ordering::Relaxed);
                let result = mask.render_value_with_budget(&tags, &FileContext::default(), bound);
                let actual = PEAK.load(Ordering::Relaxed).saturating_sub(before);
                assert!(actual<=bound,"{pattern} source={source:?} repeat={repeat}: allocatedpeak {actual} > admitted {bound}");
                match result {
                    Ok(value) => assert!(value.len() <= 1024 * 1024),
                    Err(BoundedRenderError::Mask(MaskError::BadArgument(detail))) => {
                        assert!(detail.len() <= 1027)
                    }
                    Err(BoundedRenderError::Mask(_)) => {}
                    Err(BoundedRenderError::TooLarge) => {
                        panic!("computed allowance refused itself")
                    }
                };
                checked += 1;
            }
        }
    }
    assert_eq!(checked, 150);
}
#[test]
fn malformed_utf8_boundaries_do_not_panic_and_filename_and_value_rendering_stay_distinct() {
    let tags: TagMap = [
        (TagField::Title, "尾尾x/7".into()),
        (TagField::TrackNumber, "7".into()),
    ]
    .into();
    let mask = Mask::parse_for_render("%track%/$cutmix(%title%)").unwrap();
    let work = mask
        .render_work_bytes(&tags, &FileContext::default())
        .unwrap();
    assert_eq!(
        mask.render_with_budget(&tags, &FileContext::default(), work)
            .unwrap(),
        "07/尾尾x_7"
    );
    assert_eq!(
        mask.render_value_with_budget(&tags, &FileContext::default(), work)
            .unwrap(),
        "7/尾尾x/7"
    );
    assert!(matches!(
        mask.extract("anything"),
        Err(MaskError::RenderOnly)
    ));
    assert!(matches!(
        mask.render_value_with_budget(&tags, &FileContext::default(), work - 1),
        Err(BoundedRenderError::TooLarge)
    ));
}

#[test]
fn boundary_sized_render_only_asts_compile_without_extraction_regex_allocation() {
    if isolated("boundary_sized_render_only_asts_compile_without_extraction_regex_allocation") {
        return;
    }
    let widest = format!("$and({})", vec!["a"; 4000].join(","));
    let mut nested = "%title%".to_owned();
    for _ in 0..28 {
        nested = format!("$if2({nested},x)");
    }
    let custom = format!("'{}'", "尾".repeat(2500));
    for source in [widest, nested, custom] {
        assert!(source.len() <= 8192);
        let before = LIVE.load(Ordering::Relaxed);
        PEAK.store(before, Ordering::Relaxed);
        let mask = Mask::parse_for_render(&source).unwrap();
        let peak = PEAK.load(Ordering::Relaxed).saturating_sub(before);
        let conservative = source.len() * 512 + 8192;
        assert!(
            peak <= conservative,
            "compileactual{peak}>bound{conservative}"
        );
        assert!(matches!(
            mask.extract("anything"),
            Err(MaskError::RenderOnly)
        ));
    }
}

#[cfg(unix)]
#[test]
fn invalid_os_path_expansion_and_maximum_technical_values_fit_admission() {
    if isolated("invalid_os_path_expansion_and_maximum_technical_values_fit_admission") {
        return;
    }
    use std::{
        ffi::OsString,
        os::unix::ffi::OsStringExt,
        path::PathBuf,
        time::{Duration, SystemTime},
    };
    let mut raw = b"/fixture/".to_vec();
    raw.extend([0xff; 32768]);
    raw.extend(b".flac");
    let path = PathBuf::from(OsString::from_vec(raw));
    let context = FileContext {
        path: Some(&path),
        props: Some(tagrex_core::model::AudioProps {
            duration_secs: u64::MAX,
            bitrate_kbps: Some(u32::MAX),
            sample_rate_hz: Some(u32::MAX),
            channels: Some(u8::MAX),
        }),
        size_bytes: Some(u64::MAX),
        modified: SystemTime::UNIX_EPOCH.checked_add(Duration::from_secs(u64::MAX / 2)),
        format: Some(tagrex_core::model::AudioFormat::Flac),
    };
    let tags = TagMap::new();
    for source in ["%filepath%","%filenameext%","%_length% %_length_sec% %_bitrate% %_samplerate% %_channels% %_filedate% %_filesize% %_filesize_bytes%"] {
        let mask=Mask::parse_for_render(source).unwrap();let bound=mask.render_work_bytes(&tags,&context).unwrap();
        let before=LIVE.load(Ordering::Relaxed);PEAK.store(before,Ordering::Relaxed);
        let output=mask.render_value_with_budget(&tags,&context,bound).unwrap();let peak=PEAK.load(Ordering::Relaxed).saturating_sub(before);
        assert!(peak<=bound,"{source}: actual{peak}>bound{bound}");if source=="%filenameext%" { assert!(output.is_empty(), "legacy component placeholders omit invalid UTF-8"); } else { assert!(!output.is_empty()); }
    }
}
