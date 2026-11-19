# Backend consolidation — 19 November 2026

Owner scope: commit and merge achieved backend work, attach development to main,
and retire non-main working branches while preserving recovery material.

## Candidate

Reconcile the reviewed resource/cache/query, preview cancellation, player,
artwork/EXIF, scalar tabular-import, bounded pattern/report and output-publication
implementations against merged main `12369be1`. Retain the newer #50/#54 policy,
held-file, descriptor IO, crash recovery and cross-platform corrections.

- Resource admission covers additional cache/query/artwork/output owners, with
  typed busy/limit outcomes; it is not a whole-process RSS guarantee.
- Native query snapshots share admitted rows. Worker validity must follow the
  originating App's policy rather than the worker thread's standalone defaults.
- Player control owns source-specific seeks and a bounded output router; no
  physical-device latency or native device-recovery guarantee is claimed.
- CSV/TSV scalar imports compose through the shared stage, locks and Apply/Undo.
- Artwork pixel/EXIF processing, bounded rendering/report helpers and confined
  output publication retain their documented supported-input boundaries.
- Existing frontend source and wire shapes remain; new controls and general
  repeated-value/mixed-format integrations are separate work.

## Exclusions and recovery

The full source snapshot is retained at
`refs/checkpoints/backend-consolidation-2026-11-19`, with its manifest under the
workspace `checkpoints/backend-consolidation-2026-11-19/` directory. This includes
unfinished work without requiring a working branch.

The incomplete stress runner and its Windows birth-owned process-supervision
blocker are excluded from this production integration. Its 40 source methods
are not executed passes. The original unconditional portable metadata-preservation
canary is preserved unchanged in that snapshot: #54's protected Linux route does
not establish equivalent preservation on the legacy Windows/macOS writer.
Generalized exact-value migration and additional-container designs remain plans.

## Completion gate

Local builds/tests/benchmarks/stress remain prohibited by the owner. This
candidate is assembled using source review, conflict reconciliation and Git
whitespace checks. Linux, Windows, macOS and advisory hosted checks on the exact
pushed head must pass before merge. Hosted component probes must exercise the
additional real decoder, tabular media/pressure and cache-pressure cases.
Record actual results and merge SHA here before reporting delivery.

## Upstream route

Shared backend work is intended for curated `tagrex/tagrex:main` after compatible
native-field/text-codec foundations exist. Preserve upstream inline journal
images and waveform APIs; do not copy the fork-only pooled journal wholesale.
Existing open upstream contribution heads need recoverable source when local
working branches are retired. No upstream acceptance is implied.

## Development attachment

Vite now runs as transient user unit `tagnetic-ui-main.service`, with working
directory `/mnt/Games/Development/tagrex/personal-app` and port 1420. HTTP startup
was verified. The native process was not restarted; session epoch, open-library
state and staging revision 35 (zero staged files) were unchanged across the switch.
The currently running native binary still predates this candidate: moving the
frontend source does not load newly compiled Rust into that existing process.

## Reconciliation details

Retain #54's per-App policy rather than restoring the older process-global
interpretation. Query snapshots, shared forks and computed views retain their
originating policy owner so worker-thread TLS cannot spuriously invalidate them.
The old invalidated-scope query canary now checks explicit error propagation
instead of accepting an unusable snapshot; old views remain invalid. Service
policy checks cover both peer isolation and owning-App settings invalidation.

The first hosted candidate `98f6951f` failed formatting. Its hosted formatter
patch was downloaded and applied without running a local formatter; further
combined checks remain pending on the next head.

## Hosted compile follow-up

Head `736ddb61` reached Windows Clippy and exposed an unnecessary tail `return`
in the Windows fingerprint branch. macOS CI and both native FAT32/exFAT output
jobs exposed unconstrained type inference for the optional original-proof vector.
Remove the redundant return and explicitly type `Vec<Option<Proof>>`; retain the
whole-batch duplicate-identity checks and every runtime assertion. These are
source corrections awaiting the next hosted execution, not test passes. Logs
are retained in the consolidation checkpoint's `evidence/736ddb61/` directory.

Head `62044a1e` passed the previous compile blockers and reached two more
platform-specific lint gates: macOS simplified the Unix-only compiled identity
comparison; Windows reported a test fixture's lifetime-owned temporary root as
unread because its path is used only by Unix probes. Express each platform's
identity predicate directly and name the retained fixture owner `_root`. Keep
the Windows full native identity witness, Unix equality and all test cases.
Logs are retained in `evidence/62044a1e/`; runtime validation remains hosted.

Head `423297d0` reached actual macOS FAT32/exFAT output execution. Both volumes
failed normal new-file publication at candidate ownership validation before the
foreign-replacement interleaving. Retain both original assertions and add tagged
test-only diagnostics to distinguish a cached identity change, content/stamp
mismatch and pathname resolution. The existing publication regression on real
mounted disposable volumes is the red feedback loop. No local reproduction or
filesystem-support claim follows. Diagnostics must be removed after the actual
cause is confirmed and the corrected hosted regression passes.

## FAT-family ownership correction

Hosted `0080c152` diagnostics confirm the first macOS hypothesis: cached identity
was unequal while current held/named device/inode, size and timestamps were
identical on both FAT32 and exFAT. Compare current kernel-handle identities on
Unix. Keep content hashing, strict mutation stamps and foreign-object refusal.
The tagged diagnostic code has been removed; its logs remain archived.

Windows exFAT separately returned error 87 during native identity validation.
Use the extended identity query where supported. Only unsupported-class errors
may try a legacy volume-serial/file-index query, and only after the held handle
identifies its own filesystem as FAT/FAT32/exFAT. Do not downgrade ReFS or a
permission/IO failure. Keep the basic change/attribute witness mandatory. See
[Microsoft's handle query](https://learn.microsoft.com/en-us/windows/win32/api/fileapi/nf-fileapi-getfileinformationbyhandle)
and [identity semantics and ReFS limits](https://learn.microsoft.com/en-us/windows/win32/api/fileapi/ns-fileapi-by_handle_file_information).
These source changes still require actual Windows execution.

A retained real-file regression now holds an empty candidate through growth,
checks the reopened handle, then renames it and creates an identical foreign
replacement: the owned object must match and the replacement must refuse.
The existing full publication and rollback assertions remain unchanged. The
Windows disposable-volume label is shortened and explicitly checked after the
runner reported an illegal label. The output workflow also watches changes in
the split ownership module. No local workload ran; all current runtime gates
remain pending on the corrected head.
