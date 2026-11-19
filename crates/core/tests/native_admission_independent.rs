#![cfg(target_os = "linux")]
//! Independent resource-pressure and callback probes against disposable media.
use std::{
    fs,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc,
    },
    time::{Duration, Instant},
};
use tagrex_core::{
    journal::{
        native::{Entries, Intent, NativeJournal},
        AppliedBatch, BatchId, JournalError, SqliteJournal, UndoJournal,
    },
    media_mutation::{BoundMedia, Phase, Record, Store},
    model::TagField,
    plan::{ApplyStep, ChangePlan, Executor, FieldChange, FileChange},
    resources::{Budget, Charge},
};

struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let p = std::env::temp_dir().join(format!(
            "tagrex-native-admission-independent-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&p).unwrap();
        Self(p)
    }
    fn file(&self, name: &str) -> PathBuf {
        let p = self.0.join(name);
        fs::write(&p, flac()).unwrap();
        p
    }
    fn journal(&self, name: &str) -> SqliteJournal {
        SqliteJournal::open(&self.0.join(name)).unwrap()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
fn flac() -> Vec<u8> {
    let mut b = b"fLaC\0\0\0\x22".to_vec();
    let mut info = [0; 34];
    info[..4].copy_from_slice(&[0x10, 0, 0x10, 0]);
    info[10..18].copy_from_slice(&[0x0a, 0xc4, 0x42, 0xf0, 0, 0, 0, 0]);
    b.extend(info);
    let mut c = 3u32.to_le_bytes().to_vec();
    c.extend(b"own");
    let fields = [
        "TITLE=Before",
        "ARTIST=A; literal",
        "ARTIST=",
        "ARTIST=尾",
        "CUSTOM=unrelated",
    ];
    c.extend((fields.len() as u32).to_le_bytes());
    for s in fields {
        c.extend((s.len() as u32).to_le_bytes());
        c.extend(s.as_bytes());
    }
    let n = c.len();
    b.extend([4, (n >> 16) as u8, (n >> 8) as u8, n as u8]);
    b.extend(c);
    b.extend([0x82, 0, 0, 8]);
    b.extend(b"OWN!\0\xff\x01\x02");
    b.extend([
        0xff, 0xf8, 0xc9, 0x18, 0, 0xc2, 0, 0, 0, 0, 0, 0, 0xb8, 0xee,
    ]);
    b
}
fn plan(paths: &[PathBuf]) -> ChangePlan {
    ChangePlan {
        changes: paths
            .iter()
            .map(|p| FileChange {
                path: p.clone(),
                tag_changes: vec![FieldChange {
                    field: TagField::Title,
                    old: Some("Before".into()),
                    new: Some("Changed 🎵".into()),
                }],
                ..Default::default()
            })
            .collect(),
        ..Default::default()
    }
}
fn title(path: &Path) -> String {
    let b = fs::read(path).unwrap();
    let mut at = 4;
    loop {
        let flag = b[at];
        let n = ((b[at + 1] as usize) << 16) | ((b[at + 2] as usize) << 8) | b[at + 3] as usize;
        at += 4;
        let c = &b[at..at + n];
        at += n;
        if flag & 127 == 4 {
            let num = |i| u32::from_le_bytes(c[i..i + 4].try_into().unwrap()) as usize;
            let mut p = 4 + num(0);
            let count = num(p);
            p += 4;
            for _ in 0..count {
                let n = num(p);
                p += 4;
                let s = std::str::from_utf8(&c[p..p + n]).unwrap();
                p += n;
                if let Some(v) = s.strip_prefix("TITLE=") {
                    return v.into();
                }
            }
        }
        assert_eq!(flag & 128, 0);
    }
}

// One controlled sequence owns deliberate process-budget pressure. It does not
// serialize production or any pre-existing test suite.
#[test]
fn budget_contention_waits_before_effects_and_retained_prefix_can_be_recovered() {
    let f = Fixture::new();
    let a = f.file("a.flac");
    let b = f.file("b.flac");
    let original = fs::read(&a).unwrap();
    let roots = std::slice::from_ref(&f.0);
    let budget = Budget::process();
    // A real public Store retains one working job while an unrelated owner
    // retains the remaining seven; neither is governed by the private IO gate.
    let direct = f.file("concurrent-direct.flac");
    let direct_store = Store::open(
        &f.0.join("concurrent-store"),
        Default::default(),
        budget.clone(),
    )
    .unwrap();
    let prepared = direct_store
        .prepare_file(
            BoundMedia::open(&direct).unwrap(),
            "flac",
            0,
            &mut |_: &Record| Ok(()),
            &mut (),
            |_, _| Ok(()),
        )
        .unwrap();
    assert_eq!(budget.usage().0.jobs, 1);
    // Capture is cancellable while other participating owners have all jobs.
    let held = budget
        .try_acquire(Charge {
            jobs: 7,
            ..Default::default()
        })
        .unwrap();
    let cancel = Arc::new(AtomicBool::new(false));
    let flag = cancel.clone();
    let worker = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(40));
        flag.store(true, Ordering::SeqCst);
    });
    let mut j = f.journal("cancel.sqlite");
    let started = Instant::now();
    let result = Executor::apply_with(
        &plan(std::slice::from_ref(&a)),
        &mut j,
        roots,
        &mut |_| {},
        &cancel,
    );
    worker.join().unwrap();
    assert!(result.is_err());
    assert!(started.elapsed() < Duration::from_secs(2));
    drop(held);
    drop(prepared);
    assert!(direct_store.records().unwrap().is_empty());
    assert_eq!(fs::read(&direct).unwrap(), original);
    drop(direct_store);
    assert_eq!(fs::read(&a).unwrap(), original);
    assert!(j.batches().unwrap().is_empty());
    // Initial Store admission can wait, but has not invoked its editor yet.
    let mut release = None;
    let mut entered = false;
    let p = plan(std::slice::from_ref(&a));
    let batch = Executor::apply_with(
        &p,
        &mut j,
        roots,
        &mut |progress| {
            if !entered && progress.done == 1 && progress.completed.is_none() {
                entered = true;
                let held = budget
                    .try_acquire(Charge {
                        jobs: 8,
                        ..Default::default()
                    })
                    .unwrap();
                release = Some(std::thread::spawn(move || {
                    std::thread::sleep(Duration::from_millis(60));
                    drop(held);
                }));
            }
        },
        &AtomicBool::new(false),
    )
    .unwrap();
    release.take().unwrap().join().unwrap();
    assert!(batch.native_tags);
    assert_eq!(title(&a), "Changed 🎵");
    Executor::undo(&mut j, batch.id, roots).unwrap();
    assert_eq!(fs::read(&a).unwrap(), original);
    // A persistent external budget owner may deny rollback too. The operation
    // must report failure, retain interrupted history, and later undo exactly.
    let mut held = None;
    let p = plan(&[a.clone(), b.clone()]);
    let started = Instant::now();
    let result = Executor::apply_with(
        &p,
        &mut j,
        roots,
        &mut |progress| {
            if progress.completed == Some(ApplyStep::Tags) && progress.path == a {
                held = Some(
                    budget
                        .try_acquire(Charge {
                            jobs: 8,
                            ..Default::default()
                        })
                        .unwrap(),
                );
            }
        },
        &AtomicBool::new(false),
    );
    assert!(result.is_err());
    assert!(started.elapsed() < Duration::from_secs(5));
    assert_eq!(title(&a), "Changed 🎵");
    assert_eq!(fs::read(&b).unwrap(), original);
    drop(held.take());
    let batches = j.batches().unwrap();
    assert_eq!(batches.len(), 1);
    assert!(batches[0].interrupted);
    Executor::undo(&mut j, batches[0].id, roots).unwrap();
    assert_eq!(fs::read(&a).unwrap(), original);
    assert_eq!(fs::read(&b).unwrap(), original);
    assert!(j.batches().unwrap().is_empty());
    assert_eq!(budget.usage().0.transient, 0);
    assert_eq!(budget.usage().0.jobs, 0);
    // Public Store APIs deliberately remain nonblocking under the same budget.
    let store = Store::open(
        &f.0.join("direct-store"),
        Default::default(),
        budget.clone(),
    )
    .unwrap();
    let held = budget
        .try_acquire(Charge {
            jobs: 8,
            ..Default::default()
        })
        .unwrap();
    let mut called = false;
    let started = Instant::now();
    let result = store.prepare_file(
        BoundMedia::open(&a).unwrap(),
        "flac",
        0,
        &mut |_: &Record| Ok(()),
        &mut (),
        |_, _| {
            called = true;
            Ok(())
        },
    );
    assert!(result.is_err());
    assert!(!called);
    assert!(started.elapsed() < Duration::from_millis(200));
    drop(held);
    assert!(store.records().unwrap().is_empty());
    assert_eq!(fs::read(&a).unwrap(), original);
    drop(store);
    drop(j);
    assert_eq!(budget.usage().0, Charge::default());
    sink_reentry_refuses_before_callbacks_and_panic_releases_compound_admission();
    public_progress_does_not_hold_private_admission();
    assert_eq!(budget.usage().0, Charge::default());
}

