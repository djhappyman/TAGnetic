# Completed-copy checkpoint: independent review

Reviewed the frozen candidate and the orchestrator's corrections on disposable
Linux fixtures. This is a review of completed-copy checkpoint/recovery behavior,
not approval of the entire backend or the separate protected metadata writer.
Reusable backend changes are eligible for a curated `tagrex/tagrex:main` PR;
upstream inline journal images and the stable waveform interface remain separate.
No production code, active profile, runtime, user media or existing tests were
changed by this reviewer. Reviewer-owned files are
`crates/core/tests/copy_checkpoint_independent.rs` and this report.

## Ranked finding and correction

**P2 — duplicate copy evidence caused partial Undo before a known ambiguity was
reported, and reservation refusal could mutate that evidence. Resolved and
independently rechecked.**

The original candidate indexed `batch.copy_stamps` using
`entry(path).or_insert(first)`. With three copied audio files and their sidecars,
inserting a conflicting duplicate row for the last sidecar let Undo remove
previous outputs before the eventual duplicate-row reservation error. The test's
`Undo` call returned an error; the preservation assertion then failed because an
earlier copied audio file was already absent. No foreign-file deletion was
observed in this reproduction: the error was avoidable partial recovery against
an ambiguity present before Undo began.

Two narrower public-API reproductions established the reservation problems:
SQLite updated both identical rows' `undo_path`, then reported its row-count
error without rolling back; `VecJournal` accepted the duplicate and changed the
first matching row. These are retained as three normal regression tests, not
ignored or weakened. Their original red logs remain available.

The orchestrator corrected production code:

- Executor rejects duplicate transfer-evidence paths before move recovery,
  callbacks, path mutation or copy removal. Identical duplicates also refuse.
- SQLite validates uniqueness and reserves the removal path in one transaction;
  refusal leaves the stored evidence unchanged.
- VecJournal validates uniqueness, strong digest, non-move role and existing
  reservation agreement before changing its in-memory entry.

The original three red probes now pass. The integration probe also checks
identical and conflicting duplicates under Strict, KeepChanged and Overwrite:
all six combinations refuse with zero callbacks, every copied output intact,
unchanged source bytes and unchanged complete proof collection.

## Independent probes

The new target has **13 passing functions**, including one child-process entry
and twelve actual parent probes. Besides the three regressions above, it checks:

- A same-inode encoded-audio change after transfer but before metadata is not
  adopted by the refreshed checkpoint. Apply refuses before a Tags completion,
  preserves the changed copy and interrupted history, and all three Undo modes
  refuse deletion. The selected field remains its original value.
- An externally changed final sidecar blocks all earlier copy removals in all
  three Undo modes, with zero callbacks.
- Missing, legacy, directory-role and future native proof variants in the final
  sidecar block all prior output removal under every Undo mode.
- A byte-identical replacement installed by an Undo callback survives alongside
  the saved original and sidecar; history remains available.
- A separate hardlink to a copied file survives ordinary reopened-journal Undo
  with exactly unchanged bytes. Undo removes only its recorded copied names.
- SQLite and VecJournal agree on cross-batch refusal, stale refresh tokens,
  same-object refresh and idempotent repeated acknowledgements.
- An unsupported foreign journal refuses a sidecar-only copying plan before
  recording or callbacks. Its ordinary metadata Apply/Undo workflow still works.
- A real SQLite AFTER UPDATE trigger first inserts an audit row, then raises
  ABORT during metadata checkpoint refresh. The checkpoint emits no Tags
  completion; SQL audit remains empty, owned copies roll back, sources remain
  byte-exact and earlier history survives reopening.
- A real child exits after the second metadata completion in a three-audio,
  three-sidecar batch. The reopened interrupted journal contains six actual
  current proofs, including refreshed and not-yet-edited copies. One Undo safely
  removes all six copies and retains every source.

