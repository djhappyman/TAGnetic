# Backend parity and hardening programme

This programme covers the remaining Rust capabilities in the README roadmap,
including the new foundations at `2a9341e9` and `75fc07fc`. It preserves
Tagnetic's current workflow and existing data. Backend-only delivery leaves
product feature checkboxes unchecked until the UI is implemented and verified.

## Scope and dependency order

### Checkpoint and pause — 18 November 2026

The authorized finish-and-pause boundary is protected Linux FLAC
[PR #54](https://github.com/djhappyman/TAGnetic/pull/54), reviewed head
`5af5d3885244bb9f894bc6175a34858005cdc5a6`. Required hosted checks and merge
records are linked there. Service authority/lifecycle
[PR #50](https://github.com/djhappyman/TAGnetic/pull/50) merged at `67625add`
after all four hosted checks passed.

**Remaining implementation is paused.** Read [resume tracker #51](https://github.com/djhappyman/TAGnetic/issues/51)
and the [handoff](backend-pause-2026-11-18.md) before starting again. The clean
FLAC publication passed 1,598 workspace tests, zero failures, 34 existing optional
ignored; workspace Clippy/formatting, two private D-Bus tests and UI check/lint/
build plus 216 unit tests passed. Independent coordinator, journal, policy and
admission reviews completed after corrections. These overlapping scopes do not
certify the wider parked programme, every platform or a release.

| Wave | Capability | Checkpoint and remaining work |
| --- | --- | --- |
| 1A | Shared resource admission, cache identity and policy invalidation | Process Budget and required native admission/per-App policy included in #54. Broader resource/cache/query/response ownership WIP is parked; combined RSS and actual large-library proof remain open. |
| 1B | Track-bound seek and job/preview contracts | Playback/control ownership reviews preserved locally. Physical-device/native lifecycle verification and focused publication remain open. |
| 1C | Durable Apply/Undo recovery | Reviewed shared file authority and copy/move/staging recovery prerequisites included in #54. Native filesystems, power-loss and low-latency busy SQLite cancellation remain open. |
| 2 | Exact repeated metadata and native identity | Narrow whole-plan in-place plain-text FLAC Apply/persistent Undo on supporting Linux profiles included in #54. Coordinator 135, journal 61, policy 17 and admission 54 scoped review gates passed (not additive). Exact Stage/DTO, mixed/typed/other-format and portable preservation remain open. |
| 3 | Artwork processing, properties/order and patterned export | Pixel/EXIF implementation/reviews parked. Native picture properties/order, shared stage/export, command and UI integration remain open. |
| 4 | CSV/TSV import mapping | Scalar parser/shared-stage implementation and diagnostics parked. Exact-value destinations, publication and UI remain open. |
| 5 | Grouped playlists/reports/encodings/append | Renderer/output ownership implementations and reviews parked. Command/stage/grouped publication and atomic append remain open. |
| 6 | Value/pattern helpers, mappings, filenames and composed recipes | Further implementation and end-to-end integration remain planned. |
| 7 | Portable settings/recipes, safe sources and compatibility | Further implementation and producer/schema evidence remain planned. |
| 8 | Format policies, timestamps, containers and chapters | Further implementation and per-container preservation/platform evidence remain planned. |
| 9 | Integrated stress and delivery | Parked 48-case runner contract has 16 passing harness tests. NEW independent runner review and a complete frozen-programme run remain open. No full-backend readiness claim. |

On an explicitly requested resume, start from then-current merged main, reconcile
already integrated #50/#54 code and curate only remaining WIP from the recovery
archive. Do not apply the old whole-programme patch over main. Independently
review [the runner contract](../performance/backend-stress-coverage-readiness.md),
then run combined unchanged-source workloads. Keep original fault canaries and
existing quotas; publish focused slices with exact-head hosted gates before
continuing UI exposure. [Publication evidence](../performance/native-flac-publication.md)
records the Linux-only route and portable canary curation limits.

The [ranked findings](../performance/backend-roadmap-findings.md) retain
historical reproductions and correction gates. The Title-only preservation
failure is corrected in the admitted Linux route; broader legacy-path
preservation is still open.

Tests must include cold/warm and over-capacity cases, concurrent operations,
external changes, invalid input, cancellation, process death and recovery where
applicable. Timings include workload/environment and distinguish native compute,
queue/lock wait, filesystem work and IPC. Resource accounting is not an RSS
guarantee. No user media, runtime dependencies or existing Undo data are test
fixtures. Preserve compact results and recovery evidence; remove task-owned
disposable fixture trees only after verification.

- [ ] **Shared hardening:** audit new operations; bound aggregate memory and work;
  retain useful metadata cache entries under pressure; validate settings and
  inputs; check cancellation, invalidation and recovery boundaries.
- [ ] **Lossless repeated values:** implement format-aware lists throughout
  native reads/writes, staged edits, persisted state, Apply, Undo and recovery.
  Keep literal punctuation distinct from separators. Migrate older scalar
  journal data without changing its meaning.
- [ ] **Artwork:** process existing embedded images, change properties, preserve
  every supported picture category and order, and support patterned export.
  Bound decoded-image memory as well as compressed input sizes.
- [ ] **Spreadsheet imports:** parse quoted CSV and tabular data, support explicit
  mappings and match keys, and report malformed or ambiguous records.
- [ ] **Playlists:** load supported playlist files in order and generate custom
  grouped playlists through guarded output publication.
- [ ] **Reports:** reusable structured templates, grouping, totals, encoding/BOM
  choices and atomic append behavior with collision and external-change checks.
- [ ] **Patterns and mappings:** individual-value access, field enumeration,
  additional text/date/path helpers, consistent field aliases and direct filename
  rearrangement while retaining existing naming and sidecar protections.
- [ ] **Composed recipes:** coordinate tag, encoding repair, artwork, path and
  export steps with explicit stage/save boundaries, cancellation and failure
  behavior. Do not bypass the existing executor or output publisher.
- [ ] **Portability and extensibility:** versioned settings/recipe import-export,
  validated source definitions and explicit Mp3tag compatibility diagnostics.
  Never execute imported configuration as arbitrary code.
- [ ] **Formats and preservation:** finer tag policies, timestamp preservation,
  additional containers and existing chapter metadata. Verify actual writer
  support before advertising a format; preserve unrelated container data.

The active [ranked findings ledger](../performance/backend-roadmap-findings.md)
records reproduced consequences and correction gates, including open findings.

## Completion gates

Each capability gets an independently reviewable, committed checkpoint after
its tests and review pass. A checkpoint records the exact implemented scope,
remaining limitations, commands/results and upstream route. This document is
tracking active work; unchecked items are not complete.

Every relevant checkpoint must demonstrate:

1. Correct behavior against official feature/format documentation, adapted to
   our domain and shared stage. Unknown or unsupported inputs fail explicitly.
2. Backward compatibility with existing persisted settings and Undo history.
   A feature requiring migration includes upgrade and old-data fixtures.
3. Bounded input, intermediate allocations, retained caches and output. Caches
   have defined invalidation and eviction behavior; write staleness checks use
   authoritative data rather than trusting cached previews.
4. Cancellation without publishing a partial preview; tested rollback/recovery
   for partial writes and interrupted operations where applicable.
5. Disposable media checks for preview, Apply and Undo, with independent
   metadata readback and encoded audio hashes for lossless metadata operations.
6. Meaningful failure and scale tests, format-specific constraints, formatting
   and warning-free Clippy. Run platform checks before claiming platform support.
7. Backend and UI progress remain separately tracked in the README. No UI source
   changes or active-runtime/real-library mutations belong in this programme.

## Current audit

- **Metadata cache eviction, verified:** the existing cache invalidates on
  writes/settings/session changes and fingerprints reads, but clears all entries
  when full. Replace wholesale eviction with bounded least-recently-used eviction
  and test retained hot entries, byte pressure, failed reads and invalidation.
  The local checkpoint is documented in [cache verification](../performance/metadata-cache-hardening.md).
- **Requirements matrix, reviewed:** [official-source requirements](backend-parity-requirements.md) now cover all remaining families, test gates and dependency limits. Exact resolved versions were verified against Cargo.lock. The matrix is requirements evidence, not implemented parity.

- **Native playlist preview and text codec, verified:** local M3U/M3U8 resolution and shared strict conversion now have tests and [checkpoint evidence](../performance/playlist-loading.md). Playlist UI, expression grouping and export publication integration remain pending.

- **App-wide hardening checkpoints, verified locally:** unchanged query reuse,
  staged-view lock/copy reduction, streaming plan accounting, bounded exports and
  derived query memory are implemented. See the [audit](../performance/rust-hardening-audit.md)
  and linked evidence. Preview queue supersession, artwork resource admission, playback coalescing,
  shared metadata and indexed integrity lookups are also implemented. Production
  profiling and platform checks remain separate. Feature/UI completion is unchanged.

## Ownership and upstream route

Continue in the existing dependent `feat/native-field-foundations` checkout and
reuse its toolchain, dependencies and build cache. Preserve all other checkouts,
pending human edits, running profiles, portable builds and existing Undo history.
The parent owns Rust implementation, integration, tests and this tracking file.
Any research task owns only its explicitly assigned requirements document.

Reusable backend changes are candidates for curated upstream `tagrex/tagrex:main`
contributions. Shared interface work would follow upstream `next` and is outside
this Rust-only scope. Fork-specific service/storage differences require separate
compatibility review; preserve upstream's inline journal images and waveform API.
Publication, hosted verification and acceptance are recorded separately from a
local checkpoint, following `docs/WORKFLOW.md`.