struct SinkJournal {
    inner: SqliteJournal,
    peer: SqliteJournal,
    peer_plan: ChangePlan,
    roots: Vec<PathBuf>,
    undo_peer: SqliteJournal,
    undo_peer_id: BatchId,
    fired: bool,
    panic: bool,
}
impl UndoJournal for SinkJournal {
    fn native_tags(&mut self) -> Option<&mut dyn NativeJournal> {
        Some(self)
    }
    fn record(&mut self, b: &AppliedBatch) -> Result<BatchId, JournalError> {
        self.inner.record(b)
    }
    fn batches(&self) -> Result<Vec<AppliedBatch>, JournalError> {
        self.inner.batches()
    }
    fn batch(&self, id: BatchId) -> Result<Option<AppliedBatch>, JournalError> {
        self.inner.batch(id)
    }
    fn finish(&mut self, id: BatchId, b: &AppliedBatch) -> Result<(), JournalError> {
        self.inner.finish(id, b)
    }
    fn rollback(&mut self, id: BatchId) -> Result<(), JournalError> {
        self.inner.rollback(id)
    }
}
impl NativeJournal for SinkJournal {
    fn store(&mut self) -> Result<Store, JournalError> {
        NativeJournal::store(&mut self.inner)
    }
    fn record_native(&mut self, b: &AppliedBatch, i: &[Intent]) -> Result<BatchId, JournalError> {
        NativeJournal::record_native(&mut self.inner, b, i)
    }
    fn entries(&self, b: &AppliedBatch) -> Result<Entries, JournalError> {
        NativeJournal::entries(&self.inner, b)
    }
    fn begin(&mut self, b: &AppliedBatch, i: usize, o: u8) -> Result<String, JournalError> {
        NativeJournal::begin(&mut self.inner, b, i, o)
    }
    fn checkpoint(
        &mut self,
        b: &AppliedBatch,
        pos: (usize, u8),
        r: &Record,
        e: &Intent,
        receipts: Option<&[u8]>,
        done: bool,
    ) -> Result<(), JournalError> {
        if r.phase() == Phase::Prepared && !self.fired {
            self.fired = true;
            let mut callbacks = 0;
            let start = Instant::now();
            let result = Executor::apply_with(
                &self.peer_plan,
                &mut self.peer,
                &self.roots,
                &mut |_| callbacks += 1,
                &AtomicBool::new(false),
            );
            assert!(result.is_err(), "sink reentry must refuse");
            assert_eq!(callbacks, 0, "reentry must refuse before callbacks");
            assert!(start.elapsed() < Duration::from_millis(200));
            assert!(self.peer.batches()?.is_empty());
            let mut undo_callbacks = 0;
            let result = Executor::undo_with(
                &mut self.undo_peer,
                self.undo_peer_id,
                &self.roots,
                &mut |_| {
                    undo_callbacks += 1;
                },
            );
            assert!(result.is_err(), "sink Undo reentry must refuse");
            assert_eq!(
                undo_callbacks, 0,
                "Undo reentry must refuse before callbacks"
            );
            assert!(self.undo_peer.batch(self.undo_peer_id)?.is_some());
            if self.panic {
                panic!("independent sink panic");
            }
        }
        NativeJournal::checkpoint(&mut self.inner, b, pos, r, e, receipts, done)
    }
    fn confirm(
        &self,
        b: &AppliedBatch,
        p: (usize, u8),
        r: &Record,
        e: &Intent,
        receipts: &[u8],
    ) -> Result<(), JournalError> {
        NativeJournal::confirm(&self.inner, b, p, r, e, receipts)
    }
    fn finish_native(&mut self, b: &AppliedBatch) -> Result<(), JournalError> {
        NativeJournal::finish_native(&mut self.inner, b)
    }
    fn retain_native(&mut self, b: &AppliedBatch) -> Result<(), JournalError> {
        NativeJournal::retain_native(&mut self.inner, b)
    }
    fn remove_native(&mut self, id: BatchId) -> Result<(), JournalError> {
        NativeJournal::remove_native(&mut self.inner, id)
    }
}
fn sink_reentry_refuses_before_callbacks_and_panic_releases_compound_admission() {
    for panic in [false, true] {
        let f = Fixture::new();
        let a = f.file("outer.flac");
        let peer = f.file("peer.flac");
        let original = fs::read(&a).unwrap();
        let roots = vec![f.0.clone()];
        let undo_target = f.file("undo-peer.flac");
        let mut undo_peer = f.journal("undo-peer.sqlite");
        let undo_batch = Executor::apply(
            &plan(std::slice::from_ref(&undo_target)),
            &mut undo_peer,
            &roots,
        )
        .unwrap();
        let undo_baseline = fs::read(&undo_target).unwrap();
        let mut j = SinkJournal {
            inner: f.journal("outer.sqlite"),
            peer: f.journal("peer.sqlite"),
            peer_plan: plan(std::slice::from_ref(&peer)),
            roots: roots.clone(),
            undo_peer,
            undo_peer_id: undo_batch.id,
            fired: false,
            panic,
        };
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            Executor::apply(&plan(std::slice::from_ref(&a)), &mut j, &roots)
        }));
        assert!(j.fired);
        assert_eq!(fs::read(&peer).unwrap(), original);
        assert_eq!(fs::read(&undo_target).unwrap(), undo_baseline);
        Executor::undo(&mut j.undo_peer, undo_batch.id, &roots).unwrap();
        assert_eq!(fs::read(&undo_target).unwrap(), original);
        if panic {
            assert!(result.is_err());
            assert_eq!(fs::read(&a).unwrap(), original);
        } else {
            let batch = result.unwrap().unwrap();
            Executor::undo(&mut j, batch.id, &roots).unwrap();
            assert_eq!(fs::read(&a).unwrap(), original);
        }
        // Run a new complete mutation on this same thread; a leaked TLS/phase
        // guard or Store working lease would refuse or stall this operation.
        let batch = Executor::apply(&j.peer_plan, &mut j.peer, &roots).unwrap();
        Executor::undo(&mut j.peer, batch.id, &roots).unwrap();
        assert_eq!(fs::read(&peer).unwrap(), original);
    }
}