Synthetic FLAC/Vorbis comments are parsed directly by the test, independently of
TagEngine readback. It checks the selected artist, unrelated Unicode title and
exact encoded audio-frame bytes. This fixture verifies the exercised FLAC
copy/metadata path; it is not a format coverage claim for MP3, MP4 or all audio.

## Final verification

On the final source hashes below:

- Nine integration binaries: **161 passed, 0 failed, 3 existing ignored**.
- Core library units: **276 passed, 0 failed, 2 existing ignored**, including the
  same-connection genuine SQLITE_FULL checkpoint probe.
- Combined retained result: **437 passed, 0 failed, 5 existing ignored**.
- Default core test Clippy and scoped all-features Clippy passed without lint
  exemptions. Core formatting passed.

```sh
source /mnt/Games/Development/tagrex/dev-env.sh
cargo test -p tagrex-core --test copy_checkpoint_independent --test copy_checkpoint --test recovery_stress --test sqlite_journal --test executor --test move_staging_adversarial --test move_context_recheck --test unix_u2_recovery_recheck --test move_cleanup_index_adversarial --locked -- --nocapture
cargo test -p tagrex-core --lib --locked
cargo clippy -p tagrex-core --tests --locked -- -D warnings
cargo clippy -p tagrex-core --test copy_checkpoint_independent --all-features --locked -- -D warnings
cargo fmt --check -p tagrex-core
```

Compact evidence under `/mnt/Games/Development/tagrex/`:

- `copy-checkpoint-independent-final-gate.log`
- `copy-checkpoint-independent-unit.log`
- `copy-checkpoint-independent-core-clippy.log`
- `copy-checkpoint-independent-clippy.log`
- `copy-checkpoint-independent-format.log`
- Original reds: `copy-checkpoint-independent-duplicate.log`,
  `copy-checkpoint-independent-reservation.log`,
  `copy-checkpoint-independent-memory-duplicate.log`.

```text
cd881f16e9c8bab88d9a985582b0ab175551d26b5a0ae235a01939e5955b0d45  crates/core/src/plan.rs
9954adac6057a9154ece3e4394f66f379fa8a9ec8586e32f1871adaad5db7c9a  crates/core/src/journal.rs
bdd4d844b6c3f53a5638aa23c82eaa8f4e9648cd0a9f5ccc658f3956a6976544  crates/core/src/plan_paths.rs
30c1c5bd9ae70408209791c9af8c8be2802fbb6078d7c4323a11bee63505314a  crates/core/tests/copy_checkpoint_independent.rs
```

The graph project `tagrex-backend-readiness`, generation
`2026-11-18T12:48:55Z`, was confirmed at the intended checkout. Its three old
CopyStamp/UndoJournal/SqliteJournal symbols omit the new checkpoint methods;
coverage marks production as changed and new tests as untracked. Actual source
reads supplied the evidence instead of treating stale graph snippets as current.
The cleanup-index implementation was not re-audited; its source hash stayed
unchanged and its existing normal regression target passed.

## Remaining limits

No unresolved blocker remains within this bounded Linux completed-copy scope.
The synchronous busy-SQLite path still waits through configured busy handling;
a retained run observed about ten seconds after the reservation transaction
change, compared with the earlier author's approximately fifteen-second run.
This is not a prompt cancellation or latency contract. No timing policy changed.

A copy interrupted before its durable checkpoint, or metadata interrupted before
its refreshed checkpoint, remains unknown and must retain output and history.
The tests establish actual process-death/reopening behavior, not physical
power-loss guarantees. Native macOS/Windows execution, network filesystems,
filesystems lacking usable native birth identity, concurrent third-party writes
inside a metadata syscall, full protected-writer integration and UI exposure
remain separate gates. Five existing manual probes were ignored in this retained
gate and are not counted as independently rerun here.

Fixture guards removed every reviewer-owned temporary media tree, including the
child-process fixture after it was reaped. Useful red/green evidence remains;
no persistent runtime state was created or cleaned.
