//! The transactional change pipeline — the heart of TagRex.
//!
//! ```text
//! source of changes --> plan --> preview --> apply --> undo journal
//! ```
//!
//! Everything else (masks, transforms, online providers) is just a producer
//! of plans. **No module writes tags or renames files directly; all writes go
//! through [`Executor`].** This is an invariant, not a preference.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use thiserror::Error;

use crate::journal::{
    AppliedBatch, AppliedValue, BatchId, BlockFingerprint, CopyStamp, FileRevision, JournalError,
    MoveStage, NativeIdentity, UndoJournal,
};
use crate::model::{
    is_storable_custom_name_at, CoverArt, Id3v2Revision, TagBlockContent, TagBlockKind, TagEngine,
    TagField, TagIoError, TrackFile,
};

/// A change to a single tag field: `old` is what preview shows as "current",
/// `new` is what will be written. `None` means the field is absent/removed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FieldChange {
    pub field: TagField,
    pub old: Option<String>,
    pub new: Option<String>,
}

/// A change to the file's embedded images (#56). Both sides are the WHOLE set:
/// `old` is what the file carries and what undo writes back, `new` is what
/// replaces it, and an empty side means no images at all.
///
/// A snapshot rather than a list of per-image operations, because an embedded
/// picture has no stable identity to address. Adding, replacing, removing,
/// reordering and retyping all become the same change, its undo is trivially
/// the other side, and the staleness check stays one comparison.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct CoverChange {
    pub old: Vec<CoverArt>,
    pub new: Vec<CoverArt>,
}

/// A change to one whole tag block (#47, #205). Both sides are the WHOLE block,
/// the way [`CoverChange`]'s are the whole image set: `old` is what the file
/// carries and what undo writes back, `new` is what replaces it, and `None` on
/// either side means no block of that kind at all.
///
/// One shape for three operations, because they are the same write with
/// different ends: stripping a block is `new: None`, giving a file a block it
/// lacked is `old: None`, and rewriting one in place — a different ID3v2
/// revision, say — is both sides present. A conversion between two kinds is two
/// of these, one per kind, which is why this is per-kind rather than a
/// from/to pair.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BlockChange {
    pub kind: TagBlockKind,
    /// Which ID3v2 revision to write, for the ID3v2 block only. `None` follows
    /// the app-wide preference (#79), which is what a removal's undo and an
    /// ordinary write both want; a conversion sets it deliberately.
    pub revision: Option<Id3v2Revision>,
    /// The revision the block was in, so undo puts that back rather than
    /// whatever the app-wide preference happens to say. A revision is part of
    /// what the block *was*, and an undo that quietly leaves a 2.3 file as 2.4
    /// has not undone anything the user can see.
    pub old_revision: Option<Id3v2Revision>,
    /// The ID3v2 block as it was, in bytes, so undo puts it back frame for
    /// frame instead of rebuilding it from `old` (#206); likewise the Vorbis
    /// comment block, comment for comment (#669). `None` for every other kind,
    /// and for a change that creates a block rather than replacing one.
    ///
    /// Pictures are not in here — they are in `old` — so restoring takes both.
    pub old_bytes: Option<Vec<u8>>,
    pub old: Option<TagBlockContent>,
    pub new: Option<TagBlockContent>,
    /// Whether this rewrites the Latin-1 text of the ID3v2 block in Unicode
    /// (#698). The block says the same before and after, so both sides are the
    /// same content, and only `old_bytes` can put the old spelling back.
    pub text_to_unicode: bool,
}

impl BlockChange {
    /// Strip a block, keeping what it held so undo can rebuild it.
    pub fn removal(kind: TagBlockKind, removed: TagBlockContent) -> Self {
        Self {
            kind,
            revision: None,
            old_revision: None,
            old_bytes: None,
            old: Some(removed),
            new: None,
            text_to_unicode: false,
        }
    }

    /// Whether this only changes which revision the ID3v2 block is written in,
    /// leaving its contents alone — the 2.3 <-> 2.4 switch (#205).
    ///
    /// Worth telling apart because it can be done without rebuilding the block,
    /// and so without losing the frames a rebuild cannot carry.
    fn is_revision_only(&self) -> bool {
        self.kind == TagBlockKind::Id3v2
            && !self.text_to_unicode
            && self.old.is_some()
            && self.old == self.new
    }

    /// Whether undoing this change would put the block back whole.
    ///
    /// True when the bytes of the old block were kept (#206), which makes the
    /// restore frame-for-frame whatever the kind; otherwise it falls back to
    /// what a rebuild from the model's view can manage, which is ID3v1 alone —
    /// see [`TagBlockContent::exact`]. The preview says so before the change is
    /// staged, which is the only warning the user gets.
    pub fn exact(&self) -> bool {
        self.old_bytes.is_some() || TagBlockContent::exact(self.kind)
    }
}

/// All changes planned for one file.
#[derive(Debug, Clone, Default)]
pub struct FileChange {
    pub path: PathBuf,
    pub tag_changes: Vec<FieldChange>,
    /// Planned cover-art change, if any.
    pub cover_change: Option<CoverChange>,
    /// Whole-block changes for this file (#47, #205). A list because a file can
    /// carry several blocks and one operation can touch more than one of them —
    /// a conversion writes the target block and drops the source in the same
    /// change, and undoing half of that would be worse than not offering it.
    pub block_changes: Vec<BlockChange>,
    /// Planned rename, if any.
    pub rename_to: Option<PathBuf>,
    /// Whether `rename_to` is a COPY rather than a move (#153). The source is
    /// left exactly as it is — which is what makes copy the safe half of writing
    /// outside the library — so a copy change's tag and cover changes are
    /// written to the *copy*, never to the original. Undo removes the copy.
    pub copy: bool,
    /// Sidecar files that travel with this file's rename/move (#58): each a
    /// `(from, to)` pair. Detected at preview time and journaled with the plan so
    /// they move together and are restored together on undo. Empty when the file
    /// isn't renamed or the feature is off.
    pub sidecar_renames: Vec<(PathBuf, PathBuf)>,
}

/// A complete, previewable plan of changes over a set of files.
#[derive(Debug, Clone, Default)]
pub struct ChangePlan {
    /// Human-readable summary, inherited from the [`PlanSource`] that built
    /// the plan, and carried into the journal so history shows what a batch
    /// was (see [`AppliedBatch::description`]).
    pub description: String,
    /// The same description as a code and its values, encoded by the layer that
    /// knows what the encoding means (#268).
    ///
    /// Opaque here on purpose: the journal has to persist it so History can be
    /// rendered in whatever language is chosen *later*, but nothing in this
    /// crate reads it, and giving the core a serialization format for it would
    /// buy a dependency for a string it only ever stores and hands back.
    /// `None` for a plan built before this existed.
    pub message: Option<String>,
    pub changes: Vec<FileChange>,
    /// Whether to remove the folders a move empties (#153). Off by default, so
    /// a plain rename never prunes anything; the reorganize operation turns it
    /// on. Only directories this batch actually emptied are removed, and undo
    /// puts them back — see [`AppliedBatch::removed_dirs`].
    pub prune_empty_dirs: bool,
}

impl ChangePlan {
    pub fn is_empty(&self) -> bool {
        self.changes.is_empty()
    }

    pub fn file_count(&self) -> usize {
        self.changes.len()
    }
}

/// Anything that can produce a plan: a mask rename, a filename-to-tags
/// import, a provider release import, a transform chain over selected fields.
pub trait PlanSource {
    /// Human-readable description for the preview header and the journal.
    fn describe(&self) -> String;

    /// Build a plan against the current state of `files`. Must be pure:
    /// no I/O side effects, no writes.
    fn build_plan(&self, files: &[TrackFile]) -> Result<ChangePlan, PlanError>;
}

/// The only component allowed to write. Applies a plan and records the
/// applied batch in the journal so it can be rolled back — including after
/// an application restart.
///
/// Within a single file, tag writes happen before the rename: once a file is
/// renamed, the plan's path no longer points at it, so keeping the move last
/// means every tag write uses the original path and a failure mid-move leaves
/// the file at its old path with new tags — a recoverable state. Tag writing
/// and renaming are still separate *user* operations, each with its own tab in
/// the UI as taggers conventionally present them; a plan can carry either or
/// both.
pub struct Executor;

#[cfg(target_os = "linux")]
mod native_flac;
#[path = "plan_paths.rs"]
mod pinned_paths;

#[cfg(test)]
#[path = "plan_copy_removal_tests.rs"]
mod copy_removal_tests;
#[cfg(test)]
#[path = "plan_descriptor_io_tests.rs"]
mod descriptor_io_tests;
#[cfg(test)]
#[path = "plan_move_metrics.rs"]
pub(crate) mod move_metrics;
#[cfg(test)]
#[path = "plan_move_staging_tests.rs"]
mod move_staging_tests;

impl Executor {
    /// Apply a plan's tag changes and renames to disk, then record the batch
    /// so it can be rolled back.
    ///
    /// Every source path and every rename target is confined to `allowed_roots`:
    /// a plan touching a path that resolves outside all of them is rejected
    /// wholesale before anything is written. The entire plan is validated up
    /// front (root containment + staleness + rename collisions) so a bad file
    /// cannot leave the batch half-applied. A write that fails at run time
    /// anyway (a locked file, a share dropping out) is rolled back, best-effort,
    /// before the error is returned, so the library is left as it was (#441).
    ///
    /// There is more than one root because a reorganize can file tracks into a
    /// folder outside the open library (#153). That second root is not inferred
    /// from the plan — which would make the containment check circular — it is a
    /// destination the user picked, handed down from the caller and recorded on
    /// the batch so undo is still authorized for it after a restart.
    pub fn apply(
        plan: &ChangePlan,
        journal: &mut dyn UndoJournal,
        allowed_roots: &[PathBuf],
    ) -> Result<AppliedBatch, PlanError> {
        Self::apply_with(
            plan,
            journal,
            allowed_roots,
            &mut |_| {},
            &AtomicBool::new(false),
        )
    }

