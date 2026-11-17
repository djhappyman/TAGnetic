# App-wide Rust performance and hardening audit

Date: 17 November 2026. Baseline: native branch `15859883`, including the bounded
metadata LRU checkpoint. This is a source-directed audit of the Rust backend and
desktop Rust dispatch. It is not an exhaustive security certification or a live
profile of the installed app. No user library or running profile was mutated.

The graph is indexed at another checkout/generation (13 November); coverage was
checked for relied-on paths. Stale/missing ranges were read from the actual
branch. The graph was used for discovery rather than completeness claims.

## Confirmed findings, in implementation order

1. **Unchanged reads invalidate query state.** `App::read_track_rows` reconstructs
   the index through `QueryIndex::with_updates`, which always invalidates the old
   snapshot and its duplicate results, then discards derived caches. A disposable
   one-file WAV probe reproduced replacement/invalidation with unchanged media
   bytes. Fix by preserving the existing snapshot on actual no-op reads while
   retaining filesystem identity/change evidence. Do not compare display tags
   alone: audio/private metadata may change without changing those tags.

2. **Export limits are checked after allocation.** Service export code calls
   whole-output CSV/HTML/XML/report/M3U/CUE builders and checks the 64 MiB limit
   afterwards. The intended quota cannot prevent a large renderer allocation.
   Add bounded/streaming output construction, count escaping/encoding expansion,
   and stop before crossing the budget. This is a source-confirmed allocation
   risk, not a measured out-of-memory event.

3. **Staging reads repeat filesystem scope work under a shared lock.**
   `stage_get` holds state while its per-field visibility predicate calls
   `destination`, which uses filesystem metadata/canonicalization. Paging scans
   the staged files and can repeat checks when filtering returned fields. Move
   I/O out of global critical sections, reuse one path decision per request, and
   preserve revision/grant consistency. User-selected roots include their
   subfolders recursively; ordinary views should reuse this context. These are
   connected-agent grant checks, not new user permission prompts. Authoritative
   write validation remains separate.

4. **Plan projections clone data they will discard.** `staging::filtered` first
   clones an entire selected file change and then removes excluded fields,
   images, blocks and rename details. Large lyrics or artwork/block payloads
   therefore impose avoidable transient work on narrow views. Filter ownership
   first, clone only retained parts, and retain shared artwork pooling/remapping.

5. **Plan accounting allocates serialized copies.** `plans::seal` uses
   `serde_json::to_vec(&plan).len()` for byte accounting, alongside hashing and
   projection work. Some of this is inside shared application/state ownership.
   Use capped counting/streaming serializers for size/hash operations and fail
   before building oversized intermediate buffers. Do not remove digest or
   authorization inputs to save time.

6. **Derived query caches have count limits without complete byte accounting.**
   Facets may contain many distinct long values; computed-field indexes clone
   all source rows, then add values. Facet/regex limits clear the entire cache
   when full. Add retained-byte budgets/admission and measured eviction; avoid
   materializing a whole second row set where a bounded derived overlay suffices.
   The memory growth path is confirmed; exact practical thresholds need profiles
   with high-cardinality and large-tag fixtures.

7. **Old preview work can queue after it is obsolete.** Desktop `Queue` correctly
   admits work before spawning a blocking thread, but has no bounded pending
   queue or generic latest-request cancellation. Native field-operation dispatch
   calls the noncancellable preview wrapper. Add per-lane generation/cancellation
   checks before starting expensive obsolete work, with separate write admission.
   Quantify queue wait separately from compute time before choosing limits.

## Implemented checkpoints

- Finding 1: `1a183a37`, [snapshot reuse evidence](query-snapshot-reuse.md).
- Findings 3–5: `1f7d3973`, [staged view/accounting evidence](staged-view-hardening.md).
- Finding 2: `853011f3`, [bounded export evidence](bounded-export-rendering.md).
- Finding 6: `f4653130`, [derived query memory evidence](derived-query-memory.md).
- Finding 7 remains in progress. Additional targets below are not completion claims.

## Additional profiling/hardening targets

- **Artwork working memory.** There is image-job admission and header/pixel
  validation in fetch paths, plus row-based shrinking for huge PNGs. Core cover
  processing can still fall back to a full decode followed by resize and RGB
  copies. Establish a shared decoded/scratch/output budget and cache only bounded
  encoded results by content/options. Measure actual peak memory before claiming
  a gain; do not cache unbounded decoded artwork.
- **Playback command backlog.** The player uses an unbounded command channel.
  Profile seek/volume bursts and decoder startup; coalesce superseded state
  commands while preserving Play/Stop ordering and responsiveness.
- **Integrity I/O.** Whole-block operations hash complete files before/after
  writes; matching block fingerprints to plan changes also does a linear search.
  Index those lookups and measure hashing overhead on long recordings. Do not
  replace authoritative integrity proof with cached mtime/size as an optimization.
- **Metadata ownership.** Cached reads return owned cloned tag maps, and row
  refresh derives a DTO and query row through multiple snapshot accesses.
  Profile allocation/stat counts, then share immutable snapshots and derive
  several views from one read where it removes demonstrated work.

## Existing protections to retain

- Batched calculated columns already share one metadata snapshot per file.
- Immutable query rows use Arc sharing; warm queries do not rescan audio files.
- BPM processes audio chunks and reuses its detector across a run. Duplicate
  audio comparison also streams decoded packets rather than retaining full PCM.
- Blocking resource queues acquire admission before spawning; image work has a
  separate bounded admission path and analysis has explicit concurrent-job caps.
- Apply/Undo bypass preview caches for authoritative staleness checks. Existing
  guarded output publication and journal recovery must survive optimization.

## Measurement and delivery gates

Measure queue wait, lock wait, compute, disk reads/stats, serialization, bytes
transferred, cache hits/misses/evictions and peak retained/transient allocation
separately. Use tiny/large tags, high-cardinality fields, many staged rows, large
artwork, rapid preview edits and slow filesystem fixtures. Compare cold and warm
states on the same workload; record p50/p95 and cancellation behavior.

This audit predicts no speedup percentage. The no-op snapshot issue was directly
reproduced; other confirmed findings are code paths with unmeasured practical
impact. Each implementation needs behavior-preservation tests, relevant scale
measurement, formatting/Clippy and existing Rust suites. Platform-specific claims
require corresponding filesystem/runtime tests. UI work remains excluded.

Reusable fixes are upstream `main` candidates after curation; service/storage
differences require compatibility review. No PR, hosted result or merge is
claimed by this audit. Progress belongs in the backend programme, with feature
boxes remaining unchecked until UI completion where relevant.
