//! Work-bounded rendering uses the same AST and preserves legacy semantics.
use tagrex_core::{
    mask::{BoundedRenderError, FileContext, Mask, MaskError},
    model::{TagField, TagMap},
};

#[test]
fn unicode_cutmix_does_not_split_a_codepoint_when_comparing_ascii_noise_suffixes() {
    let mask = Mask::parse_for_render("$cutmix(%title%)").unwrap();
    let tags: TagMap = [(TagField::Title, "尾尾x".into())].into();
    let budget = mask
        .render_work_bytes(&tags, &FileContext::default())
        .unwrap();
    assert_eq!(
        mask.render_value_with_budget(&tags, &FileContext::default(), budget)
            .unwrap(),
        "尾尾x"
    );
}

#[test]
fn render_only_parser_preserves_rendering_and_explicitly_refuses_extraction() {
    let tags: TagMap = [
        (TagField::Artist, "İ Artist".into()),
        (TagField::Title, "尾 / Title".into()),
    ]
    .into();
    for pattern in [
        "%artist% - %title%",
        "$upper(%artist%) [%title%]",
        "'literal ,,[[$' %title%",
    ] {
        let original = Mask::parse(pattern).unwrap();
        let mask = Mask::parse_for_render(pattern).unwrap();
        assert!(matches!(
            mask.extract("anything"),
            Err(MaskError::RenderOnly)
        ));
        let limit = mask
            .render_work_bytes(&tags, &FileContext::default())
            .unwrap();
        assert_eq!(
            mask.render_with_budget(&tags, &FileContext::default(), limit)
                .unwrap(),
            original.render(&tags).unwrap()
        );
        assert_eq!(
            mask.render_value_with_budget(&tags, &FileContext::default(), limit)
                .unwrap(),
            original
                .render_value_with(&tags, &FileContext::default())
                .unwrap()
        );
        assert!(matches!(
            mask.render_with_budget(&tags, &FileContext::default(), limit - 1),
            Err(BoundedRenderError::TooLarge)
        ));
    }
}

struct Observer;
static LIVE: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
static PEAK: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
fn grew(bytes: usize) {
    use std::sync::atomic::Ordering;
    let live = LIVE.fetch_add(bytes, Ordering::Relaxed) + bytes;
    PEAK.fetch_max(live, Ordering::Relaxed);
}
// SAFETY: delegate every pointer/layout unchanged to System; counters only
// observe successful allocation sizes and do not affect allocation data.
unsafe impl std::alloc::GlobalAlloc for Observer {
    unsafe fn alloc(&self, l: std::alloc::Layout) -> *mut u8 {
        let p = unsafe { std::alloc::GlobalAlloc::alloc(&std::alloc::System, l) };
        if !p.is_null() {
            grew(l.size());
        }
        p
    }
    unsafe fn alloc_zeroed(&self, l: std::alloc::Layout) -> *mut u8 {
        let p = unsafe { std::alloc::GlobalAlloc::alloc_zeroed(&std::alloc::System, l) };
        if !p.is_null() {
            grew(l.size());
        }
        p
    }
    unsafe fn dealloc(&self, p: *mut u8, l: std::alloc::Layout) {
        unsafe { std::alloc::GlobalAlloc::dealloc(&std::alloc::System, p, l) };
        LIVE.fetch_sub(l.size(), std::sync::atomic::Ordering::Relaxed);
    }
    unsafe fn realloc(&self, p: *mut u8, l: std::alloc::Layout, size: usize) -> *mut u8 {
        let p = unsafe { std::alloc::GlobalAlloc::realloc(&std::alloc::System, p, l, size) };
        if !p.is_null() {
            if size >= l.size() {
                grew(size - l.size());
            } else {
                LIVE.fetch_sub(l.size() - size, std::sync::atomic::Ordering::Relaxed);
            }
        }
        p
    }
}
#[global_allocator]
static ALLOCATOR: Observer = Observer;
fn isolated(name: &str) -> bool {
    if std::env::var("TAGREX_MASK_WORK_CASE").as_deref() == Ok(name) {
        return false;
    }
    let output = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", name, "--nocapture"])
        .env("TAGREX_MASK_WORK_CASE", name)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    true
}