    /// [`Executor::apply`] that reports how far it has got and can be stopped
    /// (#437). `progress` is called before each step of the write phase;
    /// `cancel` is read between steps, never inside one, so a cancelled batch
    /// cannot leave a half-written file behind. A cancel is rolled back exactly
    /// like a failure and comes back as [`PlanError::Cancelled`], with nothing
    /// changed and nothing journaled. Once every file is written the batch is
    /// committed whatever the flag says.
    pub fn apply_with(
        plan: &ChangePlan,
        journal: &mut dyn UndoJournal,
        allowed_roots: &[PathBuf],
        progress: &mut dyn FnMut(&ApplyProgress),
        cancel: &AtomicBool,
    ) -> Result<AppliedBatch, PlanError> {
        if crate::model::recovery_active() {
            return Err(JournalError::NotRollbackable(
                "new user operations are refused during exact rollback".into(),
            )
            .into());
        }
        #[cfg(target_os = "linux")]
        if native_flac::private_phase_active() {
            return Err(JournalError::NotRollbackable(
                "native IO reentry is refused before callbacks".into(),
            )
            .into());
        }
        let _policy = crate::model::OperationPolicy::enter().ok_or_else(policy_changed)?;
        let rollback_policy = crate::model::RecoveryPolicy::capture().ok_or_else(policy_changed)?;
        ensure_operation_policy()?;
        ensure_unicode_plan(plan, allowed_roots)?;
        if plan.changes.iter().any(|change| {
            change.copy
                && (effective_rename(change).is_some() || !change.sidecar_renames.is_empty())
        }) && !journal.supports_copy_checkpoints()
        {
            return Err(JournalError::NotRollbackable(
                "journal does not support completed-copy checkpoints".into(),
            )
            .into());
        }
        ensure_operation_policy()?;
        let roots = canonical_roots(allowed_roots)?;
        let _paths = pinned_paths::Scope::enter(&roots, operation_paths(plan, &[]))
            .map_err(PlanError::Io)?;

        // Keep native move identities from before any public callback. A later
        // row's preflight callback cannot endorse a replacement of an earlier row.
        let move_proofs = capture_move_stamps(plan, cancel)?;
        #[cfg(target_os = "linux")]
        let native_capture = if journal.native_tags().is_some() {
            native_flac::capture(plan, &roots, cancel)?
        } else {
            None
        };

        ensure_operation_policy()?;

        // Pre-flight: validate the WHOLE plan before touching disk. Rename
        // targets are collected so two files can't be planned onto the same
        // destination.
        let total = plan.changes.len() + count_steps(plan);
        let mut planned_targets = HashSet::new();
        for (index, change) in plan.changes.iter().enumerate() {
            // Reading every file to check it is not stale is the long part of a
            // big apply on a slow disk, and it writes nothing, so a cancel here
            // has nothing to put back (#437).
            if cancel.load(Ordering::SeqCst) {
                return Err(PlanError::Cancelled);
            }
            ensure_within_roots(&change.path, &roots)?;
            let _step = pinned_paths::Step::enter(std::iter::once(change.path.clone()))
                .map_err(PlanError::Io)?;
            progress(&ApplyProgress {
                done: index,
                total,
                path: change.path.clone(),
                completed: None,
            });
            ensure_operation_policy()?;
            ensure_within_roots(&change.path, &roots)?;
            ensure_not_stale(change)?;
            ensure_storable_custom_names(change)?;
            if let Some(target) = effective_rename(change) {
                let canonical_target = resolve_target_within_roots(target, &roots)?;
                // The planned spelling, not the canonical one: canonicalizing a
                // name the file system folds can hand back the file's own.
                ensure_target_free(&change.path, target, change.copy)?;
                if !planned_targets.insert(canonical_target.clone()) {
                    return Err(PlanError::RenameCollision(canonical_target));
                }
            }
            // Sidecars (#58) obey the same rules: source inside a root, target
            // inside a root, and never overwrite an existing file or another
            // planned destination.
            for (from, to) in &change.sidecar_renames {
                ensure_within_roots(from, &roots)?;
                let canonical_target = resolve_target_within_roots(to, &roots)?;
                ensure_target_free(from, to, change.copy)?;
                if !planned_targets.insert(canonical_target.clone()) {
                    return Err(PlanError::RenameCollision(canonical_target));
                }
            }
        }

        #[cfg(target_os = "linux")]
        if let Some(captured) = &native_capture {
            native_flac::revalidate(plan, captured, cancel)?;
        }

        ensure_operation_policy()?;

        // Everything below touches disk. A failure part-way must not leave the
        // library half-applied with nothing in the journal for Undo to find
        // (#441), so what was done is tracked and put back before the error goes
        // up. Rollback is best-effort: it tries every step and keeps going.
        //
        // A rollback needs a process to run it, though. So the batch is in the
        // journal before the first write, marked interrupted (#659): should the
        // process stop part-way, the next start finds it there to undo. A
        // journal that cannot record it means nothing is written at all.
        let mut batch = AppliedBatch {
            native_tags: false,
            // Placeholder: the journal assigns the real id on record so ids
            // stay unique across restarts.
            id: BatchId(0),
            description: plan.description.clone(),
            message: plan.message.clone(),
            applied_at: now_unix_secs(),
            plan: plan.clone(),
            created_dirs: Vec::new(),
            // Write pruning intent before anything can remove a source parent.
            // A crash may leave some candidates unpruned; recreating an existing
            // authorized directory during Undo is harmless.
            removed_dirs: if plan.prune_empty_dirs {
                pruning_candidates(plan, &roots, &[])
            } else {
                Vec::new()
            },
            allowed_roots: roots.clone(),
            copy_stamps: move_proofs,
            applied_values: Vec::new(),
            block_fingerprints: capture_block_fingerprints(plan)?,
            interrupted: true,
            id3v2_revisions: id3v2_revisions(plan),
        };
        #[cfg(target_os = "linux")]
        if let Some(captured) = native_capture {
            let driver = journal.native_tags().ok_or_else(|| {
                JournalError::NotRollbackable("native journal capability disappeared".into())
            })?;
            ensure_operation_policy()?;
            return native_flac::apply(batch, captured, driver, progress, cancel);
        }
        let mut move_context = MoveContext::compile(BatchId(0), plan, &batch.copy_stamps, &roots)?;
        ensure_operation_policy()?;
        batch.id = journal.record(&batch)?;
        move_context.id = batch.id;
        let _move_context = MoveContextScope::activate(move_context);

        let mut written = Written::new(plan.changes.len());
        written.copy_stamps.extend(
            batch
                .copy_stamps
                .iter()
                .map(|stamp| (stamp.path.clone(), stamp.clone())),
        );
        written.block_baselines.extend(
            batch
                .block_fingerprints
                .iter()
                .map(|item| (item.path.clone(), item.before.clone())),
        );
        written.revisions.clone_from(&batch.id3v2_revisions);
        let mut steps = Steps::new(total, progress, cancel);
        steps.done = plan.changes.len();
        let phase = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            ensure_operation_policy()?;
            Self::write_phase(
                plan,
                &roots,
                &batch.removed_dirs,
                &mut batch.block_fingerprints,
                &mut written,
                &mut steps,
                journal,
                batch.id,
            )
        }));
        let removed_dirs = match phase {
            Ok(Ok(removed_dirs)) => removed_dirs,
            Ok(Err(error)) => {
                return Err(rollback_failed_apply(
                    plan,
                    &mut batch,
                    &mut written,
                    journal,
                    &rollback_policy,
                    error,
                    false,
                ))
            }
            Err(payload) => {
                let _ = rollback_failed_apply(
                    plan,
                    &mut batch,
                    &mut written,
                    journal,
                    &rollback_policy,
                    JournalError::Storage("Apply callback unwound".into()).into(),
                    false,
                );
                std::panic::resume_unwind(payload)
            }
        };
        batch.created_dirs = std::mem::take(&mut written.created_dirs);
        batch.removed_dirs = removed_dirs;
        batch.copy_stamps = written.copy_stamps.values().cloned().collect();
        batch.applied_values = merge_applied_values(&written.applied_values, Vec::new());
        batch.interrupted = false;
        let finish = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            ensure_operation_policy()?;
            journal.finish(batch.id, &batch)?;
            ensure_operation_policy()
        }));
        match finish {
            Ok(Ok(())) => {}
            Ok(Err(error)) => {
                return Err(rollback_failed_apply(
                    plan,
                    &mut batch,
                    &mut written,
                    journal,
                    &rollback_policy,
                    error,
                    true,
                ))
            }
            Err(payload) => {
                let _ = rollback_failed_apply(
                    plan,
                    &mut batch,
                    &mut written,
                    journal,
                    &rollback_policy,
                    JournalError::Storage("Apply completion unwound".into()).into(),
                    true,
                );
                std::panic::resume_unwind(payload)
            }
        }
        Ok(batch)
    }

    /// The write half of [`Executor::apply`]: tags, then moves and copies, then
    /// sidecars, then the tags of copies. Progress is recorded in `written` as it
    /// goes — before each step is attempted, so a step that fails half-way is
    /// still covered by the rollback. Returns the directories pruned afterwards.
    #[allow(clippy::too_many_arguments)]
    fn write_phase(
        plan: &ChangePlan,
        roots: &[PathBuf],
        pruning_intent: &[PathBuf],
        block_fingerprints: &mut [BlockFingerprint],
        written: &mut Written,
        steps: &mut Steps,
        journal: &mut dyn UndoJournal,
        batch_id: BatchId,
    ) -> Result<Vec<PathBuf>, PlanError> {
        let before_blocks: HashMap<_, _> = block_fingerprints
            .iter()
            .map(|item| (item.path.as_path(), item.before.as_slice()))
            .collect();
        // Tags first, at each file's original path — but only for the changes
        // that move. A copy must not touch its source at all (#153), so its tag
        // changes wait and are written to the copy below.
        for (index, change) in plan.changes.iter().enumerate() {
            if change.copy || !writes_in_place(change) {
                continue;
            }
            let _step = pinned_paths::Step::enter(std::iter::once(change.path.clone()))
                .map_err(PlanError::Io)?;
            steps.begin(&change.path)?;
            with_bound_path(&change.path, true, |path| {
                if let Some(target) = effective_rename(change) {
                    if !written
                        .copy_stamps
                        .get(target)
                        .is_some_and(|stamp| stamp.matches_identity(path))
                    {
                        return Err(PlanError::Stale(change.path.clone()));
                    }
                }
                ensure_not_stale_at(change, path)?;
                if let Some(before) = before_blocks.get(change.path.as_path()) {
                    if file_fingerprint(path)?.as_slice() != *before {
                        return Err(PlanError::Stale(change.path.clone()));
                    }
                }
                written.tag_steps[index] = 1;
                write_tag_changes(path, change, Direction::Apply)?;
                written
                    .applied_values
                    .extend(applied_values_at(change, path));
                written.tag_steps[index] = 2;
                apply_cover_change(path, change, Direction::Apply)?;
                if change.cover_change.is_some() {
                    written.cover_outcomes.insert(
                        change.path.clone(),
                        cover_digest(&TagEngine::read_covers(path)?),
                    );
                }
                written.tag_steps[index] = 3;
                write_blocks(path, change, Direction::Apply)?;
                if !change.block_changes.is_empty() {
                    written
                        .block_outcomes
                        .insert(change.path.clone(), file_fingerprint(path)?);
                }
                Ok(())
            })?;
            // Keep observations before progress can hand control to a caller or
            // a later step can make this path temporarily unreadable.
            written
                .applied_values
                .extend(applied_values_at(change, &change.path));
            steps.complete(&change.path, ApplyStep::Tags);
            ensure_operation_policy()?;
        }
        // ...then the moves and copies, creating any folders the targets need.
        // Directories are created here rather than in the pre-flight so a
        // validation failure can't leave empty folders behind.
        for (index, change) in plan.changes.iter().enumerate() {
            if let Some(target) = effective_rename(change) {
                let _step = pinned_paths::Step::enter([change.path.clone(), target.to_path_buf()])
                    .map_err(PlanError::Io)?;
                steps.begin(&change.path)?;
                if let Some(parent) = target.parent() {
                    written.created_dirs.extend(create_dirs_recording(parent)?);
                }
                // Checked before the transfer is recorded: a file found there is
                // not this batch's, and rolling back must not move it (#605).
                ensure_target_free(&change.path, target, change.copy)?;
                if !change.copy
                    && !written
                        .copy_stamps
                        .get(target)
                        .is_some_and(|stamp| stamp.matches_identity(&change.path))
                {
                    return Err(PlanError::Stale(change.path.clone()));
                }
                let mut created = false;
                let mut stamp = None;
                let result = if change.copy {
                    copy_file(&change.path, target, &mut created, &mut stamp)
                } else {
                    let proof = written
                        .copy_stamps
                        .get(target)
                        .ok_or_else(|| PlanError::Stale(change.path.clone()))?;
                    staged_move(journal, batch_id, proof, &change.path, target, &mut created)
                };
                if created {
                    written.transferred.push(index);
                }
                if let Some(stamp) = stamp {
                    written.copy_stamps.insert(target.to_path_buf(), stamp);
                }
                result?;
                if change.copy {
                    let stamp = written
                        .copy_stamps
                        .get(target)
                        .ok_or_else(|| PlanError::Stale(target.to_path_buf()))?;
                    journal.checkpoint_copy(batch_id, stamp, None)?;
                    ensure_operation_policy()?;
                }
                steps.complete(&change.path, ApplyStep::Transfer);
                ensure_operation_policy()?;
            }
        }
        // Sidecars follow their file, the same way (#58), after the main
        // transfers so their target directories already exist.
        for change in &plan.changes {
            for (from, to) in &change.sidecar_renames {
                let _step = pinned_paths::Step::entries([from.clone(), to.clone()])
                    .map_err(PlanError::Io)?;
                steps.begin(from)?;
                if let Some(parent) = to.parent() {
                    written.created_dirs.extend(create_dirs_recording(parent)?);
                }
                ensure_target_free(from, to, change.copy)?;
                if !change.copy
                    && !written
                        .copy_stamps
                        .get(to)
                        .is_some_and(|stamp| stamp.matches_identity(from))
                {
                    return Err(PlanError::Stale(from.clone()));
                }
                let mut created = false;
                let mut stamp = None;
                let result = if change.copy {
                    copy_file(from, to, &mut created, &mut stamp)
                } else {
                    let proof = written
                        .copy_stamps
                        .get(to)
                        .ok_or_else(|| PlanError::Stale(from.clone()))?;
                    staged_move(journal, batch_id, proof, from, to, &mut created)
                };
                if created {
                    written
                        .sidecars
                        .push((from.clone(), to.clone(), change.copy));
                }
                if let Some(stamp) = stamp {
                    written.copy_stamps.insert(to.clone(), stamp);
                }
                result?;
                if change.copy {
                    let stamp = written
                        .copy_stamps
                        .get(to)
                        .ok_or_else(|| PlanError::Stale(to.clone()))?;
                    journal.checkpoint_copy(batch_id, stamp, None)?;
                    ensure_operation_policy()?;
                }
                steps.complete(from, ApplyStep::Transfer);
                ensure_operation_policy()?;
            }
        }
        // Now that the source is a copy of its own, the copy gets the tag
        // changes the source was spared.
        for change in &plan.changes {
            if !change.copy {
                continue;
            }
            let Some(target) = effective_rename(change) else {
                continue;
            };
            if !writes_in_place(change) {
                continue;
            }
            let _step = pinned_paths::Step::enter(std::iter::once(target.to_path_buf()))
                .map_err(PlanError::Io)?;
            steps.begin(target)?;
            if !written
                .copy_stamps
                .get(target)
                .is_some_and(|stamp| stamp.matches(target))
            {
                return Err(PlanError::Stale(target.to_path_buf()));
            }
            let previous = written
                .copy_stamps
                .get(target)
                .cloned()
                .ok_or_else(|| PlanError::Stale(target.to_path_buf()))?;
            let original_copy = same_file::Handle::from_file(
                open_bound_file(target, false, false).map_err(PlanError::Io)?,
            )
            .map_err(PlanError::Io)?;
            with_bound_path(target, true, |path| {
                if original_copy
                    != same_file::Handle::from_file(
                        open_bound_file(path, false, false).map_err(PlanError::Io)?,
                    )
                    .map_err(PlanError::Io)?
                {
                    return Err(PlanError::Stale(target.to_path_buf()));
                }
                ensure_not_stale_at(change, path)?;
                write_tag_changes(path, change, Direction::Apply)?;
                apply_cover_change(path, change, Direction::Apply)?;
                write_blocks(path, change, Direction::Apply)?;
                let mut current = open_bound_file(path, true, false).map_err(PlanError::Io)?;
                current.sync_all().map_err(PlanError::Io)?;
                let mut stamp = CopyStamp::from_file(path, &mut current)
                    .ok_or_else(|| PlanError::Stale(target.to_path_buf()))?;
                stamp.path = target.to_path_buf();
                written.copy_stamps.insert(target.to_path_buf(), stamp);
                Ok(())
            })?;
            let stamp = written
                .copy_stamps
                .get(target)
                .ok_or_else(|| PlanError::Stale(target.to_path_buf()))?;
            journal.checkpoint_copy(batch_id, stamp, Some(&previous))?;
            ensure_operation_policy()?;
            steps.complete(&change.path, ApplyStep::Tags);
            ensure_operation_policy()?;
        }

        // A move can leave the folder it emptied behind (#153). Removing those
        // is opt-in per plan, and only ever the directories THIS batch emptied —
        // recorded so undo can put them back.
        // Observe only after all tag writes and transfers, before pruning can
        // remove a directory needed by error rollback.
        if !block_fingerprints.is_empty() {
            let mut by_path = HashMap::new();
            for change in &plan.changes {
                by_path.entry(change.path.as_path()).or_insert(change);
            }
            for fingerprint in block_fingerprints {
                let change = by_path
                    .get(fingerprint.path.as_path())
                    .expect("fingerprint belongs to this plan");
                let observed = written
                    .block_outcomes
                    .get(&change.path)
                    .ok_or_else(|| PlanError::Stale(change.path.clone()))?;
                if file_fingerprint(effective_rename(change).unwrap_or(&change.path))? != *observed
                {
                    return Err(PlanError::ChangedSinceApplied(change.path.clone()));
                }
                fingerprint.after = Some(observed.clone());
            }
        }
        ensure_operation_policy()?;
        let pruned = if plan.prune_empty_dirs {
            prune_emptied_dirs(pruning_intent, roots, &written.created_dirs)
        } else {
            Vec::new()
        };
        pinned_paths::validate().map_err(PlanError::Io)?;
        Ok(pruned)
    }

    /// Roll a previously applied batch back: move every renamed file back to
    /// its original path, remove every copy it made, restore every field's
    /// `old` value, then remove the batch from the journal.
    ///
    /// Renames are reversed *before* tag restoration, mirroring apply in
    /// reverse: the file lives at its rename target now, so it has to move
    /// back before the original-path tag write can find it.
    ///
    /// Containment works the same way as apply, over `allowed_roots` *plus the
    /// roots the batch itself recorded*. That second part is what lets a
    /// reorganize into a folder outside the library be undone in a later
    /// session (#153), when the caller no longer knows about that folder: the
    /// destination was confirmed by the user when the batch was applied, and
    /// the journal is where that authorization lives. Undo can still only move
    /// files back along paths the journal recorded, so this widens what undo
    /// may touch without widening what a plan may reach.
    pub fn undo(
        journal: &mut dyn UndoJournal,
        batch_id: BatchId,
        allowed_roots: &[PathBuf],
    ) -> Result<UndoReport, PlanError> {
        Self::undo_with(journal, batch_id, allowed_roots, &mut |_| {})
    }

    /// [`Executor::undo`] that reports how far it has got (#448), the same way
    /// [`Executor::apply_with`] does. There is no cancel: stopping half-way would
    /// leave the batch partly undone, which is the state #441 removed for apply.
    pub fn undo_with(
        journal: &mut dyn UndoJournal,
        batch_id: BatchId,
        allowed_roots: &[PathBuf],
        progress: &mut dyn FnMut(&ApplyProgress),
    ) -> Result<UndoReport, PlanError> {
        Self::undo_with_mode(journal, batch_id, allowed_roots, UndoMode::Strict, progress)
    }

    /// [`Executor::undo_with`] told what to do about a field changed after the
    /// batch was applied (#658). [`UndoMode::Strict`] refuses, which is what the
    /// interface asks first. A refusal had no way past it when the later edit
    /// came from another program, since there is no later batch to undo and the
    /// history is a stack; the other two modes are that way past, chosen by the
    /// person: keep the newer values and undo the rest, or put every old value
    /// back over them.
    pub fn undo_with_mode(
        journal: &mut dyn UndoJournal,
        batch_id: BatchId,
        allowed_roots: &[PathBuf],
        mode: UndoMode,
        progress: &mut dyn FnMut(&ApplyProgress),
    ) -> Result<UndoReport, PlanError> {
        if crate::model::recovery_active() {
            return Err(JournalError::NotRollbackable(
                "new user operations are refused during exact rollback".into(),
            )
            .into());
        }
        #[cfg(target_os = "linux")]
        if native_flac::private_phase_active() {
            return Err(JournalError::NotRollbackable(
                "native IO reentry is refused before callbacks".into(),
            )
            .into());
        }
        let _policy = crate::model::OperationPolicy::enter().ok_or_else(policy_changed)?;
        ensure_operation_policy()?;
        let mut report = UndoReport::default();
        let batch = journal
            .batch(batch_id)?
            .ok_or(PlanError::Journal(JournalError::UnknownBatch(batch_id)))?;
        ensure_operation_policy()?;
        ensure_unicode_plan(&batch.plan, allowed_roots)?;
        // Ambiguous persisted ownership must refuse before move recovery or
        // any earlier copy can be removed. Never silently choose one row.
        let mut copy_stamps = HashMap::new();
        for stamp in &batch.copy_stamps {
            if copy_stamps.insert(stamp.path.as_path(), stamp).is_some() {
                return Err(PlanError::Journal(JournalError::NotRollbackable(
                    "duplicate transfer ownership evidence".into(),
                )));
            }
        }

        let mut roots = canonical_roots(allowed_roots)?;
        for root in &batch.allowed_roots {
            if !roots.contains(root) {
                roots.push(root.clone());
            }
        }
        let _move_context =
            MoveContextScope::enter(batch.id, &batch.plan, &batch.copy_stamps, &roots)?;
        let move_stages = journal.move_stages(batch_id)?;
        ensure_operation_policy()?;
        let _paths = pinned_paths::Scope::enter(
            &roots,
            operation_paths(&batch.plan, &batch.copy_stamps).chain(
                move_stages
                    .iter()
                    .flat_map(|stage| [stage.from.clone(), stage.to.clone(), stage.path.clone()]),
            ),
        )
        .map_err(PlanError::Io)?;
        if batch.native_tags {
            if !batch.copy_stamps.is_empty() || !move_stages.is_empty() {
                return Err(JournalError::NotRollbackable(
                    "native history contains transfer recovery".into(),
                )
                .into());
            }
            #[cfg(target_os = "linux")]
            {
                let driver = journal.native_tags().ok_or_else(|| {
                    JournalError::NotRollbackable("native journal capability unavailable".into())
                })?;
                ensure_operation_policy()?;
                return native_flac::undo(&batch, driver, mode, progress, false);
            }
            #[cfg(not(target_os = "linux"))]
            return Err(JournalError::NotRollbackable(
                "native history requires Linux recovery".into(),
            )
            .into());
        }
        ensure_operation_policy()?;
        recover_move_stages(&move_stages)?;

        // Validate before touching disk: the file's current location (rename
        // target if it was renamed, else its path) and the restore
        // destination must both sit within a root.
        // A transfer that is already undone — by an earlier attempt that stopped
        // half-way (#604) — has nothing left to check or do.
        // Copy stamps and ID3v2 revisions are indexed once: Undo checks them per
        // change, and a linear scan of either made a large batch quadratic.
        let mut revisions = HashMap::new();
        for file in &batch.id3v2_revisions {
            revisions
                .entry(file.path.as_path())
                .or_insert(file.revision);
        }
        for change in &batch.plan.changes {
            match effective_rename(change) {
                Some(target)
                    if transfer_undone(target, &change.path, change.copy, &copy_stamps) =>
                {
                    resolve_target_within_roots(&change.path, &roots)?;
                }
                Some(target) => {
                    ensure_within_roots(
                        current_copy_path(target, change.copy, &copy_stamps),
                        &roots,
                    )?;
                    resolve_target_within_roots(&change.path, &roots)?;
                    ensure_untransfer_safe(
                        target,
                        &change.path,
                        change.copy,
                        &copy_stamps,
                        batch.interrupted,
                    )?;
                }
                None => ensure_within_roots(&change.path, &roots)?,
            }
            // A sidecar currently lives at its `to`; it must go back to `from`,
            // and both must sit within a root (#58).
            for (from, to) in &change.sidecar_renames {
                if transfer_undone(to, from, change.copy, &copy_stamps) {
                    resolve_target_within_roots(from, &roots)?;
                    continue;
                }
                if change.copy {
                    ensure_within_roots(current_copy_path(to, true, &copy_stamps), &roots)?;
                } else {
                    // A recorded sidecar move transports the link entry itself,
                    // never its referent. The referent may disappear after Apply.
                    resolve_target_within_roots(to, &roots)?;
                    std::fs::symlink_metadata(to).map_err(PlanError::Io)?;
                }
                resolve_target_within_roots(from, &roots)?;
                ensure_untransfer_safe(to, from, change.copy, &copy_stamps, batch.interrupted)?;
            }
        }

        let mut changed = changed_since_applied(&batch)?;
        ensure_block_undo_safe(&batch)?;
        ensure_covers_undo_safe(&batch)?;
        match mode {
            UndoMode::Strict => {
                if let Some((_, first)) = changed.first() {
                    return Err(PlanError::ChangedSinceApplied(first.path.clone()));
                }
            }
            UndoMode::KeepChanged => {
                // Named where the file is once undone: a rename goes back first.
                report.kept = changed
                    .iter()
                    .map(|(index, kept)| KeptField {
                        path: batch.plan.changes[*index].path.clone(),
                        field: kept.field.clone(),
                    })
                    .collect();
            }
            UndoMode::Overwrite => {}
        }

        // Put back the folders the batch emptied, before anything moves: a file
        // has to have a directory to move back into (#153).
        ensure_operation_policy()?;
        for dir in &batch.removed_dirs {
            if resolve_target_within_roots(dir, &roots).is_ok() {
                pinned_paths::create_all(dir).map_err(PlanError::Io)?;
            }
        }

        // Reverse the transfers first, so tag restoration finds each file back
        // at its original path. Undoing a copy means removing the copy — the
        // source was never touched, so there is nothing else to put back.
        let never = AtomicBool::new(false);
        let expected = expected_fields(&batch.applied_values);
        let block_expected: HashMap<_, _> = batch
            .block_fingerprints
            .iter()
            .map(|item| (item.path.as_path(), item))
            .collect();
        let mut steps = Steps::new(count_undo_steps(&batch.plan), progress, &never);
        for change in &batch.plan.changes {
            if let Some(target) = effective_rename(change) {
                let _step = pinned_paths::Step::enter([target.to_path_buf(), change.path.clone()])
                    .map_err(PlanError::Io)?;
                steps.begin(target)?;
                if !transfer_undone(target, &change.path, change.copy, &copy_stamps) {
                    if change.copy {
                        remove_recorded_copy(journal, batch_id, target, &copy_stamps)?;
                    } else {
                        untransfer_owned_move(
                            journal,
                            batch_id,
                            target,
                            &change.path,
                            &copy_stamps,
                        )?;
                    }
                }
                steps.done();
            }
            for (from, to) in &change.sidecar_renames {
                let _step = pinned_paths::Step::entries([to.clone(), from.clone()])
                    .map_err(PlanError::Io)?;
                steps.begin(to)?;
                if !transfer_undone(to, from, change.copy, &copy_stamps) {
                    if change.copy {
                        remove_recorded_copy(journal, batch_id, to, &copy_stamps)?;
                    } else {
                        untransfer_owned_move(journal, batch_id, to, from, &copy_stamps)?;
                    }
                }
                steps.done();
            }
        }
        for (index, change) in batch.plan.changes.iter().enumerate() {
            // A copy's tag changes were written to the copy, which no longer
            // exists; the source still holds its original values.
            if change.copy || !writes_in_place(change) {
                continue;
            }
            let _step = pinned_paths::Step::enter(std::iter::once(change.path.clone()))
                .map_err(PlanError::Io)?;
            steps.begin(&change.path)?;
            // Blocks come back before the field and image values are restored,
            // mirroring apply in reverse. It matters when the block that was
            // stripped is the one the app reads from: put it back first and the
            // restoring writes land on the same block they were taken from.
            with_bound_path(&change.path, true, |path| {
                if let Some(target) = effective_rename(change) {
                    ensure_move_identity(target, path, &copy_stamps)?;
                }
                if let Some(fingerprint) = block_expected.get(change.path.as_path()) {
                    let current = file_fingerprint(path)?;
                    if current == fingerprint.before {
                        return Ok(());
                    }
                    if fingerprint.after.as_ref() != Some(&current) {
                        return Err(PlanError::ChangedSinceApplied(change.path.clone()));
                    }
                } else if let Some(cover) = &change.cover_change {
                    let current = TagEngine::read_covers(path)?;
                    if current != cover.old && (batch.interrupted || current != cover.new) {
                        return Err(PlanError::ChangedSinceApplied(change.path.clone()));
                    }
                }
                for field in field_restore_gate(path, change, &expected, batch.interrupted, mode)? {
                    if !changed
                        .iter()
                        .any(|(at, item)| *at == index && item.field == field)
                    {
                        changed.push((
                            index,
                            KeptField {
                                path: change.path.clone(),
                                field: field.clone(),
                            },
                        ));
                        report.kept.push(KeptField {
                            path: change.path.clone(),
                            field,
                        });
                    }
                }
                write_blocks(path, change, Direction::Undo)?;
                let restorable = without_kept_fields(change, index, &changed, mode);
                write_tag_changes_with_gate(
                    path,
                    &restorable,
                    Direction::Undo,
                    Some(FieldGate {
                        expected: &expected,
                        interrupted: batch.interrupted,
                        mode,
                    }),
                )?;
                apply_cover_change(path, change, Direction::Undo)?;
                if let Some(&recorded) = revisions.get(change.path.as_path()) {
                    restore_revision_value(path, recorded)?;
                }
                Ok(())
            })?;
            steps.done();
        }

        // Finally remove the folders this batch created, deepest first. Only
        // empty ones go: if the user has since put something in there, the
        // directory stays and rollback is still considered successful.
        for dir in batch.created_dirs.iter().rev() {
            if ensure_within_roots(dir, &roots).is_ok() {
                pinned_paths::remove(dir, true).ok();
            }
        }

        for stamp in &batch.copy_stamps {
            if let Some(directory) = stamp.undo_path.as_deref().and_then(Path::parent) {
                if ensure_within_roots(directory, &roots).is_ok() {
                    let _ = pinned_paths::remove(directory, true);
                }
            }
        }

        pinned_paths::validate().map_err(PlanError::Io)?;
        ensure_operation_policy()?;
        // Successful terminal removal is the commit boundary. An arbitrary
        // adapter's post-delete callback cannot be uncommitted by this API.
        journal.rollback(batch_id)?;
        Ok(report)
    }
}

