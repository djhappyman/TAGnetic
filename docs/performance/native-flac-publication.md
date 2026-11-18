# Protected FLAC publication checkpoint

Status: local checkpoint ready after descriptor IO compatibility and independent rechecks; exact-head hosted checks remain required before merge.

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

- `cargo test --workspace --no-fail-fast`: 1,638 passed, zero failed, 34 optional/manual
  tests ignored across 89 outer targets. Three newly added child dispatchers are
  included transparently in the pass count. Nested helper results are not added.
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
[admission review](backend-native-admission-independent-review.md),
[portable boundary review](backend-portable-policy-independent-review.md) and
[process-death harness review](backend-process-death-harness-review.md) and
[descriptor IO review](backend-descriptor-io-independent-review.md) retain ranked
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
Rust deprecates `fetch_update` since 1.99; the local shared toolchain is 1.98.1. The
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

The next hosted Windows lint gate identified a constant capability assertion
in the non-Linux refusal test. It now evaluates in a const block, preserving
the assertion and the actual runtime unsupported-Store checks. No production
behavior, test exclusion or admission quota changed in that follow-up.

A subsequent Windows all-target lint identified `TagField` imported globally
for a Unix-only probe. The import now lives inside that unchanged Unix test;
no test was removed, disabled or given a broader allow attribute.

Read-only portability audit found a P3 in the optional move profiler: flushing
an owned fixture opened read-only fails on Windows. The profiler now opens its
owned fixture read/write before syncing; assertions and default opt-in status
are unchanged. [Microsoft's FlushFileBuffers contract](https://learn.microsoft.com/en-us/windows/win32/api/fileapi/nf-fileapi-flushfilebuffers)
requires write access. This is a test-harness correction, not new native Windows
protected-write support.

The read-only portability review covered all 35 added core test targets: nine
whole-crate Linux targets, three Linux-scoped modules (two with portable opt-out
probes), two whole-crate Unix targets and 21 portable crates with item guards.
It found no further confirmed cfg/import/helper gap after the known import and
P3 profiler findings. This is static source evidence, not Windows/macOS execution.

The corrected exact-read adversarial target passed four tests. The existing
explicit `actual_journal_and_directory_layout_costs` profile also passed: 1,000
owned synthetic WAV fixtures in each shared/fanout layout, data-advised and warm
repeat cases, complete restored bytes/history cleanup and four extra descriptors
observed at progress callbacks. It finished in 25.06 seconds locally. Metadata
cache state was uncontrolled; this is not a real-library throughput measurement.
Workspace all-target/all-feature Clippy, formatting and diff checks pass. The
production source is unchanged from the 1,598-test compatibility gate; current
hosted Windows/macOS/Linux/advisory results remain mandatory.

## Shared operation policy correction after Windows execution

Hosted Windows at `96480a82` reached the unchanged App policy/panic canary and
failed: the native route refused invalidation but portable legacy Apply did not.
Real WAV/SQLite and FLAC/VecJournal probes reproduced the shared boundary gap on
Linux. A further independent SQLite probe exposed a native final-completion
callback bypass. Both P2 groups are corrected in the new frozen source.

The common entry pins inherited App settings or standalone defaults. Policy
checks surround public progress, material journal calls and writes. Failed or
panicking Apply makes ownership-checked rollback under a sealed admitted snapshot;
existing field/block/source/root witnesses remain. Legacy Undo retains its
incremental contract and history on refusal before the next effect. Successful
terminal history removal is its commit boundary; arbitrary adapter behavior
after deletion cannot be transactionally recreated through the old trait.

[Portable boundary review](backend-portable-policy-independent-review.md)
accepted 36 distinct tests: eighteen new probes plus retained policy/App cases
and the original unconditional App canary. The root App/WAV bridge additionally
checks staged-plan preservation, byte/history readback and retry through
Apply/Undo/Redo/Undo. These counts overlap the workspace gate.

Surviving Rust unwind now performs owned rollback. Crash scenarios therefore
use actual disposable child termination with exact exit status, not panic as a
stand-in for death. Their original restart/foreign-data recovery assertions stay
retained. A nested unwind case still checks same-thread outer-context restoration
and now explicitly verifies automatic inner cleanup. Independent process-death review accepted all three targets: 46 raw tests,
comprising 43 behavior tests and three default no-op dispatchers. All original
foreign-object/whole-collection assertions remain; each child must exit exactly
73, with a 30-second watchdog, kill/reap cleanup and owned logfile diagnostics.
No platform gate or existing test was ignored to obtain green results.

The final source passed the complete workspace (1,623/0/34), workspace all-target/
all-feature Clippy, formatting, diff checks and two private D-Bus tests. The
original Windows canary remains unchanged. Earlier 1,598-test results elsewhere
in this report describe previous frozen checkpoints; the latest combined gate
is 1,623. Actual Windows/macOS execution remains an exact-head hosted merge gate.
Future stress-runner review must reconcile the true-death/renamed nested-unwind
probe mapping against current main instead of copying old panic fixtures.


## Descriptor IO correction after macOS execution

macOS on `96480a82` exposed 54 command failures, predominantly unknown-format
writes. Darwin descriptor aliases duplicate a shared file position; a previous
read can leave the next writer at EOF. Artificial aliases also lose logical
filename hints. The correction retains the logical name and supplies fresh file
sessions through pinned parent authority, comparing each opened object against
the held source before IO. Matching binding failures cannot fall back to an
unchecked pathname. Exact candidate sources retain precedence.

Independent review also challenged unrelated peer requests inside an operation.
They retain their original standalone behavior instead of inheriting another
App's root. The original command tests remain unchanged. There is no hardlink
workaround, added dependency or new whole-file copying workaround.

[Descriptor IO review](backend-descriptor-io-independent-review.md) records 31
passing scoped tests, primary Apple/Lofty sources and the frozen hashes. It
covers mixed MP3/FLAC/WAV tags/artwork with encoded-audio readback, logical-hint
fallback, EOF, held-source precedence, foreign replacement, aliases, nested peer
operations and unwind cleanup. Linux shared-description emulation proves the
library mechanism; it does not replace actual macOS execution. Exact-head hosted
Linux, Windows, macOS and advisory gates remain required before merging.


Final combined verification on the descriptor IO freeze passed **1,638 workspace
tests, zero failed, 34 optional/manual ignored across 89 outer targets**. Three
child dispatchers are included, with nested results excluded from the count.
Workspace all-target/all-feature Clippy, formatting, diff checks and two private
D-Bus MPRIS tests passed. The production hashes match the independent descriptor
review. The earlier 1,623 gate above records the previous common-policy freeze;
1,638 is the current combined gate. Actual macOS runtime confirmation remains
pending the new hosted head; no broader platform/format guarantee follows.
