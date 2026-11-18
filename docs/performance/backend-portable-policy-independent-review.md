# Independent portable metadata policy boundary review

Status: confirmed boundary gaps corrected; eighteen retained independent probes
pass. No remaining defect found in this bounded review. Hosted verification of
the exact pushed head remains a separate merge gate.

## Scope

This review follows the Windows failure of the original public App policy-change
test at publication checkpoint `96480a82127fda33bd2a6ae7edae5a0d7f241b92`.
The original test remains unconditional. The Linux native FLAC implementation
refused policy changes, while the portable legacy Executor still proceeded.
This review targets that shared boundary, without claiming native FLAC support
on Windows or macOS.

The reviewer owns only `crates/core/tests/portable_policy_independent.rs` and this
report. Production corrections belong to the implementation author. The graph
project `tagrex-backend-readiness`, generation `2026-11-18T12:48:55Z`, indexes a
different checkout; coverage checks on the publication policy/model/plan/journal
paths returned `outside_project`. Conclusions use the actual publication source
and public API executions.

Initial reviewed hashes:

| File | SHA-256 |
| --- | --- |
| `crates/core/src/plan.rs` | `930c9843ee0148e70f56122549cc228333785c616ac62c57e937149b2c36a2f1` |
| `crates/core/src/model.rs` | `1e864a3d1940a15e8a7e0f683bcad58927759f27f617e747e95c105857f559ab` |
| `crates/core/src/model/policy.rs` | `75e1ab3ca940bf85612f95b65577518506b104e4f3ffeb360d1cb5080ded3401` |

## Ranked result before correction

**P2: the common legacy Apply/Undo path does not enforce its operation's metadata
policy generation around public or journal callbacks.** Portable public
Executor probes using real disposable FLAC files and VecJournal deliberately
select the legacy path on Linux. They reproduce the cross-platform behavior gap:

- An already-invalid owner, an owned configuration change, or a standalone
  setter in the first progress callback still produces successful Apply and
  media/history changes.
- Owned configuration change after a completed prefix or the final completion
  callback is silently accepted as successful Apply.
- A policy change inside foreign journal `record` is silently accepted. The
  retained combined test also checks `finish` after the correction.
- Callback panic after a completed Apply prefix leaves partially changed media.
- Undo accepts changes from its first progress callback or journal-load callback,
  restores media and removes history. A change at the next begin callback, after
  raw inspection proves an earlier file was restored, does not stop the remaining
  restores or retain history.
- Automatic legacy rollback does not establish a sealed policy boundary before
  its journal callback. Reentrant public Apply reaches progress, contrary to the
  native recovery boundary already established by the preceding review.
- Direct public Executor without an explicit ModelPolicy scope loses its
  admitted multi-value interpretation: change standalone separator after the
  first artist edit, cancel, and the owned artist remains changed with
  `RollbackIncomplete`, despite no external media mutation.

These are manifestations of the shared boundary gap, not independently
ranked bugs. The first provisional Undo probe assumed completed events, but
legacy Undo emits begin events only. It was corrected to observe actual restored
media at the next begin callback; only that corrected probe is defect evidence.

Red evidence under `/mnt/Games/Development/tagrex/tmp`:

- `flac-portable-policy-independent-red-v2.log`: 3 pass, 8 fail, including the
  corrected incremental Undo probe.
- `flac-portable-policy-invalid-entry-red.log`.
- `flac-portable-policy-undo-load-red.log`.
- `flac-portable-policy-reentry-red.log`.
- `flac-portable-policy-standalone-prefix-red.log` (the common plan path was still
  unchanged while the author had started its new policy helper).

The first correction passed all seventeen original portable probes and scoped
Clippy. Changed-seam review then found the same **P2 completion boundary gap in
native dispatch**: common Apply returned directly into native FLAC, whose final
completed callback was not checked before finishing. An eighteenth,
platform-neutral SQLite variant of the final-completion test returned successful
`AppliedBatch { native_tags: true }` on Linux after invalidation. The independent
failure is retained in `flac-portable-policy-sqlite-final-red.log`. Its original
assertion now passes after correction of the native callback/finish boundary.

The correction pins an operation's inherited App policy or standalone defaults
at the shared Apply/Undo entry. Checks surround public progress and material
journal callbacks. Failed/panicking Apply restores its known owned prefix under
the admitted interpretation in a sealed private recovery scope; public Executor
reentry cannot use that scope to start new work. Original field, block, source
and root ownership checks remain. Native dispatch now checks completed callbacks,
checkpoint acknowledgements, confirmations and finish before success, using its
existing recovery mechanism.