/// What [`Executor::undo_with_mode`] does with a field that no longer reads as
/// the batch left it.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum UndoMode {
    /// Refuse the undo and write nothing (#658). What is asked first.
    #[default]
    Strict,
    /// Leave each such field as it now is and undo everything else; the
    /// [`UndoReport`] names the fields left.
    KeepChanged,
    /// Put every old value back, over the later edits too.
    Overwrite,
}

/// What an undo left as it found it: empty unless it ran as
/// [`UndoMode::KeepChanged`] over fields changed after the batch.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct UndoReport {
    pub kept: Vec<KeptField>,
}

/// One field an undo kept: where the file was when the undo looked, and which
/// field.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeptField {
    pub path: PathBuf,
    pub field: TagField,
}

fn without_kept_fields(
    change: &FileChange,
    index: usize,
    changed: &[(usize, KeptField)],
    mode: UndoMode,
) -> FileChange {
    let mut restorable = FileChange {
        path: change.path.clone(),
        tag_changes: change.tag_changes.clone(),
        ..FileChange::default()
    };
    if mode == UndoMode::KeepChanged {
        restorable.tag_changes.retain(|field_change| {
            !changed
                .iter()
                .any(|(at, kept)| *at == index && kept.field == field_change.field)
        });
    }
    restorable
}

/// How far a running [`Executor::apply_with`] has got (#437). `total` counts the
/// preflight reads plus write steps (tags of a file, a move or copy, a sidecar),
/// `done` those finished. `completed` distinguishes a successful write from a
/// preflight/step-start notification. Media completions use the original plan
/// key; sidecar completions use the sidecar's original source path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApplyProgress {
    pub done: usize,
    pub total: usize,
    pub path: PathBuf,
    /// A successful write at its original source path (media or sidecar).
    /// Never emitted for a failed step; the enclosing batch may still roll back.
    pub completed: Option<ApplyStep>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApplyStep {
    Tags,
    Transfer,
}

fn policy_changed() -> PlanError {
    JournalError::NotRollbackable("metadata operation policy changed".into()).into()
}
fn ensure_operation_policy() -> Result<(), PlanError> {
    if crate::model::operation_policy_valid() {
        Ok(())
    } else {
        Err(policy_changed())
    }
}
fn rollback_failed_apply(
    plan: &ChangePlan,
    batch: &mut AppliedBatch,
    written: &mut Written,
    journal: &mut dyn UndoJournal,
    policy: &crate::model::RecoveryPolicy,
    cause: PlanError,
    after_write_phase: bool,
) -> PlanError {
    let _recovery = policy.enter();
    if after_write_phase {
        written.created_dirs.clone_from(&batch.created_dirs);
    }
    batch.interrupted = true;
    batch.copy_stamps = written.copy_stamps.values().cloned().collect();
    batch.applied_values = merge_applied_values(&written.applied_values, Vec::new());
    for fingerprint in &mut batch.block_fingerprints {
        fingerprint.after = written.block_outcomes.get(&fingerprint.path).cloned();
    }
    // A copy rollback may first reserve/move material. Keep its strong witness
    // durable before invoking the existing ownership-checked restoration.
    let _ = journal.retain_interrupted(batch.id, batch);
    let dirs = if after_write_phase {
        batch.removed_dirs.as_slice()
    } else {
        &[]
    };
    let error = written.roll_back_after(plan, dirs, cause, journal, batch.id);
    if !matches!(
        error,
        PlanError::RollbackIncomplete { .. } | PlanError::RecoveryRequired { .. }
    ) {
        let _ = journal.rollback(batch.id);
    } else {
        batch.created_dirs.clone_from(&written.created_dirs);
        batch.applied_values = merge_applied_values(&written.applied_values, Vec::new());
        for fingerprint in &mut batch.block_fingerprints {
            fingerprint.after = written.block_outcomes.get(&fingerprint.path).cloned();
        }
        retain_copy_removal_paths(batch, journal);
        let _ = journal.retain_interrupted(batch.id, batch);
    }
    error
}

/// Counts the steps of the write phase and is the one place it asks whether to
/// go on: before every step, never inside one.
struct Steps<'a> {
    total: usize,
    done: usize,
    progress: &'a mut dyn FnMut(&ApplyProgress),
    cancel: &'a AtomicBool,
}

impl<'a> Steps<'a> {
    fn new(
        total: usize,
        progress: &'a mut dyn FnMut(&ApplyProgress),
        cancel: &'a AtomicBool,
    ) -> Self {
        Self {
            total,
            done: 0,
            progress,
            cancel,
        }
    }

    /// Called before a step: stops the batch if cancelled, else reports it.
    fn begin(&mut self, path: &Path) -> Result<(), PlanError> {
        if self.cancel.load(Ordering::SeqCst) {
            return Err(PlanError::Cancelled);
        }
        ensure_operation_policy()?;
        (self.progress)(&ApplyProgress {
            done: self.done,
            total: self.total,
            path: path.to_path_buf(),
            completed: None,
        });
        ensure_operation_policy()?;
        Ok(())
    }

    fn done(&mut self) {
        self.done += 1;
    }

    fn complete(&mut self, path: &Path, kind: ApplyStep) {
        self.done();
        (self.progress)(&ApplyProgress {
            done: self.done,
            total: self.total,
            path: path.to_path_buf(),
            completed: Some(kind),
        });
    }
}

/// Whether the change writes tag fields, images or blocks into the file.
fn writes_in_place(change: &FileChange) -> bool {
    !change.tag_changes.is_empty()
        || change.cover_change.is_some()
        || !change.block_changes.is_empty()
}

/// The steps [`Executor::write_phase`] will take, counted the way it takes them.
fn count_steps(plan: &ChangePlan) -> usize {
    plan.changes
        .iter()
        .map(|change| {
            let in_place = usize::from(writes_in_place(change));
            let moves = usize::from(effective_rename(change).is_some());
            // A copy's tags are written to the copy, a move's to the original:
            // one in-place step either way, but only a copy needs a target.
            let in_place = if change.copy && moves == 0 {
                0
            } else {
                in_place
            };
            in_place + moves + change.sidecar_renames.len()
        })
        .sum()
}

/// The steps [`Executor::undo_with`] will take: each transfer and sidecar put
/// back, then each file whose fields, images or blocks are restored.
fn count_undo_steps(plan: &ChangePlan) -> usize {
    plan.changes
        .iter()
        .map(|change| {
            let restores = usize::from(!change.copy && writes_in_place(change));
            restores
                + usize::from(effective_rename(change).is_some())
                + change.sidecar_renames.len()
        })
        .sum()
}

/// What a partially failed [`Executor::apply`] has done to disk so far, so it
/// can be put back (#441). Mirrors [`Executor::undo`], over only the steps that
/// were reached.
struct Written {
    /// Per change: how far its in-place writes got. 0 = untouched, 1 = tag
    /// fields attempted, 2 = + images attempted, 3 = + blocks attempted.
    tag_steps: Vec<u8>,
    /// Indices of changes whose move or copy was attempted.
    transferred: Vec<usize>,
    /// Sidecar (from, to, copy) transfers that were attempted.
    sidecars: Vec<(PathBuf, PathBuf, bool)>,
    created_dirs: Vec<PathBuf>,
    /// The ID3v2 revision of each file before the batch, as journaled (#666).
    revisions: Vec<FileRevision>,
    /// Observed field outcomes of completed in-place writes, before rollback.
    applied_values: Vec<AppliedValue>,
    copy_stamps: BTreeMap<PathBuf, CopyStamp>,
    cover_outcomes: HashMap<PathBuf, Vec<u8>>,
    block_outcomes: HashMap<PathBuf, Vec<u8>>,
    block_baselines: HashMap<PathBuf, Vec<u8>>,
}

impl Written {
    fn new(changes: usize) -> Self {
        Self {
            tag_steps: vec![0; changes],
            transferred: Vec::new(),
            sidecars: Vec::new(),
            created_dirs: Vec::new(),
            revisions: Vec::new(),
            applied_values: Vec::new(),
            copy_stamps: BTreeMap::new(),
            cover_outcomes: HashMap::new(),
            block_outcomes: HashMap::new(),
            block_baselines: HashMap::new(),
        }
    }