#[test]
fn actual_function_family_allocations_fit_the_ast_work_allowance() {
    if isolated("actual_function_family_allocations_fit_the_ast_work_allowance") {
        return;
    }
    use std::sync::atomic::Ordering;
    let tags: TagMap = [
        (TagField::Title, "İ tails 尾 / ; ".repeat(4000)),
        (TagField::Artist, "The Artist".into()),
    ]
    .into();
    let patterns = [
        "$lower(%title%)",
        "$upper(%title%)",
        "$caps(%title%)",
        "$caps2(%title%)",
        "$left(%title%,100)",
        "$right(%title%,100)",
        "$substr(%title%,2,100)",
        "$trim(%title%)",
        "$trimleft(%title%)",
        "$trimright(%title%)",
        "$padleft(%title%,10)",
        "$padright(%title%,10)",
        "$num(7,8)",
        "$len(%title%)",
        "$strpos(%title%,尾)",
        "$replace(%title%,尾,x)",
        "$insert(%title%,tail,3)",
        "$reverse(%title%)",
        "$getpart(%title%,' ',2)",
        "$stripprefix(%artist%,The)",
        "$swapprefix(%artist%,The)",
        "$cutmix(%title%)",
        "$if(%title%,%title%,x)",
        "$if2(%title%,x)",
        "$equal(%title%,x)",
        "$nequal(%title%,x)",
        "$and(%title%,x)",
        "$or(%title%,x)",
        "$not(%title%)",
        "$greater(3,2)",
        "$longer(%title%,x)",
        "$isnumber(12)",
        "$in(尾,%title%)",
        "$add(1,2,3)",
        "$sub(1,2)",
        "$mul(2,3,4)",
        "$div(3,2)",
        "$mod(3,2)",
        "$min(1,2)",
        "$max(1,2)",
        "$round(2.2,1)",
        "$left(%title%,%title%)",
        "[$upper(%title%)]",
        "$if($and(%title%,%artist%),$upper(%title%),$lower(%title%))",
    ];
    let mut checked = 0;
    for pattern in patterns {
        let mask =
            Mask::parse_for_render(pattern).unwrap_or_else(|error| panic!("{pattern}: {error}"));
        let work = mask
            .render_work_bytes(&tags, &FileContext::default())
            .unwrap();
        let before = LIVE.load(Ordering::Relaxed);
        PEAK.store(before, Ordering::Relaxed);
        let result = mask.render_value_with_budget(&tags, &FileContext::default(), work);
        let peak = PEAK.load(Ordering::Relaxed).saturating_sub(before);
        assert!(peak <= work, "{pattern}: actualpeak{peak} > bound{work}");
        let legacy = mask.render_value_with(&tags, &FileContext::default());
        match (result, legacy) {
            (Ok(a), Ok(b)) => assert_eq!(a, b),
            (
                Err(BoundedRenderError::Mask(MaskError::BadArgument(a))),
                Err(MaskError::BadArgument(b)),
            ) => {
                if b.len() > 1024 {
                    assert!(a.len() <= 1027);
                    assert!(b.starts_with(a.trim_end_matches('…')));
                } else {
                    assert_eq!(a, b);
                }
            }
            (Err(BoundedRenderError::Mask(a)), Err(b)) => assert_eq!(a.to_string(), b.to_string()),
            other => panic!("{pattern}: {other:?}"),
        }
        checked += 1;
    }
    assert!(
        checked == patterns.len(),
        "fixture must cover the documented function families, got{checked}"
    );
}

#[test]
fn structural_argument_bomb_refuses_before_allocating_argument_buffers() {
    if isolated("structural_argument_bomb_refuses_before_allocating_argument_buffers") {
        return;
    }
    use std::sync::atomic::Ordering;
    let tags: TagMap = [(TagField::Comment, "x".repeat(1024 * 1024))].into();
    let mask =
        Mask::parse_for_render(&format!("$and({})", vec!["%comment%"; 100].join(","))).unwrap();
    assert!(
        mask.render_work_bytes(&tags, &FileContext::default())
            .unwrap()
            > 64 * 1024 * 1024
    );
    let before = LIVE.load(Ordering::Relaxed);
    PEAK.store(before, Ordering::Relaxed);
    assert!(matches!(
        mask.render_value_with_budget(&tags, &FileContext::default(), 64 * 1024 * 1024),
        Err(BoundedRenderError::TooLarge)
    ));
    assert!(PEAK.load(Ordering::Relaxed).saturating_sub(before) < 64 * 1024);
}
