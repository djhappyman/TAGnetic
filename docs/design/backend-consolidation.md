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