    /// Roll back, then pair `cause` with whatever the rollback could not undo
    /// (#442): `cause` itself when everything is back, otherwise
    /// [`PlanError::RollbackIncomplete`], so the user is told the library needs a
    /// look instead of only that the write failed.
    fn roll_back_after(
        &mut self,
        plan: &ChangePlan,
        removed_dirs: &[PathBuf],
        cause: PlanError,
        journal: &mut dyn UndoJournal,
        id: BatchId,
    ) -> PlanError {
        let mut failed = self.roll_back(plan, removed_dirs, journal, id);
        if failed.is_empty() {
            return cause;
        }
        PlanError::RollbackIncomplete {
            cause: Box::new(cause),
            count: failed.len(),
            first: failed.swap_remove(0),
        }
    }

    /// Reverse what was done, in the order undo uses: restore pruned folders,
    /// move or remove transfers, restore blocks/fields/images, drop created
    /// folders. Every step is attempted and its error ignored: the caller is
    /// already returning the failure that started this. Whether something is
    /// back is decided by looking at the disk afterwards, not by the result of
    /// the undo call — the step that failed usually fails to undo too, having
    /// done nothing to undo. Returns the paths that are still not as they were.
    fn roll_back(
        &mut self,
        plan: &ChangePlan,
        removed_dirs: &[PathBuf],
        journal: &mut dyn UndoJournal,
        id: BatchId,
    ) -> Vec<PathBuf> {
        let mut failed = Vec::new();
        match journal.move_stages(id) {
            Ok(stages) => {
                let validated = match validate_move_stages(&stages) {
                    Ok(validated) => validated,
                    Err(error) => return vec![recovery_path(&error).unwrap_or_default()],
                };
                if let Err(error) = recover_validated_move_stages(validated) {
                    failed.push(recovery_path(&error).unwrap_or_default());
                }
            }
            Err(_) => {
                return vec![plan
                    .changes
                    .first()
                    .map(|change| change.path.clone())
                    .unwrap_or_default()]
            }
        }
        let mut restored_fields = Vec::new();
        let mut restored_blocks = Vec::new();
        let mut revisions = HashMap::new();
        let expected = expected_fields(&self.applied_values);
        let stamps: HashMap<_, _> = self
            .copy_stamps
            .iter()
            .map(|(path, stamp)| (path.as_path(), stamp))
            .collect();
        for file in &self.revisions {
            revisions
                .entry(file.path.as_path())
                .or_insert(file.revision);
        }
        for dir in removed_dirs {
            let _ = pinned_paths::create_all(dir);
            if !dir.is_dir() {
                failed.push(dir.clone());
            }
        }
        for (from, to, copy) in self.sidecars.iter().rev() {
            let _ = if *copy {
                remove_recorded_copy(journal, id, to, &stamps)
            } else {
                untransfer_owned_move(journal, id, to, from, &stamps)
            };
            if !transfer_undone(to, from, *copy, &stamps) {
                failed.push(from.clone());
            }
        }
        for &index in self.transferred.iter().rev() {
            let change = &plan.changes[index];
            if let Some(target) = effective_rename(change) {
                let _ = if change.copy {
                    remove_recorded_copy(journal, id, target, &stamps)
                } else {
                    untransfer_owned_move(journal, id, target, &change.path, &stamps)
                };
                if !transfer_undone(target, &change.path, change.copy, &stamps) {
                    failed.push(change.path.clone());
                }
            }
        }
        for (index, change) in plan.changes.iter().enumerate() {
            let steps = self.tag_steps[index];
            if change.copy || steps == 0 {
                continue;
            }
            let restored = with_bound_path(&change.path, true, |path| {
                if let Some(target) = effective_rename(change) {
                    ensure_move_identity(target, path, &stamps)?;
                }
                field_restore_gate(path, change, &expected, true, UndoMode::Strict)?;
                if let Some(cover) = &change.cover_change {
                    let current = TagEngine::read_covers(path)?;
                    if current != cover.old
                        && self.cover_outcomes.get(&change.path) != Some(&cover_digest(&current))
                    {
                        return Err(PlanError::ChangedSinceApplied(change.path.clone()));
                    }
                }
                if steps >= 3 && !change.block_changes.is_empty() {
                    let current = file_fingerprint(path)?;
                    if self.block_baselines.get(&change.path) == Some(&current) {
                        return Ok(applied_values_at(change, path));
                    }
                    if self.block_outcomes.get(&change.path) != Some(&current) {
                        return Err(PlanError::ChangedSinceApplied(change.path.clone()));
                    }
                    write_blocks(path, change, Direction::Undo)?;
                }
                write_tag_changes_with_gate(
                    path,
                    change,
                    Direction::Undo,
                    Some(FieldGate {
                        expected: &expected,
                        interrupted: true,
                        mode: UndoMode::Strict,
                    }),
                )?;
                if steps >= 2 {
                    apply_cover_change(path, change, Direction::Undo)?;
                }
                if let Some(&recorded) = revisions.get(change.path.as_path()) {
                    restore_revision_value(path, recorded)?;
                }
                Ok(applied_values_at(change, path))
            });
            // The file holds the values the plan recorded as `old` again, or it
            // does not.
            if restored.is_err() || ensure_not_stale(change).is_err() {
                failed.push(change.path.clone());
            }
            if let Ok(fields) = restored {
                restored_fields.extend(fields);
                if !change.block_changes.is_empty() {
                    if let Ok(hash) = file_fingerprint(&change.path) {
                        restored_blocks.push((change.path.clone(), hash));
                    }
                }
            }
        }
        for dir in self.created_dirs.iter().rev() {
            let _ = pinned_paths::remove(dir, true);
        }
        self.applied_values.extend(restored_fields);
        self.block_outcomes.extend(restored_blocks);
        failed
    }
}

/// Whether one [`transfer`] is undone: a move is back at `original` with nothing
/// left at `target`, a copy is gone from `target`.
fn is_untransferred(target: &Path, original: &Path, copy: bool) -> bool {
    // Entry existence must not follow a sidecar link. Other I/O errors are
    // unknown, not evidence that transport/rollback completed successfully.
    let absent = |path: &Path| {
        matches!(std::fs::symlink_metadata(path),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound)
    };
    if copy {
        absent(target)
    } else {
        std::fs::symlink_metadata(original).is_ok()
            && (absent(target) || folds_onto(original, target))
    }
}

/// Whether `to` reaches the file at `from` only because the file system folds
/// the name (#707): one directory entry, spelled another way. Being the same
/// file is not enough, as two hard links are that under two real names, and a
/// rename from one onto the other does nothing. What tells them apart is whether
/// `to` is spelled so in its folder. Only asked once both paths are found to
/// exist, so the listing is not a cost a normal rename pays.
fn folds_onto(from: &Path, to: &Path) -> bool {
    same_file::is_same_file(from, to).unwrap_or(false) && !is_listed(to)
}

/// Whether `path`'s folder lists an entry with exactly its name. A folder that
/// cannot be listed counts as listing it, the answer that refuses a move.
fn is_listed(path: &Path) -> bool {
    let (Some(parent), Some(name)) = (path.parent(), path.file_name()) else {
        return true;
    };
    match std::fs::read_dir(parent) {
        Ok(mut entries) => {
            entries.any(|entry| entry.map_or(true, |entry| entry.file_name() == name))
        }
        Err(_) => true,
    }
}

/// Which side of a [`FieldChange`] to write: `new` when applying, `old` when
/// undoing. Both directions share one write path so they can't diverge.
#[derive(Clone, Copy)]
enum Direction {
    Apply,
    Undo,
}

/// Canonicalize every allowed root, dropping the ones that no longer resolve.
/// A root that has been deleted or renamed since it was chosen simply stops
/// authorizing anything, which fails safe; only when NONE of them resolve is
/// this an error, since then nothing could be written anyway.
fn canonical_roots(allowed_roots: &[PathBuf]) -> Result<Vec<PathBuf>, PlanError> {
    let mut roots = Vec::with_capacity(allowed_roots.len());
    let mut first_error = None;
    for root in allowed_roots {
        match std::fs::canonicalize(root) {
            Ok(canonical) => {
                if !roots.contains(&canonical) {
                    roots.push(canonical);
                }
            }
            Err(error) => {
                first_error.get_or_insert(error);
            }
        }
    }
    match first_error {
        Some(error) if roots.is_empty() => Err(PlanError::Io(error)),
        _ => Ok(roots),
    }
}

fn ensure_unicode_plan(plan: &ChangePlan, roots: &[PathBuf]) -> Result<(), PlanError> {
    if roots
        .iter()
        .map(PathBuf::as_path)
        .chain(plan.changes.iter().flat_map(|change| {
            std::iter::once(change.path.as_path())
                .chain(change.rename_to.as_deref())
                .chain(
                    change
                        .sidecar_renames
                        .iter()
                        .flat_map(|(from, to)| [from.as_path(), to.as_path()]),
                )
        }))
        .any(|path| path.to_str().is_none())
    {
        return Err(PlanError::Io(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "Paths must be Unicode until the durable journal supports lossless native path storage",
        )));
    }
    Ok(())
}

fn operation_paths<'a>(
    plan: &'a ChangePlan,
    stamps: &'a [CopyStamp],
) -> impl Iterator<Item = PathBuf> + 'a {
    plan.changes
        .iter()
        .flat_map(|change| {
            std::iter::once(change.path.clone())
                .chain(change.rename_to.iter().cloned())
                .chain(
                    change
                        .sidecar_renames
                        .iter()
                        .flat_map(|(from, to)| [from.clone(), to.clone()]),
                )
        })
        .chain(stamps.iter().filter_map(|stamp| stamp.undo_path.clone()))
}

pub(crate) fn open_bound_entry(path: &Path) -> std::io::Result<std::fs::File> {
    if metadata_identity(path).is_some() {
        return open_bound_file(path, false, false);
    }
    pinned_paths::entry(path)
}

fn capture_move_stamps(
    plan: &ChangePlan,
    cancel: &AtomicBool,
) -> Result<Vec<CopyStamp>, PlanError> {
    let mut stamps = Vec::new();
    for change in &plan.changes {
        if cancel.load(Ordering::SeqCst) {
            return Err(PlanError::Cancelled);
        }
        if change.copy {
            // Copying link targets would escape link-entry semantics. Refuse
            // before journaling or any mutation until link copying is supported.
            for (from, _) in &change.sidecar_renames {
                if open_bound_entry(from)
                    .map_err(PlanError::Io)?
                    .metadata()
                    .map_err(PlanError::Io)?
                    .file_type()
                    .is_symlink()
                {
                    return Err(PlanError::Stale(from.clone()));
                }
            }
            continue;
        }
        if let Some(target) = effective_rename(change) {
            stamps.push(CopyStamp::for_move(&change.path, target)?);
        }
        for (from, to) in &change.sidecar_renames {
            if cancel.load(Ordering::SeqCst) {
                return Err(PlanError::Cancelled);
            }
            stamps.push(CopyStamp::for_move(from, to)?);
        }
    }
    Ok(stamps)
}

fn ensure_move_identity(
    target: &Path,
    actual: &Path,
    stamps: &HashMap<&Path, &CopyStamp>,
) -> Result<(), PlanError> {
    if stamps
        .get(target)
        .is_some_and(|stamp| stamp.matches_identity(actual))
    {
        Ok(())
    } else {
        Err(PlanError::Stale(actual.to_path_buf()))
    }
}
fn untransfer_owned_move(
    journal: &mut dyn UndoJournal,
    id: BatchId,
    target: &Path,
    original: &Path,
    stamps: &HashMap<&Path, &CopyStamp>,
) -> Result<(), PlanError> {
    let proof = stamps
        .get(target)
        .ok_or_else(|| PlanError::Stale(target.to_path_buf()))?;
    let mut attempted = false;
    staged_move(journal, id, proof, target, original, &mut attempted)
}

