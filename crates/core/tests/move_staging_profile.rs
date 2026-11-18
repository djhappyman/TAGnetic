//! Real persistent-journal timing decomposition, never a throughput pass target.
use std::{
    cell::Cell,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};
use tagrex_core::{
    journal::{AppliedBatch, BatchId, JournalError, MoveStage, SqliteJournal, UndoJournal},
    plan::{ChangePlan, Executor, FileChange},
};
#[derive(Clone, Copy, Default, Debug)]
struct Timing {
    calls: usize,
    elapsed: Duration,
}
struct Timed {
    inner: SqliteJournal,
    stages: Cell<Timing>,
    other: Cell<Timing>,
}
fn measure<T>(clock: &Cell<Timing>, work: impl FnOnce() -> T) -> T {
    let start = Instant::now();
    let value = work();
    let previous = clock.get();
    clock.set(Timing {
        calls: previous.calls + 1,
        elapsed: previous.elapsed + start.elapsed(),
    });
    value
}
impl UndoJournal for Timed {
    fn prepare_move_stage(&mut self, id: BatchId, stage: &MoveStage) -> Result<(), JournalError> {
        measure(&self.stages, || self.inner.prepare_move_stage(id, stage))
    }
    fn move_stages(&self, id: BatchId) -> Result<Vec<MoveStage>, JournalError> {
        measure(&self.other, || self.inner.move_stages(id))
    }
    fn prepare_copy_removal(
        &mut self,
        id: BatchId,
        target: &Path,
        undo: &Path,
    ) -> Result<(), JournalError> {
        measure(&self.other, || {
            self.inner.prepare_copy_removal(id, target, undo)
        })
    }
    fn record(&mut self, batch: &AppliedBatch) -> Result<BatchId, JournalError> {
        measure(&self.other, || self.inner.record(batch))
    }
    fn batches(&self) -> Result<Vec<AppliedBatch>, JournalError> {
        measure(&self.other, || self.inner.batches())
    }
    fn batch(&self, id: BatchId) -> Result<Option<AppliedBatch>, JournalError> {
        measure(&self.other, || self.inner.batch(id))
    }
    fn finish(&mut self, id: BatchId, batch: &AppliedBatch) -> Result<(), JournalError> {
        measure(&self.other, || self.inner.finish(id, batch))
    }
    fn retain_interrupted(
        &mut self,
        id: BatchId,
        batch: &AppliedBatch,
    ) -> Result<(), JournalError> {
        measure(&self.other, || self.inner.retain_interrupted(id, batch))
    }
    fn rollback(&mut self, id: BatchId) -> Result<(), JournalError> {
        measure(&self.other, || self.inner.rollback(id))
    }
}
struct Fixture(PathBuf);
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
fn wave() -> Vec<u8> {
    let mut out = b"RIFF".to_vec();
    out.extend(38u32.to_le_bytes());
    out.extend(b"WAVEfmt ");
    out.extend(16u32.to_le_bytes());
    out.extend(1u16.to_le_bytes());
    out.extend(1u16.to_le_bytes());
    out.extend(8000u32.to_le_bytes());
    out.extend(16000u32.to_le_bytes());
    out.extend(2u16.to_le_bytes());
    out.extend(16u16.to_le_bytes());
    out.extend(b"data");
    out.extend(2u32.to_le_bytes());
    out.extend([0, 0]);
    out
}
#[test]
#[ignore = "Manual actual SQLite intent timing vs all other move-staging work"]
fn actual_journal_and_directory_layout_costs() {
    use sha2::{Digest, Sha256};
    let hex = |bytes: &[u8]| {
        Sha256::digest(bytes)
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>()
    };
    eprintln!(
        "MOVE_PROFILE_SOURCE plan={} journal={} paths={}",
        hex(include_bytes!("../src/plan.rs")),
        hex(include_bytes!("../src/journal.rs")),
        hex(include_bytes!("../src/plan_paths.rs"))
    );
    for fanout in [false, true] {
        let root = std::env::temp_dir().join(format!(
            "tagrex-stage-profile-{}-{fanout}",
            std::process::id()
        ));
        std::fs::create_dir(&root).unwrap();
        let fixture = Fixture(root);
        let library = fixture.0.join("library");
        std::fs::create_dir(&library).unwrap();
        let original = wave();
        let mut changes = Vec::new();
        for index in 0..1000 {
            let parent = library.join(if fanout {
                format!("album-{index}")
            } else {
                "album".into()
            });
            std::fs::create_dir_all(&parent).unwrap();
            let source = parent.join(format!("{index}.wav"));
            std::fs::write(&source, &original).unwrap();
            changes.push(FileChange {
                path: source,
                rename_to: Some(library.join(format!("moved/{index}.wav"))),
                ..Default::default()
            });
        }
        let mut journal = Timed {
            inner: SqliteJournal::open(&fixture.0.join("journal.sqlite")).unwrap(),
            stages: Cell::default(),
            other: Cell::default(),
        };
        let plan = ChangePlan {
            changes,
            ..Default::default()
        };
        for cache in ["data-advised", "warm-repeat"] {
            if cache == "data-advised" {
                for change in &plan.changes {
                    let file = std::fs::File::open(&change.path).unwrap();
                    file.sync_all().unwrap();
                    #[cfg(target_os = "linux")]
                    rustix::fs::fadvise(&file, 0, None, rustix::fs::Advice::DontNeed).unwrap();
                }
            }
            journal.stages.set(Timing::default());
            journal.other.set(Timing::default());
            let extra_fds = Cell::new(0usize);
            #[cfg(target_os = "linux")]
            let baseline_fds = std::fs::read_dir("/proc/self/fd").unwrap().count();
            let mut progress = |event: &tagrex_core::plan::ApplyProgress| {
                let _ = event;
                #[cfg(target_os = "linux")]
                if event.done.is_multiple_of(128) {
                    extra_fds.set(
                        extra_fds.get().max(
                            std::fs::read_dir("/proc/self/fd")
                                .unwrap()
                                .count()
                                .saturating_sub(baseline_fds),
                        ),
                    );
                }
            };
            let start = Instant::now();
            let batch = Executor::apply_with(
                &plan,
                &mut journal,
                std::slice::from_ref(&library),
                &mut progress,
                &std::sync::atomic::AtomicBool::new(false),
            )
            .unwrap();
            let apply = start.elapsed();
            let apply_stages = journal.stages.replace(Timing::default());
            let apply_other = journal.other.replace(Timing::default());
            let start = Instant::now();
            Executor::undo_with(
                &mut journal,
                batch.id,
                std::slice::from_ref(&library),
                &mut progress,
            )
            .unwrap();
            let undo = start.elapsed();
            eprintln!("MOVE_STAGE_TIMING cache={cache} metadata_cache=uncontrolled fanout={fanout} n=1000 apply={apply:?} intent_sql={apply_stages:?} other_sql={apply_other:?} undo={undo:?} intent_sql={:?} other_sql={:?} extra_callback_fds={}",journal.stages.get(),journal.other.get(),extra_fds.get());
            assert!(
                extra_fds.get() < 64,
                "bounded progress-observed descriptor budget"
            );
            assert!(journal.batches().unwrap().is_empty());
            for change in &plan.changes {
                assert_eq!(std::fs::read(&change.path).unwrap(), original);
                assert!(!change.rename_to.as_ref().unwrap().exists());
            }
            assert!(!library.join("moved").exists());
        }
    }
}