## Retained probes and boundary

The new test file has no platform `cfg` gate. All writes use disposable fixture
directories, and an independent raw FLAC reader compares comment maps and
encoded audio tails. Ordered repeated artist values and a Unicode custom comment
must survive rollback. Preview/pre-effect refusal checks use whole-file bytes.
The original test suite is not serialized or weakened; the global-setter probe
isolates its process-wide mutation in a child test process.

The eighteen probes cover invalid entry, first callback, owned change-and-restore
(ABA), standalone setters, completed prefixes with/without cancellation, final
completion, panic unwind, fresh retry, Undo before effects, incremental Undo,
peer-owner isolation, journal record/finish/load callbacks and recovery reentry.
SQLite final-completion coverage exercises native dispatch on Linux and the
portable journal path on other systems.
They also retain these passing baseline counterprobes:

- Cancellation restores owned metadata/audio despite invalidation.
- An external replacement of an already-owned title is preserved and unresolved
  recovery history remains available.
- An external change to an unrelated custom comment survives rollback while the
  owned title is restored.
- Peer-owner settings do not invalidate the current operation.
- Successful terminal Undo history removal is its commit boundary. A foreign
  adapter that invalidates policy after successfully deleting that history does
  not turn the completed Undo into a misleading failure claiming retained
  recovery history.

Legacy Undo remains incremental and resumable. If policy changes after an earlier
restore, it must stop before the next effect, return an error and preserve history
for retry. This change does not add atomic roll-forward across file moves/copy
removal. The policy must be checked before terminal history removal; arbitrary
adapter effects after successful terminal deletion cannot be transactionally
recreated with the same batch ID through the existing trait.

## Verification and limits

Commands use `source /mnt/Games/Development/tagrex/dev-env.sh` and the existing
shared build cache/toolchain. Corrected-source evidence:

| Check | Result |
| --- | --- |
| `cargo test -p tagrex-core --test portable_policy_independent` | 18 passed, 0 failed/ignored, default concurrency; 0.05 seconds |
| `cargo clippy -p tagrex-core --test portable_policy_independent -- -D warnings` | Passed |
| `rustfmt --check --edition 2021 crates/core/tests/portable_policy_independent.rs` | Passed |
| Original unconditional App policy/panic canary (`cargo test -p tagrex-commands --lib owned_model_policy_refuses_same_thread_core_policy_change_and_restores_after_panic`) | 1 passed; original test source unchanged |
| `cargo test -p tagrex-core --test model_policy_independent` | 4 passed |
| `cargo test -p tagrex-commands --test model_policy_app_independent` | 13 passed |

These are 36 distinct passing tests; child-process invocations are not counted
again. The wider SQLite interruption, full workspace and hosted gates remain the
orchestrator's integration responsibility.

The final test/Clippy logs are
`flac-portable-policy-independent-corrected-final.log` and
`flac-portable-policy-independent-corrected-final-clippy.log` under the same task
log directory. The original failing assertions were retained. No test threads,
admission limits or platform gates were weakened.

Retained App/core logs are `flac-portable-policy-original-app-final.log`,
`flac-portable-policy-retained-core-final.log`, and
`flac-portable-policy-retained-app-final.log`.

Final reviewed production SHA-256 values:

| File | SHA-256 |
| --- | --- |
| `crates/core/src/plan.rs` | `daeb4b0505a3b0123247dc4b67f6b8042d811d8de2c8a4e9be828424201061b8` |
| `crates/core/src/model.rs` | `4dc857e2b4a201d60df9f687888681e7da041b740c9ed1eb669c2e354500c7cc` |
| `crates/core/src/model/policy.rs` | `3ffcc2a3db4036a6469fb8a2ac2e68dfee50b80061960da8e35466d4c9643849` |
| `crates/core/src/plan/native_flac.rs` | `cba3c092206ce2da824f09fd942496047e2dcf536ecc4f857a536800ae9d73fe` |

No live media, application runtime, portable build or existing user Undo history
is touched. Fixture destructors remove their own directories; a final directory
inventory found no remaining `tagrex-portable-policy-*` fixture directories.
Compact source and failure evidence remain for review.

Linux execution of platform-neutral legacy tests does not prove Windows/macOS
runtime behavior. Hosted checks on the exact final pushed head remain required.
Synthetic FLAC fixtures support metadata and encoded-byte assertions, not audible
decoder quality or power-loss/RSS guarantees. Broader roadmap implementation stays
paused after the authorized FLAC slice. Eligible backend changes follow the
upstream curation route in `docs/WORKFLOW.md`, preserving upstream inline journal
images and its stable waveform API.