#[derive(Clone)]
struct MoveIntent {
    from: PathBuf,
    to: PathBuf,
    allow_link: bool,
    identity: Option<Vec<u8>>,
}
struct MoveContext {
    id: BatchId,
    roots: Vec<PathBuf>,
    intents: HashMap<PathBuf, MoveIntent>,
    seen_stages: HashSet<PathBuf>,
    stage_counts: HashMap<PathBuf, usize>,
    stage_bytes: usize,
}
thread_local! { static MOVE_CONTEXT: std::cell::RefCell<Option<MoveContext>> = const { std::cell::RefCell::new(None) }; }
struct MoveContextScope(Option<MoveContext>);
impl Drop for MoveContextScope {
    fn drop(&mut self) {
        MOVE_CONTEXT.with(|slot| {
            slot.replace(self.0.take());
        });
    }
}
impl MoveContextScope {
    fn activate(context: MoveContext) -> Self {
        Self(MOVE_CONTEXT.with(|slot| slot.replace(Some(context))))
    }
    fn enter(
        id: BatchId,
        plan: &ChangePlan,
        stamps: &[CopyStamp],
        roots: &[PathBuf],
    ) -> Result<Self, PlanError> {
        let context = MoveContext::compile(id, plan, stamps, roots)?;
        Ok(Self(MOVE_CONTEXT.with(|slot| slot.replace(Some(context)))))
    }
}
fn normalized_absolute(path: &Path) -> Result<PathBuf, PlanError> {
    let path = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir().map_err(PlanError::Io)?.join(path)
    };
    if path
        .components()
        .any(|part| matches!(part, std::path::Component::ParentDir))
    {
        return Err(recovery_required(
            &path,
            "parent traversal is not a sealed recovery path",
        ));
    }
    Ok(path.components().collect())
}
impl MoveContext {
    fn compile(
        id: BatchId,
        plan: &ChangePlan,
        stamps: &[CopyStamp],
        roots: &[PathBuf],
    ) -> Result<Self, PlanError> {
        let mut intents = HashMap::new();
        let mut sources = HashSet::new();
        for change in &plan.changes {
            if change.copy {
                continue;
            }
            let audio = effective_rename(change).map(|target| (&change.path, target, false));
            for (from, to, allow_link) in audio.into_iter().chain(
                change
                    .sidecar_renames
                    .iter()
                    .map(|(from, to)| (from, to.as_path(), true)),
            ) {
                if !sources.insert(normalized_absolute(from)?) || intents.contains_key(to) {
                    return Err(recovery_required(
                        from,
                        "conflicting immutable move intents",
                    ));
                }
                resolve_target_within_roots(from, roots)
                    .map_err(|error| recovery_required(from, error.to_string()))?;
                resolve_target_within_roots(to, roots)
                    .map_err(|error| recovery_required(to, error.to_string()))?;
                intents.insert(
                    to.to_path_buf(),
                    MoveIntent {
                        from: from.clone(),
                        to: to.to_path_buf(),
                        allow_link,
                        identity: None,
                    },
                );
            }
        }
        let mut proved = HashSet::new();
        for stamp in stamps {
            if !stamp.moved {
                continue;
            }
            let Some(intent) = intents.get_mut(&stamp.path) else {
                return Err(recovery_required(
                    &stamp.path,
                    "move proof has no immutable plan intent",
                ));
            };
            if stamp.path.as_os_str() != intent.to.as_os_str()
                || !proved.insert(stamp.path.clone())
                || stamp.sha256.is_some()
                || stamp.undo_path.is_some()
            {
                return Err(recovery_required(&stamp.path, "ambiguous typed move proof"));
            }
            let identity = stamp
                .identity
                .as_ref()
                .and_then(|bytes| NativeIdentity::decode(bytes));
            if !identity.is_some_and(|identity| {
                identity.current_platform() && identity.file_role(intent.allow_link)
            }) {
                return Err(recovery_required(
                    &stamp.path,
                    "invalid typed native move identity",
                ));
            }
            intent.identity.clone_from(&stamp.identity);
        }
        Ok(Self {
            id,
            roots: roots.to_vec(),
            intents,
            seen_stages: HashSet::new(),
            stage_counts: HashMap::new(),
            stage_bytes: 0,
        })
    }
    fn check_additional_budget(&self, stages: &[MoveStage]) -> Result<(), PlanError> {
        let mut new_paths = HashSet::new();
        let mut counts = HashMap::new();
        let mut bytes = 0usize;
        for stage in stages {
            if self.seen_stages.contains(&stage.path) || !new_paths.insert(stage.path.as_path()) {
                continue;
            }
            bytes = bytes.saturating_add(crate::journal::move_stage_bytes(stage));
            let count = counts.entry(stage.target.as_path()).or_insert(0usize);
            *count += 1;
            if self
                .stage_counts
                .get(&stage.target)
                .copied()
                .unwrap_or(0)
                .saturating_add(*count)
                > 64
                || self.seen_stages.len().saturating_add(new_paths.len())
                    > crate::journal::MAX_MOVE_STAGE_ROWS
                || self.stage_bytes.saturating_add(bytes) > crate::journal::MAX_MOVE_STAGE_BYTES
            {
                return Err(recovery_required(
                    &stage.path,
                    "move staging would exceed its recovery budget",
                ));
            }
        }
        Ok(())
    }
    fn remember_stages(&mut self, stages: &[MoveStage]) -> Result<(), PlanError> {
        self.check_additional_budget(stages)?;
        for stage in stages {
            if self.seen_stages.insert(stage.path.clone()) {
                *self.stage_counts.entry(stage.target.clone()).or_default() += 1;
                self.stage_bytes = self
                    .stage_bytes
                    .saturating_add(crate::journal::move_stage_bytes(stage));
            }
        }
        Ok(())
    }
    fn validate_metadata(&self, stages: &[MoveStage]) -> Result<(), PlanError> {
        if stages.len() > crate::journal::MAX_MOVE_STAGE_ROWS {
            return Err(recovery_required(
                Path::new(""),
                "move recovery collection exceeds its row budget",
            ));
        }
        let mut paths = HashSet::new();
        let mut directories = HashSet::new();
        let mut counts = HashMap::new();
        let mut bytes = 0usize;
        for stage in stages {
            bytes = bytes.saturating_add(crate::journal::move_stage_bytes(stage));
            if bytes > crate::journal::MAX_MOVE_STAGE_BYTES {
                return Err(recovery_required(
                    &stage.path,
                    "move recovery collection exceeds its byte budget",
                ));
            }
            let Some(intent) = self.intents.get(&stage.target) else {
                return Err(recovery_required(
                    &stage.path,
                    "staged target has no immutable plan intent",
                ));
            };
            let exact = |a: &Path, b: &Path| a.as_os_str() == b.as_os_str();
            if !exact(&stage.target, &intent.to)
                || !((exact(&stage.from, &intent.from) && exact(&stage.to, &intent.to))
                    || (exact(&stage.from, &intent.to) && exact(&stage.to, &intent.from)))
            {
                return Err(recovery_required(
                    &stage.path,
                    "staged endpoints disagree with immutable plan intent",
                ));
            }
            if intent.identity.as_ref() != Some(&stage.identity)
                || !NativeIdentity::decode(&stage.identity).is_some_and(|identity| {
                    identity.current_platform() && identity.file_role(intent.allow_link)
                })
            {
                return Err(recovery_required(
                    &stage.path,
                    "staged identity disagrees with typed persisted ownership",
                ));
            }
            if !NativeIdentity::decode(&stage.directory_identity)
                .is_some_and(|identity| identity.current_platform() && identity.directory_role())
            {
                return Err(recovery_required(
                    &stage.path,
                    "invalid native private-directory identity",
                ));
            }
            let normalized = normalized_absolute(&stage.path)?;
            let directory = stage
                .path
                .parent()
                .ok_or_else(|| recovery_required(&stage.path, "missing private directory"))?;
            let parent = directory
                .parent()
                .ok_or_else(|| recovery_required(directory, "missing private parent"))?;
            let source_parent = stage
                .from
                .parent()
                .ok_or_else(|| recovery_required(&stage.from, "missing staged source parent"))?;
            let prefix = format!(".tagrex-move-{}-", self.id.0);
            let suffix = directory
                .file_name()
                .and_then(|name| name.to_str())
                .and_then(|name| name.strip_prefix(&prefix));
            let valid_name = suffix.is_some_and(|suffix| {
                let parts = suffix.split('-').collect::<Vec<_>>();
                parts.len() == 3
                    && parts[0]
                        .parse::<u128>()
                        .is_ok_and(|value| value.to_string() == parts[0])
                    && parts[1]
                        .parse::<u64>()
                        .is_ok_and(|value| value.to_string() == parts[1])
                    && parts[2]
                        .parse::<u8>()
                        .is_ok_and(|value| value < 128 && value.to_string() == parts[2])
            });
            if !stage.path.is_absolute()
                || stage.path.as_os_str() != normalized.as_os_str()
                || stage.path.file_name() != Some(std::ffi::OsStr::new("entry"))
                || !valid_name
                || normalized_absolute(parent)? != normalized_absolute(source_parent)?
            {
                return Err(recovery_required(
                    &stage.path,
                    "private path is not a canonical reservation for its planned source",
                ));
            }
            resolve_target_within_roots(&stage.path, &self.roots)
                .map_err(|error| recovery_required(&stage.path, error.to_string()))?;
            if !paths.insert(normalized) || !directories.insert(stage.directory_identity.as_slice())
            {
                return Err(recovery_required(
                    &stage.path,
                    "duplicate or conflicting private recovery reservations",
                ));
            }
            let count = counts.entry(stage.target.as_path()).or_insert(0usize);
            *count += 1;
            if *count > 64 {
                return Err(recovery_required(
                    &stage.path,
                    "move recovery retry budget exceeded",
                ));
            }
        }
        Ok(())
    }
}
struct ValidatedMoveStages<'a>(&'a [MoveStage]);
fn validate_move_stages(stages: &[MoveStage]) -> Result<ValidatedMoveStages<'_>, PlanError> {
    if stages.is_empty() {
        return Ok(ValidatedMoveStages(stages));
    }
    MOVE_CONTEXT.with(|slot| {
        let mut slot = slot.borrow_mut();
        let context = slot.as_mut().ok_or_else(|| {
            recovery_required(Path::new(""), "missing immutable move recovery context")
        })?;
        context.validate_metadata(stages)?;
        let mut pending = HashSet::new();
        // All directory seals and collection conflicts are checked before the
        // first restore or cleanup, including rows whose usual entry is absent.
        for stage in stages {
            let directory = stage
                .path
                .parent()
                .ok_or_else(|| recovery_required(&stage.path, "missing private directory"))?;
            if entry_present(directory)? {
                stage_directory_matches(stage)?;
            }
            if entry_present(&stage.path)? {
                // Native capture can encounter a directory swapped in after
                // the final stat. It must not cause earlier valid reservations
                // to be restored before this unsupported payload is refused.
                let _entry = open_bound_entry(&stage.path)
                    .map_err(|error| recovery_required(&stage.path, error.to_string()))?;
            }
            if entry_present(&stage.path)? && !pending.insert(stage.target.as_path()) {
                return Err(recovery_required(
                    &stage.path,
                    "multiple pending captures for one move intent",
                ));
            }
        }
        context.remember_stages(stages)?;
        Ok(ValidatedMoveStages(stages))
    })
}

fn recovery_path(error: &PlanError) -> Option<PathBuf> {
    match error {
        PlanError::RecoveryRequired { path, .. } => Some(path.clone()),
        _ => None,
    }
}
fn recovery_required(path: &Path, reason: impl Into<String>) -> PlanError {
    PlanError::RecoveryRequired {
        path: path.to_path_buf(),
        reason: reason.into(),
    }
}
fn entry_present(path: &Path) -> Result<bool, PlanError> {
    match std::fs::symlink_metadata(path) {
        Ok(_) => Ok(true),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(PlanError::Io(error)),
    }
}
fn stage_directory_matches(stage: &MoveStage) -> Result<(), PlanError> {
    #[cfg(test)]
    let _timer = move_metrics::Timer::new(move_metrics::Phase::Proof);
    let directory = stage
        .path
        .parent()
        .ok_or_else(|| recovery_required(&stage.path, "missing private directory"))?;
    let file = pinned_paths::directory(directory)
        .map_err(|error| recovery_required(directory, error.to_string()))?;
    if crate::journal::native_identity(&file).as_ref() != Some(&stage.directory_identity) {
        return Err(recovery_required(
            directory,
            "private directory identity changed",
        ));
    }
    Ok(())
}
fn stage_object_matches(stage: &MoveStage, path: &Path) -> bool {
    #[cfg(test)]
    let _timer = move_metrics::Timer::new(move_metrics::Phase::Proof);
    open_bound_entry(path)
        .ok()
        .is_some_and(|file| crate::journal::native_file_matches(&stage.identity, &file))
}

fn cleanup_move_directory(stage: &MoveStage) -> Result<(), PlanError> {
    #[cfg(test)]
    let _timer = move_metrics::Timer::new(move_metrics::Phase::Cleanup);
    let directory = stage
        .path
        .parent()
        .ok_or_else(|| recovery_required(&stage.path, "missing private directory"))?;
    if !entry_present(directory)? {
        return Ok(());
    }
    stage_directory_matches(stage)?;
    pinned_paths::remove(directory, true).map_err(|error| {
        recovery_required(
            directory,
            format!("private recovery directory could not be removed empty: {error}"),
        )
    })?;
    if let Some(parent) = directory.parent() {
        sync_directory(parent).map_err(|error| recovery_required(parent, error.to_string()))?;
    }
    Ok(())
}
/// Return private material to the recorded pre-staging endpoint. This never
/// deletes payloads or overwrites a name; a mismatched object remains foreign.
fn recover_move_stages(stages: &[MoveStage]) -> Result<(), PlanError> {
    recover_validated_move_stages(validate_move_stages(stages)?)
}
fn recover_validated_move_stages(stages: ValidatedMoveStages<'_>) -> Result<(), PlanError> {
    for stage in stages.0 {
        if !entry_present(&stage.path)? {
            cleanup_move_directory(stage)?;
            continue;
        }
        stage_directory_matches(stage)?;
        let owned = stage_object_matches(stage, &stage.path);
        {
            let _step = pinned_paths::Step::entries([stage.path.clone(), stage.from.clone()])
                .map_err(|error| recovery_required(&stage.path, error.to_string()))?;
            rename_noreplace(&stage.path, &stage.from).map_err(|error| {
                recovery_required(
                    &stage.path,
                    format!("cannot restore pre-staging path: {error}"),
                )
            })?;
            sync_transfer_parents(&stage.path, &stage.from)
                .map_err(|error| recovery_required(&stage.from, error.to_string()))?;
            if !owned || !stage_object_matches(stage, &stage.from) {
                return Err(recovery_required(
                &stage.from,
                "unproved staged entry restored without overwrite; retained journal for resolution",
            ));
            }
        }
        cleanup_move_directory(stage)?;
    }
    Ok(())
}
fn reserve_move_stage(
    id: BatchId,
    proof: &CopyStamp,
    from: &Path,
    to: &Path,
) -> Result<MoveStage, PlanError> {
    #[cfg(test)]
    let _timer = move_metrics::Timer::new(move_metrics::Phase::Reservation);
    let parent = normalized_absolute(
        from.parent()
            .ok_or_else(|| PlanError::Stale(from.to_path_buf()))?,
    )?;
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| PlanError::Io(std::io::Error::other(error)))?
        .as_nanos();
    let mut reserved = None;
    for attempt in 0..128 {
        let directory = parent.join(format!(
            ".tagrex-move-{}-{nonce}-{}-{attempt}",
            id.0,
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        match pinned_paths::mkdir(&directory) {
            Ok(()) => {
                reserved = Some(directory);
                break;
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(PlanError::Io(error)),
        }
    }
    let directory = reserved.ok_or_else(|| PlanError::Stale(from.to_path_buf()))?;
    let directory_identity = pinned_paths::directory(&directory)
        .ok()
        .and_then(|file| crate::journal::native_identity(&file));
    let Some(directory_identity) = directory_identity else {
        let _ = pinned_paths::remove(&directory, true);
        return Err(PlanError::Stale(directory));
    };
    let stage = MoveStage {
        target: proof.path.clone(),
        from: from.to_path_buf(),
        to: to.to_path_buf(),
        path: directory.join("entry"),
        directory_identity,
        identity: proof
            .identity
            .clone()
            .ok_or_else(|| PlanError::Stale(from.to_path_buf()))?,
    };
    if let Err(error) = sync_directory(&directory).and_then(|()| sync_directory(&parent)) {
        let _ = cleanup_move_directory(&stage);
        return Err(PlanError::Io(error));
    }
    Ok(stage)
}
fn staged_move(
    journal: &mut dyn UndoJournal,
    id: BatchId,
    proof: &CopyStamp,
    from: &Path,
    to: &Path,
    attempted: &mut bool,
) -> Result<(), PlanError> {
    staged_move_with(journal, id, proof, from, to, attempted, |_| {})
}
#[allow(clippy::too_many_arguments)]
fn staged_move_with(
    journal: &mut dyn UndoJournal,
    id: BatchId,
    proof: &CopyStamp,
    from: &Path,
    to: &Path,
    attempted: &mut bool,
    mut checkpoint: impl FnMut(&str),
) -> Result<(), PlanError> {
    ensure_operation_policy()?;
    if !proof.matches_identity(from) {
        return Err(PlanError::Stale(from.to_path_buf()));
    }
    let stage = reserve_move_stage(id, proof, from, to)?;
    if let Err(error) = MOVE_CONTEXT.with(|slot| {
        let slot = slot.borrow();
        let context = slot
            .as_ref()
            .ok_or_else(|| recovery_required(&stage.path, "missing immutable staging context"))?;
        if context.id != id {
            return Err(recovery_required(
                &stage.path,
                "staging batch disagrees with active context",
            ));
        }
        context.validate_metadata(std::slice::from_ref(&stage))?;
        context.check_additional_budget(std::slice::from_ref(&stage))
    }) {
        let _ = cleanup_move_directory(&stage);
        return Err(error);
    }
    if let Err(error) = journal.prepare_move_stage(id, &stage) {
        let _ = cleanup_move_directory(&stage);
        return Err(error.into());
    }
    *attempted = true;
    MOVE_CONTEXT.with(|slot| {
        let mut slot = slot.borrow_mut();
        let context = slot
            .as_mut()
            .ok_or_else(|| recovery_required(&stage.path, "missing immutable staging context"))?;
        context.remember_stages(std::slice::from_ref(&stage))
    })?;
    ensure_operation_policy()?;
    checkpoint("intent");
    {
        #[cfg(test)]
        let _timer = move_metrics::Timer::new(move_metrics::Phase::Capture);
        let _step = pinned_paths::Step::entries([from.to_path_buf(), stage.path.clone()])
            .map_err(PlanError::Io)?;
        if !proof.matches_identity(from) {
            return Err(PlanError::Stale(from.to_path_buf()));
        }
        stage_directory_matches(&stage)?;
        checkpoint("checked");
        rename_noreplace(from, &stage.path).map_err(PlanError::Io)?;
        sync_transfer_parents(from, &stage.path)
            .map_err(|error| recovery_required(&stage.path, error.to_string()))?;
    }
    checkpoint("staged");
    stage_directory_matches(&stage)?;
    if !stage_object_matches(&stage, &stage.path) {
        // A source swap after the final stat may have captured a foreign entry.
        // Best-effort no-clobber restoration keeps every payload discoverable.
        let _ = recover_move_stages(std::slice::from_ref(&stage));
        return Err(recovery_required(
            &stage.path,
            "captured entry does not match original move ownership",
        ));
    }
    checkpoint("verified");
    let publication = (|| {
        #[cfg(test)]
        let _timer = move_metrics::Timer::new(move_metrics::Phase::Publication);
        let _step = pinned_paths::Step::entries([stage.path.clone(), to.to_path_buf()])
            .map_err(|error| recovery_required(&stage.path, error.to_string()))?;
        stage_directory_matches(&stage)?;
        if !stage_object_matches(&stage, &stage.path) {
            return Err(recovery_required(
                &stage.path,
                "private entry changed before publish",
            ));
        }
        checkpoint("publish_checked");
        rename_noreplace(&stage.path, to).map_err(|error| {
            if error.kind() == std::io::ErrorKind::AlreadyExists {
                PlanError::RenameCollision(to.to_path_buf())
            } else {
                PlanError::Io(error)
            }
        })?;
        sync_transfer_parents(&stage.path, to)
            .map_err(|error| recovery_required(to, error.to_string()))?;
        Ok::<(), PlanError>(())
    })();
    if let Err(error) = publication {
        // A failed publish, including a destination collision, must put an
        // owned staged object back when possible before returning to Undo.
        // If that restoration is blocked, preserve the reservation and intent.
        recover_move_stages(std::slice::from_ref(&stage))?;
        return Err(error);
    }
    checkpoint("published");
    if !stage_object_matches(&stage, to) {
        return Err(recovery_required(
            to,
            "published entry no longer matches original move ownership",
        ));
    }
    cleanup_move_directory(&stage)?;
    checkpoint("cleaned");
    Ok(())
}

/// Model IO belongs to the current held metadata request only on an exact
/// logical-name match. Independent public model requests (including a peer App
/// in a progress callback) retain ordinary standalone IO. A matching request
/// never falls back to the pathname if capability or identity checks fail.
pub(crate) fn open_metadata_file(path: &Path, write: bool) -> std::io::Result<std::fs::File> {
    if metadata_identity(path).is_some() {
        open_bound_file(path, write, false)
    } else {
        std::fs::OpenOptions::new()
            .read(true)
            .write(write)
            .open(path)
    }
}

/// Open a fresh file description through pinned parent authority, then bind it
/// to the original metadata object before a backend can read or mutate it.
pub(crate) fn open_bound_file(
    path: &Path,
    write: bool,
    create_new: bool,
) -> std::io::Result<std::fs::File> {
    let expected = metadata_identity(path);
    if expected.is_some() && create_new {
        return Err(std::io::Error::other(
            "Cannot replace a held metadata source",
        ));
    }
    if write && expected.is_some() {
        ensure_operation_policy().map_err(std::io::Error::other)?;
    }
    let file = pinned_paths::open(path, write, create_new)?;
    if let Some(expected) = expected {
        let current = same_file::Handle::from_file(file.try_clone()?)?;
        if expected.as_ref() != &current {
            return Err(std::io::Error::other("Held metadata source changed"));
        }
    }
    Ok(file)
}
struct MetadataBinding {
    logical: PathBuf,
    identity: std::rc::Rc<same_file::Handle>,
}
thread_local! {static METADATA_PATHS:std::cell::RefCell<Vec<MetadataBinding>>=const{std::cell::RefCell::new(Vec::new())};}
fn metadata_identity(path: &Path) -> Option<std::rc::Rc<same_file::Handle>> {
    METADATA_PATHS.with(|paths| {
        paths
            .borrow()
            .iter()
            .rev()
            .find(|binding| binding.logical == path)
            .map(|binding| binding.identity.clone())
    })
}
struct MetadataPathGuard;
impl Drop for MetadataPathGuard {
    fn drop(&mut self) {
        METADATA_PATHS.with(|paths| {
            paths.borrow_mut().pop();
        });
    }
}
fn with_bound_path<T>(
    path: &Path,
    write: bool,
    body: impl FnOnce(&Path) -> Result<T, PlanError>,
) -> Result<T, PlanError> {
    if write {
        ensure_operation_policy()?;
    }
    if metadata_identity(path).is_some() {
        return body(path);
    }
    let held = pinned_paths::metadata_file(path, write)
        .map_err(PlanError::Io)?
        .map(same_file::Handle::from_file)
        .transpose()
        .map_err(PlanError::Io)?
        .map(std::rc::Rc::new);
    let _scope = if let Some(identity) = &held {
        METADATA_PATHS.with(|paths| {
            paths.borrow_mut().push(MetadataBinding {
                logical: path.to_path_buf(),
                identity: identity.clone(),
            });
        });
        Some(MetadataPathGuard)
    } else {
        None
    };
    // The logical name keeps extension-directed parser/repair policy intact.
    // All actual IO enters open_bound_file and receives its own seek cursor.
    let result = body(path)?;
    if held.is_some() {
        open_bound_file(path, false, false).map_err(PlanError::Io)?;
    }
    Ok(result)
}

type ExpectedFields<'a> = BTreeMap<(&'a Path, &'a TagField), &'a Option<String>>;
fn expected_fields(values: &[AppliedValue]) -> ExpectedFields<'_> {
    values
        .iter()
        .map(|value| ((value.path.as_path(), &value.field), &value.value))
        .collect()
}
struct FieldGate<'a, 'b> {
    expected: &'b ExpectedFields<'a>,
    interrupted: bool,
    mode: UndoMode,
}
fn field_restore_gate(
    path: &Path,
    change: &FileChange,
    expected: &ExpectedFields<'_>,
    interrupted: bool,
    mode: UndoMode,
) -> Result<Vec<TagField>, PlanError> {
    if change.tag_changes.is_empty() {
        return Ok(Vec::new());
    }
    let current = TagEngine::read(path)?;
    field_restore_gate_on_tags(&current, change, expected, interrupted, mode)
}
fn field_restore_gate_on_tags(
    current: &TrackFile,
    change: &FileChange,
    expected: &ExpectedFields<'_>,
    interrupted: bool,
    mode: UndoMode,
) -> Result<Vec<TagField>, PlanError> {
    let key = effective_rename(change).unwrap_or(&change.path);
    let mut kept = Vec::new();
    for field in &change.tag_changes {
        let now = current.tags.get(&field.field);
        match expected.get(&(key, &field.field)) {
            None if interrupted && now != field.old.as_ref() => {
                return Err(PlanError::ChangedSinceApplied(change.path.clone()))
            }
            Some(applied) if now != applied.as_ref() && now != field.old.as_ref() => match mode {
                UndoMode::Strict => {
                    return Err(PlanError::ChangedSinceApplied(change.path.clone()))
                }
                UndoMode::KeepChanged => kept.push(field.field.clone()),
                UndoMode::Overwrite => {}
            },
            _ => {}
        }
    }
    Ok(kept)
}
fn cover_digest(covers: &[CoverArt]) -> Vec<u8> {
    use sha2::{Digest, Sha256};
    let mut hash = Sha256::new();
    hash.update((covers.len() as u64).to_le_bytes());
    for cover in covers {
        for bytes in [
            cover.kind.to_storage_key().as_bytes(),
            cover.description.as_bytes(),
            cover.mime.as_bytes(),
            cover.data.as_ref(),
        ] {
            hash.update((bytes.len() as u64).to_le_bytes());
            hash.update(bytes);
        }
    }
    hash.finalize().to_vec()
}
fn read_bound_track(path: &Path) -> Result<TrackFile, PlanError> {
    with_bound_path(path, false, |resolved| {
        let mut file = TagEngine::read(resolved)?;
        file.path = path.to_path_buf();
        Ok(file)
    })
}

/// A transfer never replaces a file (#605). The pre-flight already refused
/// targets that exist, but it compares paths as strings: on a case-insensitive
/// file system `Track.mp3` and `track.mp3` (or the NFC and NFD spelling of one
/// name) are the same file, so the second of two such moves would land on the
/// first. Checked again right before each transfer, where the file system has
/// the last word. The one file allowed to be there already is the one being
/// moved: a case-only rename of a single file (#707).
fn ensure_target_free(from: &Path, to: &Path, copy: bool) -> Result<(), PlanError> {
    match std::fs::symlink_metadata(to) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(PlanError::Io(error)),
        Ok(_) if !copy && (from == to || folds_onto(from, to)) => Ok(()),
        Ok(_) => Err(PlanError::RenameCollision(to.to_path_buf())),
    }
}

