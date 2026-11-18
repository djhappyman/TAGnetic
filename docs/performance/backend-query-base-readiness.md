# Base query snapshot resource ownership

Base snapshot scope reviewed locally; combined current-source/platform gates pending.

## Scope

Native session listing and row refresh now reserve shared retained capacity
before constructing query field text, case-folded text and combined search
buffers. Rows hold shared leases, so an outstanding worker retains its charge
after its parent snapshot is replaced or dropped. Technical values use a small
row overlay instead of cloning a shared saved-value map.

`App::list_tracks` and `App::query_snapshot` are fallible; desktop and service
entry points propagate resource-busy/resource-limit errors. A failed listing
publishes no partial index and keeps the old current snapshot usable. Source
paths are consumed lazily rather than collecting the complete scan first.

The existing low-level `QueryRow::from_track` and `QueryIndex::from_tracks`
constructors remain explicitly unmetered for caller-managed datasets. Production
session paths use `try_from_track`; new consumers can use `try_from_tracks`.
This is participating owner accounting, not an allocator or an RSS limit.
Response DTOs, computed overlays and other nonparticipating owners still require
separate integration. The process's 256 MiB retained ceiling is unchanged, so a
large dataset may refuse admission instead of exhausting memory. No fixed
library-size guarantee follows from this checkpoint.

## Evidence

`query_base_memory_recheck` uses an observational System allocator in isolated
subprocesses. Caller-owned inputs are excluded. The original 64-track,
long-comment probe retained **26,017,274 bytes with zero shared charge**. With
admission, the same shaped dataset retained **26,021,370 bytes with 26,523,922
bytes charged**, including conservative structural overhead. Dropping the index
while retaining a row preserves its charge; dropping the final owner releases it.

The second actual-session probe fills available retained capacity, adds another
file, verifies refresh and row reads refuse without replacing/invalidation of
the old snapshot, releases pressure and verifies the next complete refresh
succeeds. These probes and a local-budget oversized/busy/Unicode case pass (three tests). The existing snapshot-reuse integration probe and 27
query unit tests pass. All 150 service-library tests pass. Commands/service all-target, all-feature Clippy passes at
the local source checkpoint.

A wider commands-library run exposed a live-sidecar-link workflow regression
in the new recovery authority. The corrected wider run passes **381 tests**,
with 10 ignored. Independent base review then exposed preallocation overhead
for many empty tag-block labels: a 100,000-block caller DTO allocated 2.4 MB
under a 64 KiB budget before refusal. The constructor now bounds blocks to
256 before allocation and reserves per-block overhead. All seven independent
review probes pass, including the unchanged regression, 256/257 boundaries,
concurrency, native cancellation/policy changes and technical-mask compatibility.
See [independent review](backend-query-base-independent-review.md).

No whole-workspace success is inferred from these focused checks. Computed
column ownership is a separate active scope with its own retained-allocation
regression; base snapshot admission alone does not account for it.

## Delivery

No commit or PR yet. Reusable query/resource changes are candidates for curated
upstream `tagrex/tagrex:main`; service error routing must retain fork compatibility.
Desktop all-target/all-feature `cargo check -p tagrex` passes. Hosted
Windows/macOS/Linux current-head checks remain required.
