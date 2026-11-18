# Native FLAC Executor: fresh independent review

**Checkpoint ready within the bounded Linux coordinator scope. Four P2 finding groups were corrected and independently rechecked.** This is a
bounded Linux coordinator review, not whole-backend or cross-platform approval.
The reviewer changes only `crates/core/tests/native_flac_executor_independent.rs`
and this report. No production edits, commits, runtime/profile changes, user
media or existing Undo writes belong to this review.

## Scope and contribution route

Reviewed the new ordinary Executor whole-plan in-place plain-text FLAC route:
trusted capture/revalidation before effects, held-source/path authority,
protected per-file publication/retirement, failure rollback and all three Undo
conflict modes. A separate reviewer owns the native journal compatibility and
storage protocol. Reusable backend changes are eligible for curated
`tagrex/tagrex:main`; preserve upstream inline artwork journal images and the
stable waveform API. Publication remains the orchestrator’s responsibility.

Graph Tier 2 used project `tagrex-backend-readiness`, generation
`2026-11-18T12:48:55Z`, confirmed correct programme root. Native coordinator
search returned no graph node. Exact coverage reports `plan.rs` changed and the
new coordinator, native ledger, media/writer modules and tests untracked.
Actual current source was read directly. The graph cannot certify this new route.

## Ranked findings

### P2, corrected and original probe green: native inode aliases

Two FileChanges named distinct hardlinks to the same FLAC inode, with the same
original Title and different desired Titles. Capture deduplicated lexical paths
but not native identity. The first alias was published; the second then failed
its baseline, and rollback refused the conflicting receipt-less alias. Apply
returned an error while both names exposed the first changed Title and history
remained interrupted. This is an avoidable partial Apply: the alias conflict was
observable before initial journaling or media mutation.

Retained regression:
`duplicate_native_inode_aliases_refuse_before_publication_or_history`.
The unchanged test requires rejection with original bytes at both names and no
batch. Its original red log is retained below. No foreign-file deletion was
observed in this reproduction.

### P2, corrected: an unreached external row blocked owned rollback

After the first file completed, a public progress callback independently edited
the still-unreached second file’s Title and requested cancellation. Apply
returned an error, but the first file retained the app’s applied Title. Whole-
collection `validated()` demanded the receipt-less later row still equal its
old intent, so rollback never reached the earlier proved write. External bytes
must remain protected without preventing exact restoration of earlier owned
work; incomplete external recovery/history must remain explicit.

Retained regression:
`failed_apply_rolls_back_owned_earlier_file_when_unreached_target_was_edited`.
It checks earlier exact tag restoration and byte-identical later external data.
This case is a valid never-started row, not malformed journal evidence; fixing
it must not weaken malformed-collection refusal before effects.

The orchestrator corrected identity deduplication before initial effects and
handling of a readable, same-inode external field on an unreached row. Both
original regression probes now pass; the latter explicitly requires
`RollbackIncomplete` and retained history.

The requested broader variants also reproduced the defect: deleting the unreached source,
replacing it with a byte-identical inode, or writing malformed bytes there each
causes a cancelled Apply to leave the earlier owned Title applied. The new
`inaccessible_or_replaced_unreached_sources_do_not_block_owned_rollback` probe
runs all three variants and checks preserved external bytes/absence and history.
This is the same incomplete-rollback consequence through bind/read failures.
The corrected coordinator validates every durable proof first, then permits
automatic Apply rollback to preserve/report only genuinely pristine, never-
published rows with missing/foreign/unreadable sources. Their operation IDs
remain checked against the complete Store inventory before any earlier restore.
The unchanged three-variant probe now passes, with history retained.

### P2, corrected: persisted rollback begin changed the role of retained receipts

A public forwarding NativeJournal first refuses history removal after a
successful Overwrite Undo, forcing rollback-of-Undo. It then returns an error
immediately after the real SQLite `begin(operation=2)` commit, before any new
Prepared acknowledgement. Both boundaries are asserted reached. The journal
has retained its prior operation-1 inverse receipts, but labels the operation 2
and clears its record. Ordinary reopened Undo refuses `native recovery receipt
binding`: those prior receipts have an external old value, not the historical
old state required by the new operation-2 interpretation.

Retained regression:
`persisted_rollback_begin_before_prepared_keeps_reopened_undo_usable`.
This is a genuine durable transition/failure case; no SQL rows are fabricated.
The source has the historical old value and history is retained, but a normal
retry is unnecessarily made unusable. The journal reviewer identified the
source-level concern and this coordinator review independently reproduced it.
The journal now persists the recovery receipt role separately from the newly
begun operation; new Prepared acknowledgements replace that role atomically.
The original durable-begin regression now passes through reopened ordinary Undo.

### P2, corrected: Strict Undo accepted stale no-op selection after a callback

One additional bounded probe was requested by the orchestrator after a source
observation. The target had already been restored to its historical old value,
so preflight selected no write. The public Undo step callback then changed that
same Title. Strict Undo returned `Ok(UndoReport { kept: [] })` and deleted history
without checking the now-stale no-op selection. External bytes were not
overwritten, but the Strict outcome and history removal were incorrect.