/// Whether undoing one transfer only touches what the batch made (#604).
///
/// A move goes back only onto a free path: a file put at the original path
/// since is not the batch's to replace (the file itself is fine, which is what a
/// case-only rename looks like). A copy is removed only while the file at its
/// path is still the copy as the batch left it. Historical missing/weak stamps
/// cannot authorize deletion; all such batches remain available for inspection.
///
/// `stamps` is indexed once per Undo so checking a large batch stays linear.
fn ensure_untransfer_safe(
    target: &Path,
    original: &Path,
    copy: bool,
    stamps: &HashMap<&Path, &CopyStamp>,
    _interrupted: bool,
) -> Result<(), PlanError> {
    if copy {
        let Some(stamp) = stamps.get(target) else {
            return Err(PlanError::Stale(target.to_path_buf()));
        };
        let actual = stamp
            .undo_path
            .as_deref()
            .filter(|path| path.exists())
            .unwrap_or(target);
        if stamp.matches(actual) {
            Ok(())
        } else {
            Err(PlanError::Stale(target.to_path_buf()))
        }
    } else {
        ensure_move_identity(target, target, stamps)?;
        ensure_target_free(target, original, false)
    }
}

fn current_copy_path<'a>(
    target: &'a Path,
    copy: bool,
    stamps: &'a HashMap<&Path, &CopyStamp>,
) -> &'a Path {
    if copy {
        if let Some(path) = stamps
            .get(target)
            .and_then(|stamp| stamp.undo_path.as_deref())
            .filter(|path| path.exists())
        {
            return path;
        }
    }
    target
}
fn transfer_undone(
    target: &Path,
    original: &Path,
    copy: bool,
    stamps: &HashMap<&Path, &CopyStamp>,
) -> bool {
    if copy && current_copy_path(target, true, stamps) != target {
        return false;
    }
    is_untransferred(target, original, copy)
        && (copy || ensure_move_identity(target, original, stamps).is_ok())
}

/// The ID3v2 revision of every file `plan` writes in place, read before it is
/// written (#666). A file with no ID3v2 block, or one that cannot be read, gets
/// nothing, and keeps whatever revision undo writes.
fn id3v2_revisions(plan: &ChangePlan) -> Vec<FileRevision> {
    plan.changes
        .iter()
        .filter(|change| !change.copy && writes_in_place(change))
        .filter_map(|change| {
            let revision = TagEngine::id3v2_revision(&change.path).ok()??;
            Some(FileRevision {
                path: change.path.clone(),
                revision,
            })
        })
        .collect()
}

/// Put `path` back in the ID3v2 revision journaled for it (#666). The writes
/// that restore its values save in the revision the app is set to, which need
/// not be the one the file was in. A 2.2 file stays as they left it: the backend
/// does not write 2.2.
fn restore_revision_value(path: &Path, recorded: Id3v2Revision) -> Result<(), PlanError> {
    if Id3v2Revision::WRITABLE.contains(&recorded)
        && TagEngine::id3v2_revision(path)?.is_some_and(|current| current != recorded)
    {
        TagEngine::set_id3v2_revision(path, recorded)?;
    }
    Ok(())
}

/// Observe fields only after a write, using the final plan path as the journal
/// key even when transfers have not run yet. An unreadable outcome is unknown.
fn applied_values_at(change: &FileChange, now_at: &Path) -> Vec<AppliedValue> {
    if change.copy || change.tag_changes.is_empty() {
        return Vec::new();
    }
    let Ok(current) = read_bound_track(now_at) else {
        return Vec::new();
    };
    let path = effective_rename(change).unwrap_or(&change.path);
    change
        .tag_changes
        .iter()
        .map(|field_change| AppliedValue {
            path: path.to_path_buf(),
            field: field_change.field.clone(),
            value: current.tags.get(&field_change.field).cloned(),
        })
        .collect()
}

/// Current values take precedence (including successful restoration to `old`),
/// while a temporarily unreadable path must not erase an earlier observation.
fn merge_applied_values(
    known: &[AppliedValue],
    mut current: Vec<AppliedValue>,
) -> Vec<AppliedValue> {
    let mut seen: BTreeSet<_> = current
        .iter()
        .map(|value| (value.path.clone(), value.field.clone()))
        .collect();
    // Prefer the last observation if a plan wrote the same path/field twice.
    let mut older = Vec::new();
    for value in known.iter().rev() {
        if seen.insert((value.path.clone(), value.field.clone())) {
            older.push(value.clone());
        }
    }
    older.reverse();
    if current.is_empty() {
        return older;
    }
    current.extend(older);
    current
}

/// Every field of `batch` that no longer reads as the batch left it (#658),
/// with the index of its change in the plan: a field must still read as the
/// batch wrote it, or as its `old` value already, where an earlier undo stopped
/// part-way (#604). Read for every file before any is touched, so a refusal
/// built on it writes nothing.
fn changed_since_applied(batch: &AppliedBatch) -> Result<Vec<(usize, KeptField)>, PlanError> {
    let mut changed = Vec::new();
    let mut recorded: HashMap<&Path, Vec<&AppliedValue>> = HashMap::new();
    for applied in &batch.applied_values {
        recorded.entry(&applied.path).or_default().push(applied);
    }
    for (index, change) in batch.plan.changes.iter().enumerate() {
        if change.copy {
            continue;
        }
        let path = effective_rename(change).unwrap_or(&change.path);
        let values = recorded.get(path);
        if values.is_none() && !batch.interrupted {
            // Completed legacy records intentionally retain their old behavior.
            continue;
        }
        if change.tag_changes.is_empty() {
            continue;
        }
        // Where the file is now: back at its own path when an earlier undo
        // already moved it there.
        let now_at = match effective_rename(change) {
            Some(target) if is_untransferred(target, &change.path, false) => &change.path,
            _ => path,
        };
        let current = read_bound_track(now_at)?;
        for field_change in &change.tag_changes {
            let now = current.tags.get(&field_change.field);
            let old = field_change.old.as_ref();
            let applied = values.and_then(|values| {
                values
                    .iter()
                    .find(|value| value.field == field_change.field)
            });
            // The plan's `new` value is intent, not proof that an interrupted
            // write produced it. Only an already-restored field is safe when
            // execution evidence is absent, even in an override mode.
            let Some(applied) = applied else {
                if batch.interrupted && now != old {
                    return Err(PlanError::ChangedSinceApplied(now_at.to_path_buf()));
                }
                continue;
            };
            if now != applied.value.as_ref() && now != old {
                changed.push((
                    index,
                    KeptField {
                        path: now_at.to_path_buf(),
                        field: field_change.field.clone(),
                    },
                ));
            }
        }
    }
    Ok(changed)
}

/// Streaming hashes avoid retaining audio or depending on modeled tag fields.
fn file_fingerprint(path: &Path) -> Result<Vec<u8>, PlanError> {
    use sha2::{Digest, Sha256};
    use std::io::Read;
    let mut file = open_bound_file(path, false, false).map_err(PlanError::Io)?;
    let mut digest = Sha256::new();
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let read = file.read(&mut buffer).map_err(PlanError::Io)?;
        if read == 0 {
            break;
        }
        digest.update(&buffer[..read]);
    }
    Ok(digest.finalize().to_vec())
}

fn capture_block_fingerprints(plan: &ChangePlan) -> Result<Vec<BlockFingerprint>, PlanError> {
    let mut seen = HashSet::new();
    plan.changes
        .iter()
        .filter(|change| {
            !change.copy && !change.block_changes.is_empty() && seen.insert(change.path.clone())
        })
        .map(|change| {
            Ok(BlockFingerprint {
                path: change.path.clone(),
                before: file_fingerprint(&change.path)?,
                after: None,
            })
        })
        .collect()
}

/// Reject any later file-content change before reversing transfers or writing
/// whole blocks. Already-restored files need no metadata rewrite on retry.
fn ensure_block_undo_safe(batch: &AppliedBatch) -> Result<HashSet<PathBuf>, PlanError> {
    let mut restored = HashSet::new();
    if batch.block_fingerprints.is_empty() {
        return Ok(restored);
    }
    let mut by_path = HashMap::new();
    for change in &batch.plan.changes {
        by_path.entry(change.path.as_path()).or_insert(change);
    }
    for fingerprint in &batch.block_fingerprints {
        let Some(change) = by_path.get(fingerprint.path.as_path()) else {
            return Err(PlanError::ChangedSinceApplied(fingerprint.path.clone()));
        };
        let current_path = match effective_rename(change) {
            Some(target) if !is_untransferred(target, &change.path, false) => target,
            _ => &change.path,
        };
        let current = file_fingerprint(current_path)?;
        if current == fingerprint.before {
            restored.insert(change.path.clone());
        } else if fingerprint.after.as_ref() != Some(&current) {
            return Err(PlanError::ChangedSinceApplied(current_path.to_path_buf()));
        }
    }
    Ok(restored)
}

/// Cover sets include bytes, ordering, picture types and descriptions. Field
/// override modes cannot choose which images to keep, so refuse an ambiguous
/// image restore in every mode before reversing any transfer. Completed plans
/// provide both possible safe states; interrupted intent alone proves only the
/// already-restored state.
fn ensure_covers_undo_safe(batch: &AppliedBatch) -> Result<(), PlanError> {
    let block_paths = batch
        .block_fingerprints
        .iter()
        .map(|fingerprint| fingerprint.path.as_path())
        .collect::<HashSet<_>>();
    for change in &batch.plan.changes {
        if change.copy {
            continue;
        }
        // A successful whole-file fingerprint check already includes covers,
        // including normalized/combined block writes.
        if block_paths.contains(change.path.as_path()) {
            continue;
        }
        let Some(covers) = &change.cover_change else {
            continue;
        };
        let now_at = match effective_rename(change) {
            Some(target) if !is_untransferred(target, &change.path, false) => target,
            _ => &change.path,
        };
        let current = TagEngine::read_covers(now_at)?;
        if current != covers.old && (batch.interrupted || current != covers.new) {
            return Err(PlanError::ChangedSinceApplied(now_at.to_path_buf()));
        }
    }
    Ok(())
}