fn public_progress_does_not_hold_private_admission() {
    let f = Fixture::new();
    let outer = f.file("outer.flac");
    let peer = f.file("peer.flac");
    let original = fs::read(&outer).unwrap();
    let roots = std::slice::from_ref(&f.0);
    let mut outer_journal = f.journal("outer.sqlite");
    let mut peer_journal = f.journal("peer.sqlite");
    let mut called = false;
    let batch = Executor::apply_with(
        &plan(std::slice::from_ref(&outer)),
        &mut outer_journal,
        roots,
        &mut |progress| {
            if progress.done == 1 && progress.completed.is_none() && !called {
                called = true;
                let peer_batch =
                    Executor::apply(&plan(std::slice::from_ref(&peer)), &mut peer_journal, roots)
                        .unwrap();
                assert!(peer_batch.native_tags);
                Executor::undo(&mut peer_journal, peer_batch.id, roots).unwrap();
                assert_eq!(fs::read(&peer).unwrap(), original);
            }
        },
        &AtomicBool::new(false),
    )
    .unwrap();
    assert!(called);
    Executor::undo(&mut outer_journal, batch.id, roots).unwrap();
    assert_eq!(fs::read(&outer).unwrap(), original);
    assert!(outer_journal.batches().unwrap().is_empty());
    assert!(peer_journal.batches().unwrap().is_empty());
}
