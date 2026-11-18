# Protected FLAC publication checkpoint

Status: local checkpoint ready; exact-head hosted checks remain required before merge.

## Scope

The first production route covers whole plans of in-place plain-text FLAC edits
on supporting Linux profiles with the SQLite journal. It preserves native text
lists outside the edited targets, publishes through held-file recovery evidence,
and records selective persistent Undo receipts. Unsupported profiles, typed
numeric/date edits, mixed artwork/block/transfer plans and other formats select
the existing route before any native journal/media effect.

The publication includes the reviewed shared file-authority and copy/move
recovery prerequisites used by this implementation. Legacy copy/move history
without strong ownership remains readable but cannot authorize unproved deletion
or relocation. It also isolates metadata interpretation per App and bounds
compound native IO admission without raising resource quotas.

The exact Stage/Review DTO migration, native picture/UI integration, broader
cache/player/import/report changes and the stress-runner implementation remain
parked. No live library/profile/Undo state was used as a fixture. No new UI
capability or release package follows from this backend checkpoint.

## Review and verification

Independent coordinator review corrected duplicate native file identities,
rollback of a completed prefix when a later source never started, operation-role
tracking across interrupted recovery, and the final strict-Undo callback check.
Independent journal review corrected impossible initial recovery state,
transactional completion validation, migration downgrade refusal, and an
independent whole-batch commitment that survives child-row deletion.

Final independent policy review accepted 17 probes after correcting rollback
after an operation's settings change and stale display/query cache admission.
Independent admission review accepted 54 tests after correcting private-sink
reentry before public callbacks/history. Original failing canaries remain.
Retained query snapshots from invalid scopes are immediately invalid for workers.
An operation-local recovery policy cannot authorize new public Apply/Undo.

Combined local verification on the frozen production source (base `c42a6cea`):

- `cargo test --workspace`: 1,598 passed, zero failed, 34 existing optional/manual
  tests ignored across 85 outer targets. Nested helper results are not added.
- `cargo clippy --workspace --all-targets --all-features -- -D warnings`: passed.
- `cargo fmt --all --check` and `git diff --check`: passed.
- Private D-Bus MPRIS tests: two passed.
- UI check: zero errors/warnings; lint: 94 files, zero problems; unit tests: 216
  passed; production UI build passed. Dependencies/toolchain were reused.
- Real eight-App, three-round Apply/Undo/Redo/Undo pressure test passed with
  distinct settings and complete resource-owner release.

[Coordinator review](backend-native-flac-executor-independent-review.md),
[journal review](backend-native-flac-journal-independent-review.md),
[policy review](backend-native-policy-independent-review.md) and
[admission review](backend-native-admission-independent-review.md) retain ranked
findings, original probes, source hashes and limits. Earlier component counts
overlap and must not be added together. The coordinator/journal reports record
their earlier protocol freeze; subsequent policy/admission changes have their
own reviews and the retained protocol tests passed in the combined gate.

The original unconditional `metadata_preservation_recheck.rs` remains unchanged
in the parked programme, with separate Linux evidence. It was not weakened or
platform-disabled to enter this publication. Linux native-route preservation
probes are included here; portable legacy-path preservation remains open.
Windows/macOS compile/refusal coverage is required but does not establish native
protected-write support there. Hosted results and the publication commit belong
in the linked PR; green local checks alone do not authorize merging.

## Journal compatibility and upstream route

Native history uses a versioned, signed schema and old-reader refusal boundary.
Released legacy scalar history stays readable. An older binary is not a safe
rollback route once native history exists; retain compatible code and recovery
material. Original image/receipt authority remains necessary for restoration.

The reusable backend work is intended for `tagrex/tagrex:main`. Submission is
waiting for compatible native-field/text-codec foundations absent upstream at
this checkpoint; the fork's pooled journal representation must not replace
upstream inline artwork images or remove its waveform API. No upstream PR or
acceptance is implied.

## Pause and resumption

After this slice and its documentation merge, pause all remaining backend work.
Resume through GitHub issue #51 and the programme handoff. Curate subsequent
slices from then-current merged main. The parked whole-programme archive is a
historical source snapshot and must not be applied wholesale over those merges.

## Hosted compatibility correction

The first hosted head (`f7e25856`) stopped in Clippy on Windows/macOS. Hosted
Rust 1.99 deprecates `fetch_update`; the local shared toolchain is 1.98.1. The
replacement `try_update` would raise this code's documented 1.90 minimum, so
these two new atomic updates use compare-exchange instead. Saturating generation
and bounded IO-slot accounting retain their ordering and quota behavior.
See the [official atomic API](https://doc.rust-lang.org/stable/core/sync/atomic/type.AtomicU64.html).

Darwin's native device identifier differs from std's cross-Unix representation.
The held-source rename check now compares native `fstat` and `statat` identity
fields directly on both sides, avoiding casts or a narrowed ownership predicate.
Focused policy/admission/executor/move probes pass. The follow-up's full workspace
again passed 1,598 tests (zero failures, 34 existing optional/manual ignored),
workspace all-target/all-feature Clippy and formatting. Exact-head hosted gates
remain required before merging.
Earlier independent reports retain their original frozen hashes; these small
CI compatibility corrections are recorded separately here.

Follow-up source SHA-256 values:

- `crates/core/src/model/policy.rs`: `75e1ab3ca940bf85612f95b65577518506b104e4f3ffeb360d1cb5080ded3401`
- `crates/core/src/plan/native_flac.rs`: `333a71ca9d2363adbd046d0f56b00e01639650d2c7de29c985d2b96301204729`
- `crates/core/src/plan_paths.rs`: `d39602e8d0071a864c94524e52f83de99437fb97a635392ba85e00d39ed6e629`