/// Preserve durable copy-removal locations after a rollback attempt so a later
/// outcome update cannot discard the evidence needed to recover private files.
fn retain_copy_removal_paths(batch: &mut AppliedBatch, journal: &dyn UndoJournal) {
    if let Ok(Some(stored)) = journal.batch(batch.id) {
        let paths: HashMap<_, _> = stored
            .copy_stamps
            .into_iter()
            .filter_map(|stamp| stamp.undo_path.map(|path| (stamp.path, path)))
            .collect();
        for stamp in &mut batch.copy_stamps {
            if let Some(path) = paths.get(&stamp.path) {
                stamp.undo_path = Some(path.clone());
            }
        }
    }
}

/// Exclusively create and flush a copy; moves use the durable staging protocol.
fn copy_file(
    from: &Path,
    to: &Path,
    created: &mut bool,
    stamp: &mut Option<CopyStamp>,
) -> Result<(), PlanError> {
    use std::io;
    let mut source = open_bound_file(from, false, false).map_err(PlanError::Io)?;
    let mut target = open_bound_file(to, true, true).map_err(|error| {
        if error.kind() == io::ErrorKind::AlreadyExists {
            PlanError::RenameCollision(to.to_path_buf())
        } else {
            PlanError::Io(error)
        }
    })?;
    *created = true;
    let result = (|| {
        io::copy(&mut source, &mut target)?;
        target.set_permissions(source.metadata()?.permissions())?;
        target.sync_all()
    })();
    *stamp = CopyStamp::from_file(to, &mut target);
    result.map_err(PlanError::Io)?;
    if stamp.is_none() {
        return Err(PlanError::Stale(to.to_path_buf()));
    }
    sync_transfer_parents(from, to).map_err(PlanError::Io)
}

pub(crate) fn sync_directory(directory: &Path) -> std::io::Result<()> {
    if let Some(()) = pinned_paths::sync(directory)? {
        return Ok(());
    }
    #[cfg(unix)]
    {
        std::fs::File::open(if directory.as_os_str().is_empty() {
            Path::new(".")
        } else {
            directory
        })?
        .sync_all()
    }
    #[cfg(not(unix))]
    {
        let _ = directory;
        Ok(())
    }
}
fn sync_transfer_parents(from: &Path, to: &Path) -> std::io::Result<()> {
    if let Some(parent) = to.parent() {
        sync_directory(parent)?;
    }
    if from.parent() != to.parent() {
        if let Some(parent) = from.parent() {
            sync_directory(parent)?;
        }
    }
    Ok(())
}

/// Atomic no-clobber moves. Unsupported filesystems/platforms fail without a
/// copy/unlink fallback; that fallback has additional crash states to journal.
pub(crate) fn rename_noreplace(from: &Path, to: &Path) -> std::io::Result<()> {
    if let Some(()) = pinned_paths::rename(from, to)? {
        return Ok(());
    }
    native_rename_noreplace(from, to)
}
fn native_rename_noreplace(from: &Path, to: &Path) -> std::io::Result<()> {
    if from == to {
        return Ok(());
    }
    if folds_onto(from, to) {
        return std::fs::rename(from, to);
    }
    #[cfg(any(
        target_os = "linux",
        target_os = "android",
        target_os = "macos",
        target_os = "ios"
    ))]
    {
        rustix::fs::renameat_with(
            rustix::fs::CWD,
            from,
            rustix::fs::CWD,
            to,
            rustix::fs::RenameFlags::NOREPLACE,
        )
        .map_err(Into::into)
    }
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        let from: Vec<u16> = from.as_os_str().encode_wide().chain(Some(0)).collect();
        let to: Vec<u16> = to.as_os_str().encode_wide().chain(Some(0)).collect();
        // SAFETY: both UTF-16 buffers are NUL-terminated and live through the
        // call. MoveFileW never replaces an existing destination.
        if unsafe { windows_sys::Win32::Storage::FileSystem::MoveFileW(from.as_ptr(), to.as_ptr()) }
            != 0
        {
            Ok(())
        } else {
            Err(std::io::Error::last_os_error())
        }
    }
    #[cfg(not(any(
        target_os = "linux",
        target_os = "android",
        target_os = "macos",
        target_os = "ios",
        windows
    )))]
    {
        Err(std::io::Error::new(
            std::io::ErrorKind::Unsupported,
            "atomic no-replace rename unavailable",
        ))
    }
}

fn remove_recorded_copy(
    journal: &mut dyn UndoJournal,
    id: BatchId,
    target: &Path,
    stamps: &HashMap<&Path, &CopyStamp>,
) -> Result<(), PlanError> {
    remove_recorded_copy_with(journal, id, target, stamps, |_| {})
}

fn remove_recorded_copy_with(
    journal: &mut dyn UndoJournal,
    id: BatchId,
    target: &Path,
    stamps: &HashMap<&Path, &CopyStamp>,
    mut checkpoint: impl FnMut(&str),
) -> Result<(), PlanError> {
    let stamp = stamps
        .get(target)
        .ok_or_else(|| PlanError::Stale(target.to_path_buf()))?;
    let undo_path = match &stamp.undo_path {
        Some(path) => path.clone(),
        None => {
            let parent = target
                .parent()
                .ok_or_else(|| PlanError::Stale(target.to_path_buf()))?;
            let mut reserved = None;
            for attempt in 0..1024 {
                let directory = parent.join(format!(".tagrex-undo-copy-{}-{attempt}", id.0));
                match pinned_paths::mkdir(&directory) {
                    Ok(()) => {
                        reserved = Some(directory.join("copy"));
                        break;
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
                    Err(error) => return Err(PlanError::Io(error)),
                }
            }
            let path = reserved.ok_or_else(|| PlanError::Stale(target.to_path_buf()))?;
            if let Some(directory) = path.parent() {
                sync_directory(directory).map_err(PlanError::Io)?;
                if let Some(parent) = directory.parent() {
                    sync_directory(parent).map_err(PlanError::Io)?;
                }
            }
            if let Err(error) = journal.prepare_copy_removal(id, target, &path) {
                if let Some(directory) = path.parent() {
                    let _ = pinned_paths::remove(directory, true);
                }
                return Err(error.into());
            }
            ensure_operation_policy()?;
            path
        }
    };
    ensure_operation_policy()?;
    checkpoint("intent");
    if !undo_path.exists() {
        if !target.exists() {
            return Ok(());
        }
        // Check after the public progress callback, then atomically move to
        // our journaled private path. A replacement in the remaining window
        // is caught by the second digest check before any removal.
        if !stamp.matches(target) {
            return Err(PlanError::Stale(target.to_path_buf()));
        }
        checkpoint("checked");
        rename_noreplace(target, &undo_path).map_err(PlanError::Io)?;
        sync_transfer_parents(target, &undo_path).map_err(PlanError::Io)?;
    }
    checkpoint("moved");
    if !stamp.matches(&undo_path) {
        // Never overwrite a newly created target while returning an unrelated
        // replacement. If blocked, the journal retains its recovery location.
        let _ = rename_noreplace(&undo_path, target);
        return Err(PlanError::Stale(undo_path));
    }
    checkpoint("verified");
    pinned_paths::remove(&undo_path, false).map_err(PlanError::Io)?;
    if let Some(parent) = undo_path.parent() {
        sync_directory(parent).map_err(PlanError::Io)?;
    }
    checkpoint("removed");
    if let Some(directory) = undo_path.parent() {
        let _ = pinned_paths::remove(directory, true);
    }
    Ok(())
}

/// Existing source parents a move may empty, deepest first. Capture these in
/// the initial journal record so even a crash during pruning leaves Undo the
/// authorized parents it needs. Roots and directories created by this batch
/// are excluded; containment is verified while the parents still exist.
fn pruning_candidates(plan: &ChangePlan, roots: &[PathBuf], created: &[PathBuf]) -> Vec<PathBuf> {
    let mut candidates: Vec<PathBuf> = Vec::new();
    let mut seen = HashSet::new();
    for change in &plan.changes {
        if change.copy || effective_rename(change).is_none() {
            continue;
        }
        // The file's own folder, and every folder a file travelling with it came
        // out of (#161): carrying a `Scans/` subfolder away empties it, and an
        // empty subfolder left behind would keep its parent from being pruned.
        let sources = std::iter::once(change.path.as_path()).chain(
            change
                .sidecar_renames
                .iter()
                .map(|(from, _)| from.as_path()),
        );
        for source in sources {
            let mut dir = source.parent();
            while let Some(current) = dir {
                let owned = current.to_path_buf();
                // A root reached through a parent symlink is still a boundary.
                if !is_prunable_parent(current, roots) {
                    break;
                }
                if seen.insert(owned.clone()) {
                    candidates.push(owned);
                }
                dir = current.parent();
            }
        }
    }
    // Deepest first, so a parent is only considered once its child is gone.
    candidates.sort_by_key(|dir| std::cmp::Reverse(dir.components().count()));

    candidates.retain(|dir| !created.contains(dir));
    candidates
}

/// Prune only durably recorded candidates, never a newly discovered parent.
/// Recheck containment and emptiness now; a transient pre-write read failure
/// must leave a directory unpruned rather than create an unrecorded removal.
fn prune_emptied_dirs(
    candidates: &[PathBuf],
    roots: &[PathBuf],
    created: &[PathBuf],
) -> Vec<PathBuf> {
    let mut removed = Vec::new();
    for dir in candidates {
        if created.contains(dir) || !is_prunable_parent(dir, roots) {
            continue;
        }
        // A folder the batch emptied of everything the user keeps may still
        // hold the OS's own bookkeeping (#404) — a `.DS_Store` Finder wrote
        // while it was open. That is not content, and it is not carried either,
        // so it goes with the folder instead of keeping it from being tidied.
        if clear_os_junk(dir) && pinned_paths::remove(dir, true).is_ok() {
            removed.push(dir.clone());
        }
    }
    removed
}

/// Only strict descendants of the authorized roots may be pruned. Comparing
/// canonically protects a root even when the plan used an aliased parent path.
fn is_prunable_parent(dir: &Path, roots: &[PathBuf]) -> bool {
    let Ok(canonical) = dir.canonicalize() else {
        return false;
    };
    !roots.contains(&canonical) && roots.iter().any(|root| canonical.starts_with(root))
}

/// Whether `path` is a file the OS writes into folders for its own bookkeeping
/// (#404): Finder's `.DS_Store` and AppleDouble `._*` companions, Windows'
/// `Thumbs.db`, `ehthumbs.db` and `desktop.ini`. None of it is the user's —
/// each is rewritten by the OS on demand — so a move neither carries it nor
/// lets it keep an emptied folder from being removed.
pub fn is_os_junk(path: &Path) -> bool {
    let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
        return false;
    };
    name == ".DS_Store"
        || name.starts_with("._")
        || name.eq_ignore_ascii_case("Thumbs.db")
        || name.eq_ignore_ascii_case("ehthumbs.db")
        || name.eq_ignore_ascii_case("desktop.ini")
}

/// Delete `dir`'s OS bookkeeping files if that is all it holds, recursing into
/// subfolders that hold nothing else either, and report whether `dir` is empty
/// now. A folder with anything real in it is left exactly as it was.
///
/// Links are entries of their own and are never followed: a self-referential or
/// escaping directory symlink left in a folder must not turn this into an
/// repeated walk through link targets. Since such a link is not
/// ordinary OS bookkeeping, its presence also keeps the folder.
fn clear_os_junk(dir: &Path) -> bool {
    clear_os_junk_after_scan(dir, || {})
}

fn clear_os_junk_after_scan(dir: &Path, after_scan: impl FnOnce()) -> bool {
    /// Bounded like the rest of the executor: deeper nesting is not a folder
    /// this cleanup should be responsible for, so it stays.
    const MAX_DEPTH: usize = 64;
    fn only_junk(dir: &Path, depth: usize) -> bool {
        if depth == 0 {
            return false;
        }
        let Ok(mut entries) = std::fs::read_dir(dir) else {
            return false;
        };
        entries.all(|entry| {
            let Ok(entry) = entry else {
                return false;
            };
            let path = entry.path();
            let Ok(kind) = entry.file_type() else {
                return false;
            };
            if kind.is_symlink() {
                false
            } else if kind.is_dir() {
                only_junk(&path, depth - 1)
            } else {
                is_os_junk(&path)
            }
        })
    }
    if !only_junk(dir, MAX_DEPTH) {
        return false;
    }
    after_scan();
    fn remove_junk(dir: &Path, depth: usize) -> bool {
        if depth == 0
            || !std::fs::symlink_metadata(dir)
                .is_ok_and(|metadata| metadata.is_dir() && !metadata.file_type().is_symlink())
        {
            return false;
        }
        let Ok(entries) = std::fs::read_dir(dir) else {
            return false;
        };
        let mut empty = true;
        for entry in entries {
            let Ok(entry) = entry else {
                empty = false;
                continue;
            };
            let path = entry.path();
            match entry.file_type() {
                Ok(kind) if kind.is_dir() && !kind.is_symlink() => {
                    // Never bulk-delete a directory validated earlier: another
                    // process may have added real contents since that scan.
                    if !remove_junk(&path, depth - 1) || pinned_paths::remove(&path, true).is_err()
                    {
                        empty = false;
                    }
                }
                Ok(kind) if kind.is_file() && is_os_junk(&path) => {
                    if pinned_paths::remove(&path, false).is_err() {
                        empty = false;
                    }
                }
                _ => empty = false,
            }
        }
        empty
    }
    remove_junk(dir, MAX_DEPTH)
}

/// Resolve `path` (following symlinks, collapsing `..`) and require the result
/// to sit inside one of `roots`. This is what stops a crafted mask literal like
/// `../../etc` from steering a write outside the scanned library. Requires the
/// path to exist, since it canonicalizes the file itself.
fn ensure_within_roots(path: &Path, roots: &[PathBuf]) -> Result<(), PlanError> {
    let canonical = std::fs::canonicalize(path).map_err(PlanError::Io)?;
    if roots.iter().any(|root| canonical.starts_with(root)) {
        Ok(())
    } else {
        Err(PlanError::OutsideRoot(canonical))
    }
}

/// A rename destination that doesn't exist yet can't be canonicalized directly.
/// Resolve it against its nearest *existing* ancestor — which may be several
/// levels up when the target lands in folders that still have to be created —
/// and require that ancestor to sit inside one of `roots`. Returns the resolved
/// absolute target path.
///
/// Canonicalizing the existing part is what makes the containment check
/// meaningful: it collapses `..` and follows symlinks, so a crafted mask can't
/// steer a move outside the library by way of a path that only looks contained.
fn resolve_target_within_roots(target: &Path, roots: &[PathBuf]) -> Result<PathBuf, PlanError> {
    let mut existing = target
        .parent()
        .ok_or_else(|| PlanError::OutsideRoot(target.to_path_buf()))?;
    let mut trailing: Vec<&std::ffi::OsStr> = Vec::new();
    // `target`'s own file name is always part of the not-yet-existing tail.
    trailing.push(
        target
            .file_name()
            .ok_or_else(|| PlanError::OutsideRoot(target.to_path_buf()))?,
    );
    while !existing.exists() {
        let name = existing
            .file_name()
            .ok_or_else(|| PlanError::OutsideRoot(target.to_path_buf()))?;
        trailing.push(name);
        existing = existing
            .parent()
            .ok_or_else(|| PlanError::OutsideRoot(target.to_path_buf()))?;
    }

    let canonical = std::fs::canonicalize(existing).map_err(PlanError::Io)?;
    if !roots.iter().any(|root| canonical.starts_with(root)) {
        return Err(PlanError::OutsideRoot(target.to_path_buf()));
    }
    let mut resolved = canonical;
    for name in trailing.iter().rev() {
        resolved.push(name);
    }
    Ok(resolved)
}

/// Create `dir` and any missing ancestors, returning the directories actually
/// created, shallowest first. Recording exactly what was created lets undo
/// remove those and only those — a directory that already existed is never
/// touched, even if the rollback leaves it empty.
fn create_dirs_recording(dir: &Path) -> Result<Vec<PathBuf>, PlanError> {
    pinned_paths::create_all(dir).map_err(PlanError::Io)
}

