# Protected FLAC publication checkpoint

Status: local checkpoint ready after descriptor IO and directory receipt compatibility rechecks; exact-head hosted checks remain required before merge.

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
[descriptor IO review](backend-descriptor-io-independent-review.md) and
[Windows fixture review](backend-windows-authority-independent-review.md) and
[created-directory review](backend-created-directory-path-independent-review.md) retain ranked
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


## Windows internal authority fixture correction

Windows on `f663bd63` reached seventeen move-staging unit cases and refused their
initial ownership-proof setup. Both public Apply and Undo normalize approved
roots before entering the private directory-capability scope. The direct unit
fixtures supplied raw temporary roots instead; Windows canonicalization uses a
different extended-length spelling. The correction normalizes each newly created,
disposable fixture root before constructing any paths or entering that scope.
The descriptor unit fixture follows the same private precondition.

This is a fixture setup correction, with no production root, identity, reparse,
sharing, budget or recovery predicate changed. The original fault assertions
remain. Current-head Windows execution remains necessary to verify the previously
unreached bodies; local Linux results do not establish Windows behavior.


The [Windows fixture review](backend-windows-authority-independent-review.md)
records the two constructor hashes and the static check of all seventeen
previously unreached move-staging bodies. A Linux disposable symlink-TMPDIR probe
fails before normalization and passes afterward; its pre-fix failure is Unix
nofollow rejection, not a claim to emulate Windows prefix semantics. Existing
file-race attacks remain enabled because entry transport handles share delete;
ancestor directory guards remain held. No further scoped body defect was found
from source review. Exact-head Windows execution is still required.


The final combined gate was repeated on those frozen constructor changes:
1,638 passed, zero failed, 34 optional/manual ignored across 89 outer targets;
workspace all-target/all-feature Clippy, formatting, diff checks and two private
D-Bus tests passed. Production source is byte-identical to `bd2e6091`. Original
Windows-hosted failure evidence and current-head hosted results remain distinct;
Windows runtime confirmation is still required before merge.


## Created-directory containment oracle and complete CI diagnostics

Windows on code `503408da` and docs `a9987bb5` passed the previous internal fixture
failures: 270 core unit tests passed, with one existing ignored, and 345 command
unit tests passed, with seven existing ignored. The original policy/panic canary
passed. Both jobs then reached the same single Executor containment assertion:
47 passed and one failed. These are component results, not complete hosted gates.

The failing test compared canonical created-directory receipts with its raw
temporary-root spelling. Cleanup receipts do not promise caller spelling. The
assertion now resolves the existing root and each receipt directory before
checking containment; this also verifies those directories exist. Raw public
input paths are retained, as are the count of two created folders, file movement,
Undo removal and pre-existing-folder survival checks. No backend code changes.
The exact old assertion reproduced on Linux using an owned symlink-TMPDIR; all
48 Executor cases passed under that same alias after the oracle correction.

[Independent review](backend-created-directory-path-independent-review.md)
records the contract/consumer audit and verification. The fork's Linux, Windows
and macOS test commands now use `--no-fail-fast` to collect failures from every
workspace target in one run. Any failed target still produces a failing command
and blocks merge. This fork CI diagnostic policy changes no platform guard,
ignore, requirement or release policy. Shared backend upstream eligibility remains
waiting on compatible foundations; no upstream workflow contribution is implied.


The same review also covers a confirmed alias-path precondition in the Unix-only
cleanup-index fixture. Its original setup failed under a disposable symlink
TMPDIR before the private scope; normalizing the newly created root lets both
retained index/retirement cases run. Every retained-pin, alias-redirection and
index assertion remains. Only two setup lines inside `cfg(test, unix)` changed;
runtime authority logic is unchanged. This portable check does not claim actual
macOS execution. The final combined source hashes belong to the new review.


The final combined gate on the path-oracle, Unix fixture and CI diagnostic
freeze passed 1,638 workspace tests, zero failed, 34 optional/manual ignored
across 89 outer targets. Workspace all-target/all-feature Clippy, formatting,
diff checks and two private D-Bus tests passed. Independent review accepted 50
distinct cases, including public raw-path alias and private cleanup-index alias
runs. These counts overlap the workspace gate. Exact-head hosted checks remain
required before merge; neither code nor documentation was merged at this gate.