Retained regression:
`strict_undo_rechecks_a_skipped_target_after_its_public_callback`.
The actual red log is `native-flac-executor-independent-skipped-callback-red.log`.
An earlier transitional compilation failure is not counted as that finding.
A fresh post-callback identity and exact target-state check now runs before
Strict pending recovery/restoration, including no-op and receipt-less targets.
The original callback probe now refuses with retained history and unchanged
external bytes.

## Executed independent probes

The initial seven-test gate was **5 passed, 2 failed**. After the first fixes
and two added recovery probes, the nine-test gate was **7 passed, 2 failed**.
The final bounded callback probe brings the independent binary to ten tests;
its original red was captured separately. The final corrected binary now passes
**10 tests**, with none ignored, on the frozen source hashes below. No independent probes were ignored or
weakened. The other independent probes pass:

- Composed Title/Artist Apply and reopened Undo under every conflict mode,
  retaining literal old Artist, repeated/empty unrelated values introduced
  through an unplanned hardlink, and the original media inode.
- A policy change at the last preflight callback refuses before initial native
  record/storage creation and preserves both source files byte-for-byte.
- A completion callback panic after first publication rolls back reached work,
  releases ownership and permits actual Apply/Undo retry.
- A byte-identical replacement of the later source refuses whole Undo in all
  modes before any callback, leaving both outputs and history intact.
- A late external edit after an earlier Undo step survives rollback when a
  later source is replaced; foreign replacement and history remain intact.

Raw FLAC builders/inspectors are independent of TagEngine. Synthetic frames
exercise metadata coordination, not a new real-audio codec preservation claim.
All fixture directories are disposable and removed by their owner guards.

```sh
source /mnt/Games/Development/tagrex/dev-env.sh
cargo test -p tagrex-core --test native_flac_executor_independent --test native_flac_executor --test metadata_preservation_recheck --test recovery_stress --test executor --test sqlite_journal --locked
cargo clippy -p tagrex-core --lib --test native_flac_executor_independent --all-features --locked -- -D warnings
rustfmt --edition 2021 --check crates/core/tests/native_flac_executor_independent.rs
```

Final retained gate: **135 passed, 0 failed, four existing manual tests ignored**
across six integration binaries (48 Executor, one unchanged preservation canary,
22 authored native coordinator, ten independent coordinator, 25 recovery stress
and 29 SQLite). Child summaries are not counted twice. Scoped all-features
Clippy and owned formatting pass. Coordinator, native-ledger and parent-journal
hashes were identical before/after the independent retained run. Compact evidence under
`/mnt/Games/Development/tagrex/tmp/`:
`native-flac-executor-independent-first.log` (original two reds),
`native-flac-executor-independent-expanded.log` (five pass/two fail), and
`native-flac-executor-independent-clippy.log`. Later retained reds are
`native-flac-executor-independent-begin.log` and
`native-flac-executor-independent-unreached.log`; the full nine-test result is
`native-flac-executor-independent-second.log`.

## Candidate hashes at initial review

```text
4cd7b00264a0773dcbee0c780af3a446de1fa5b013988d5e90b3e5e0808880b4  crates/core/src/plan.rs
58c1ad92a665a8f51e87c6cb642019ab07cdb111804a3fb32a41c37444a6e93a  crates/core/src/plan/native_flac.rs
b49cc1c672855bb48c04891310dbc5c42aed0eb8f9c5a58fbf5bc95610fc1cd2  crates/core/src/media_mutation.rs
8ccfa4a88fceabc73adba77dcd8df7af2e0b1852156f8cc6f40a1a07294a948c  crates/core/tests/native_flac_executor_independent.rs
```

## Final independently rechecked hashes

```text
4cd7b00264a0773dcbee0c780af3a446de1fa5b013988d5e90b3e5e0808880b4  crates/core/src/plan.rs
db0a348d54547326ecc0ce6012549fd84f2d12a9e71fe7de98d0d3360ff17d34  crates/core/src/plan/native_flac.rs
c29348b001a1b80ba4809a5307da8f36cff012cfcb48a3f78aebb63d0c206fce  crates/core/src/journal.rs
060c9d920204296134f6fb6903685990205d35dedc01b043bdede8c3bebbf512  crates/core/src/journal/native.rs
b49cc1c672855bb48c04891310dbc5c42aed0eb8f9c5a58fbf5bc95610fc1cd2  crates/core/src/media_mutation.rs
3c5718eea622dcfadb7edc1424d3ab8557f9b7ca594aef14b70b739ef5ae35da  crates/core/tests/native_flac_executor_independent.rs
```

No unresolved defect was found within this bounded coordinator review. The
separate journal review remains responsible for its schema/compatibility/proof
collection conclusions. Final compact evidence is
`tmp/native-flac-executor-independent-final.log`,
`native-flac-executor-independent-clippy.log`, and
`native-flac-executor-independent-source-before.txt` under the shared tagrex
folder, alongside the original red logs listed above.

Native Windows/macOS, power loss, NAS, all-format/mixed/typed operations, full UI
exposure, whole-process RSS and physical audio devices remain outside this
review. The new fixtures use small synthetic FLACs and raw metadata/opaque/audio
payload inspection; they do not establish multi-gigabyte production throughput.
Existing manual real-media/historical-reader tests were not rerun in this
coordinator gate and retain their separate evidence. Broader limits remain as
specified in the author readiness report. No production source, existing
acceptance assertion or resource ceiling was changed by this reviewer.