/// The rename this change actually performs, if any: `rename_to` unless it's
/// a no-op (equal to the current path).
fn effective_rename(change: &FileChange) -> Option<&Path> {
    change
        .rename_to
        .as_deref()
        .filter(|target| *target != change.path)
}

/// Guard against TOCTOU: if the file's current on-disk value for any changed
/// field no longer matches what the plan recorded as `old`, the plan was
/// built against a stale snapshot and must not be applied.
fn ensure_not_stale(change: &FileChange) -> Result<(), PlanError> {
    with_bound_path(&change.path, false, |path| {
        ensure_not_stale_at(change, path)
    })
}
fn ensure_not_stale_at(change: &FileChange, path: &Path) -> Result<(), PlanError> {
    let current = TagEngine::read(path)?;
    for field_change in &change.tag_changes {
        let on_disk = current.tags.get(&field_change.field);
        if on_disk != field_change.old.as_ref() {
            return Err(PlanError::Stale(change.path.clone()));
        }
    }
    if let Some(cover_change) = &change.cover_change {
        if TagEngine::read_covers(path)? != cover_change.old {
            return Err(PlanError::Stale(change.path.clone()));
        }
    }
    // A block that has changed — or is already gone — since the plan was built
    // would be journaled with a snapshot undo could not put back (#47).
    for block_change in &change.block_changes {
        if TagEngine::read_block(path, block_change.kind)? != block_change.old {
            return Err(PlanError::Stale(change.path.clone()));
        }
        // The modeled content omits private/binary frames and some Vorbis
        // provenance. Compare the same exact snapshot Undo would restore.
        if let Some(old_bytes) = &block_change.old_bytes {
            let current_bytes = match block_change.kind {
                TagBlockKind::Id3v2 => TagEngine::dump_id3v2(path)?,
                TagBlockKind::VorbisComments => TagEngine::dump_vorbis(path)?,
                // Unknown exact snapshot formats must not bypass preflight.
                _ => None,
            };
            if current_bytes.as_ref() != Some(old_bytes) {
                return Err(PlanError::Stale(change.path.clone()));
            }
        }
        // dump_id3v2 deliberately serializes with normalized write options,
        // so the captured source revision needs a separate comparison.
        if block_change.kind == TagBlockKind::Id3v2 {
            if let Some(old_revision) = block_change.old_revision {
                if TagEngine::id3v2_revision(path)? != Some(old_revision) {
                    return Err(PlanError::Stale(change.path.clone()));
                }
            }
        }
    }
    Ok(())
}

/// Refuse a change that sets a custom field under a name the file's tag type
/// cannot store. The backend skips such a name without an error, so the write
/// would report success and the value be gone on the next read. Clearing one
/// (`new` empty or `None`) is always fine. The preview already flags these
/// ([`is_writable_value_at`](crate::model::is_writable_value_at)); this keeps
/// any other plan source from getting one past it.
fn ensure_storable_custom_names(change: &FileChange) -> Result<(), PlanError> {
    for field_change in &change.tag_changes {
        let (TagField::Custom(name), Some(new)) = (&field_change.field, &field_change.new) else {
            continue;
        };
        if !new.is_empty() && !is_storable_custom_name_at(&change.path, name) {
            return Err(PlanError::TagIo(TagIoError::Malformed(format!(
                "the field name {name:?} cannot be stored in {}",
                change.path.display()
            ))));
        }
    }
    Ok(())
}

/// Write this change's blocks (#47, #205). Last of the per-file writes, so the
/// field and image writes above still land on the block they were planned
/// against even when that block is the one going away.
///
/// A conversion's two halves are ordered: every block being written goes first,
/// then the ones being dropped. The other order would leave a file with no tag
/// block at all between the two writes, and a crash in that gap would lose
/// everything rather than half of it.
fn write_blocks(path: &Path, change: &FileChange, direction: Direction) -> Result<(), PlanError> {
    if change.block_changes.is_empty() {
        return Ok(());
    }
    with_bound_path(path, true, |path| {
        let target = |block_change: &BlockChange| match direction {
            Direction::Apply => block_change.new.clone(),
            Direction::Undo => block_change.old.clone(),
        };
        let revision = |block_change: &BlockChange| match direction {
            Direction::Apply => block_change.revision,
            Direction::Undo => block_change.old_revision,
        };
        for block_change in &change.block_changes {
            // A revision switch keeps the block as it is and only restamps the
            // header, so it never goes through the rebuild — see
            // [`TagEngine::set_id3v2_revision`] for why that matters.
            if block_change.is_revision_only() {
                if let Some(revision) = revision(block_change) {
                    TagEngine::set_id3v2_revision(path, revision)?;
                    continue;
                }
            }
            // The same reason, with the frames the rewrite leaves alone. Undo goes on
            // below, to the bytes.
            if block_change.text_to_unicode && matches!(direction, Direction::Apply) {
                TagEngine::id3v2_text_to_unicode(path)?;
                continue;
            }
            if let Some(content) = target(block_change) {
                // Undo puts an ID3v2 block back from its bytes when they were kept
                // (#206), so the frames the model cannot express come back too.
                // Apply never has bytes to write: it is producing a block, not
                // putting one back.
                match (direction, &block_change.old_bytes) {
                    (Direction::Undo, Some(bytes)) if block_change.kind == TagBlockKind::Id3v2 => {
                        TagEngine::restore_id3v2_bytes(
                            path,
                            bytes,
                            &content.covers,
                            block_change.old_revision,
                        )?;
                    }
                    (Direction::Undo, Some(bytes))
                        if block_change.kind == TagBlockKind::VorbisComments =>
                    {
                        TagEngine::restore_vorbis_bytes(path, bytes, &content.covers)?;
                    }
                    _ => {
                        TagEngine::write_block(
                            path,
                            block_change.kind,
                            revision(block_change),
                            &content,
                        )?;
                    }
                }
            }
        }
        for block_change in &change.block_changes {
            if target(block_change).is_none() {
                TagEngine::remove_block(path, block_change.kind)?;
            }
        }
        Ok(())
    })
}

/// Write the `new` image set (or the `old` one on undo). No-op when the change
/// carries no cover change; an empty target set removes every image.
fn apply_cover_change(
    path: &Path,
    change: &FileChange,
    direction: Direction,
) -> Result<(), PlanError> {
    if change.cover_change.is_none() {
        return Ok(());
    }
    with_bound_path(path, true, |path| {
        let Some(cover_change) = &change.cover_change else {
            return Ok(());
        };
        let target = match direction {
            Direction::Apply => &cover_change.new,
            Direction::Undo => &cover_change.old,
        };
        if !matches!(direction, Direction::Undo) || TagEngine::read_covers(path)? != *target {
            TagEngine::write_covers(path, target)?;
        }
        Ok(())
    })
}

fn write_tag_changes(
    path: &Path,
    change: &FileChange,
    direction: Direction,
) -> Result<(), PlanError> {
    write_tag_changes_with_gate(path, change, direction, None)
}
fn write_tag_changes_with_gate(
    path: &Path,
    change: &FileChange,
    direction: Direction,
    gate: Option<FieldGate<'_, '_>>,
) -> Result<(), PlanError> {
    if change.tag_changes.is_empty() {
        return Ok(());
    }
    with_bound_path(path, true, |path| {
        if change.tag_changes.is_empty() {
            return Ok(());
        }
        let mut track = TagEngine::read(path)?;
        if matches!(direction, Direction::Apply)
            && change
                .tag_changes
                .iter()
                .any(|field| track.tags.get(&field.field) != field.old.as_ref())
        {
            return Err(PlanError::Stale(change.path.clone()));
        }
        let kept = if let Some(gate) = gate {
            field_restore_gate_on_tags(&track, change, gate.expected, gate.interrupted, gate.mode)?
        } else {
            Vec::new()
        };
        // An unreached interrupted entry, or an already-restored retry, needs no
        // write. Besides avoiding normalization, this preserves untouched bytes.
        if matches!(direction, Direction::Undo)
            && change
                .tag_changes
                .iter()
                .all(|field| track.tags.get(&field.field) == field.old.as_ref())
        {
            return Ok(());
        }
        for field_change in &change.tag_changes {
            if kept.contains(&field_change.field) {
                continue;
            }
            let value = match direction {
                Direction::Apply => &field_change.new,
                Direction::Undo => &field_change.old,
            };
            match value {
                Some(value) => {
                    track.tags.insert(field_change.field.clone(), value.clone());
                }
                None => {
                    track.tags.remove(&field_change.field);
                }
            }
        }
        TagEngine::write(&track)?;
        Ok(())
    })
}

fn now_unix_secs() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs() as i64)
        .unwrap_or(0)
}

#[derive(Debug, Error)]
pub enum PlanError {
    #[error("file changed on disk since the plan was built: {0}")]
    Stale(PathBuf),
    #[error("file recovery required at {}: {reason}", .path.display())]
    RecoveryRequired { path: PathBuf, reason: String },
    #[error("rename target already exists: {0}")]
    RenameCollision(PathBuf),
    /// Undo found a field that has changed since the batch was applied (#658),
    /// and would have written the old value over that later edit.
    #[error("file changed since the batch was applied: {0}")]
    ChangedSinceApplied(PathBuf),
    #[error("path resolves outside the allowed root: {0}")]
    OutsideRoot(PathBuf),
    #[error("journal error: {0}")]
    Journal(#[from] JournalError),
    #[error("tag I/O error: {0}")]
    TagIo(#[from] crate::model::TagIoError),
    #[error("I/O error: {0}")]
    Io(#[source] std::io::Error),
    /// The apply was cancelled between two files and rolled back (#437).
    #[error("apply cancelled")]
    Cancelled,
    /// A failed apply could not be fully rolled back (#442): `count` files,
    /// folders or sidecars are not as they were, `first` being the first.
    #[error("{cause}; {count} item(s) could not be put back, the first: {}", .first.display())]
    RollbackIncomplete {
        cause: Box<PlanError>,
        count: usize,
        first: PathBuf,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    fn change_for(path: &Path, artist: &str) -> FileChange {
        FileChange {
            path: path.to_path_buf(),
            tag_changes: vec![FieldChange {
                field: TagField::Artist,
                old: None,
                new: Some(artist.to_string()),
            }],
            ..FileChange::default()
        }
    }

    fn temp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("tagrex-plan-{tag}-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// A file the plan reached but that is no longer there cannot be put back:
    /// the rollback has to say so.
    #[test]
    fn a_rollback_that_cannot_restore_a_file_reports_it() {
        let dir = temp_dir("incomplete");
        let gone = dir.join("gone.flac");
        let plan = ChangePlan {
            changes: vec![change_for(&gone, "A")],
            ..ChangePlan::default()
        };
        let mut written = Written::new(1);
        written.tag_steps[0] = 3;

        let error = written.roll_back_after(
            &plan,
            &[],
            PlanError::Io(std::io::Error::other("x")),
            &mut crate::journal::VecJournal::new(),
            BatchId(0),
        );

        match error {
            PlanError::RollbackIncomplete { count, first, .. } => {
                assert_eq!(count, 1);
                assert_eq!(first, gone);
            }
            other => panic!("expected RollbackIncomplete, got {other:?}"),
        }
        std::fs::remove_dir_all(&dir).ok();
    }

    /// Nothing was reached, so there is nothing to put back and nothing to report.
    #[test]
    fn an_untouched_plan_rolls_back_silently() {
        let dir = temp_dir("untouched");
        let plan = ChangePlan {
            changes: vec![change_for(&dir.join("never.flac"), "A")],
            ..ChangePlan::default()
        };
        let mut written = Written::new(1);

        let error = written.roll_back_after(
            &plan,
            &[],
            PlanError::Io(std::io::Error::other("x")),
            &mut crate::journal::VecJournal::new(),
            BatchId(0),
        );

        assert!(matches!(error, PlanError::Io(_)));
        std::fs::remove_dir_all(&dir).ok();
    }

    /// A transfer onto an existing file is refused, unless that file is the one
    /// being moved (#605).
    #[test]
    fn a_transfer_target_must_be_free_or_the_file_itself() {
        let dir = temp_dir("target-free");
        let from = dir.join("a.flac");
        let other = dir.join("b.flac");
        std::fs::write(&from, b"a").unwrap();
        std::fs::write(&other, b"b").unwrap();

        assert!(ensure_target_free(&from, &dir.join("free.flac"), false).is_ok());
        assert!(ensure_target_free(&from, &dir.join("free.flac"), true).is_ok());
        assert!(matches!(
            ensure_target_free(&from, &other, false),
            Err(PlanError::RenameCollision(_))
        ));
        assert!(matches!(
            ensure_target_free(&from, &other, true),
            Err(PlanError::RenameCollision(_))
        ));
        // A move onto itself is a rename in place; a copy onto itself would
        // truncate the source.
        assert!(ensure_target_free(&from, &from, false).is_ok());
        assert!(matches!(
            ensure_target_free(&from, &from, true),
            Err(PlanError::RenameCollision(_))
        ));
        // A second name of the same file is a name of its own: renaming onto
        // it would do nothing and report success (#707).
        let link = dir.join("link.flac");
        std::fs::hard_link(&from, &link).unwrap();
        assert!(matches!(
            ensure_target_free(&from, &link, false),
            Err(PlanError::RenameCollision(_))
        ));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn pruning_cannot_remove_a_parent_outside_the_recorded_intent() {
        let dir = temp_dir("bounded-pruning");
        let album = dir.join("artist/album");
        let scans = album.join("Scans");
        std::fs::create_dir_all(&scans).unwrap();
        let roots = canonical_roots(std::slice::from_ref(&dir)).unwrap();
        let removed = prune_emptied_dirs(std::slice::from_ref(&scans), &roots, &[]);
        assert_eq!(removed, vec![scans.clone()]);
        assert!(!scans.exists());
        assert!(
            album.is_dir(),
            "an eligible but unrecorded parent must remain"
        );
        assert!(dir.join("artist").is_dir());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn pruning_preserves_a_root_reached_through_a_parent_symlink() {
        let dir = temp_dir("pruning-root-alias");
        let real_parent = dir.join("real");
        let library = real_parent.join("library");
        std::fs::create_dir_all(&library).unwrap();
        let alias_parent = dir.join("alias");
        std::os::unix::fs::symlink(&real_parent, &alias_parent).unwrap();
        let aliased_root = alias_parent.join("library");
        let roots = canonical_roots(std::slice::from_ref(&library)).unwrap();
        let removed = prune_emptied_dirs(std::slice::from_ref(&aliased_root), &roots, &[]);
        assert!(removed.is_empty());
        assert!(library.is_dir(), "an allowed root must never be pruned");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn pruning_does_not_follow_a_directory_symlink_cycle() {
        let dir = temp_dir("junk-symlink-cycle");
        let album = dir.join("album");
        std::fs::create_dir_all(&album).unwrap();
        std::fs::write(album.join(".DS_Store"), b"junk").unwrap();
        std::os::unix::fs::symlink(&album, album.join("loop")).unwrap();

        // Following `loop` would recurse through the same folder forever; the
        // link is not ordinary bookkeeping, so the folder simply stays.
        assert!(!clear_os_junk(&album));
        assert!(album.is_dir());
        assert!(album.join(".DS_Store").exists());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn os_bookkeeping_still_clears_an_otherwise_empty_folder() {
        let dir = temp_dir("junk-cleared");
        let album = dir.join("album");
        std::fs::create_dir_all(&album).unwrap();
        std::fs::write(album.join(".DS_Store"), b"finder").unwrap();
        std::fs::write(album.join("Thumbs.db"), b"explorer").unwrap();

        assert!(clear_os_junk(&album));
        assert_eq!(std::fs::read_dir(&album).unwrap().count(), 0);
        std::fs::remove_dir_all(&dir).unwrap();
    }
    #[test]
    fn pruning_preserves_real_files_added_after_the_initial_junk_scan() {
        let dir = temp_dir("junk-concurrent-real-file");
        let nested = dir.join("album/nested");
        std::fs::create_dir_all(&nested).unwrap();
        std::fs::write(nested.join(".DS_Store"), b"junk").unwrap();
        let album = dir.join("album");
        assert!(!clear_os_junk_after_scan(&album, || {
            std::fs::write(album.join("new.txt"), b"new root contents").unwrap();
            std::fs::write(nested.join("new.flac"), b"new audio contents").unwrap();
        }));
        assert_eq!(
            std::fs::read(album.join("new.txt")).unwrap(),
            b"new root contents"
        );
        assert_eq!(
            std::fs::read(nested.join("new.flac")).unwrap(),
            b"new audio contents"
        );
        std::fs::remove_dir_all(dir).unwrap();
    }
}
