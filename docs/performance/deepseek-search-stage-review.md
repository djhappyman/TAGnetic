# Query and shared-stage reliability/performance review

Date: 18 November 2026. Branch `feat/native-field-foundations`, baseline
`0554022c`/`b657f65d`. Scope: `crates/commands` query/read-only snapshot and
derived caches, and `crates/service` shared-stage projection/accounting. Desktop
UI, player, core, service plans/outputs, shared `tests.rs` and user audio were not
modified. No dependency, permission or protocol change was made.

The codebase graph is indexed at another checkout and generation (13 November)
and is stale for this branch: graph queries were used only for discovery, every
relied-on range was read from the working tree, and no exhaustive coverage is
claimed. Findings were reproduced or measured on synthetic fixtures under
`/mnt/Games/Development/tagrex/tmp/deepseek-search-stage` (disposable, removed
after the run); no real library, stage, journal or audio file was touched.

Measurement method: `cargo build --release -p tagrex-commands --example
query_bench`, piped a deterministic 20k-row and 100k-row fixture (repeated
collaboration credits plus unique artist names on every eleventh row, unique
album/title per row, 200-character lyrics, a
`custom:MOOD` tag) and compared medians/p95 plus result checksums before and
after each change. Staging costs were measured in release against a 43.7 MB
plan (4096 files x 10 fields x 1 KiB) with `VmHWM` deltas. Timings are single
machine, single process and are not a product-wide speedup claim.

## Confirmed and fixed

### 1. Suggestion keystrokes rebuilt large facets in a cycle (100k rows: 501 ms to 6.4 ms)

Trigger: any search/suggestion keystroke in a library whose distinct values are
large enough that two facets exceed the retained budget — for example 100k rows
where album and title are unique per row.

Cause: `QueryIndex::facets` was capped at 32 entries / **32 MiB**, while the two
hot facets measured 17.3 MiB (`album`, 100k values) and 17.5 MiB (`title`,
100k values). Inserting one evicted the other, so every suggestion request
rebuilt whichever field had just been evicted, and the eight-field suggestion
rebuilt several. Instrumentation showed the cache holding `title`, `year`,
`format` only after each request (`album`, `genre`, `artist`, `albumartist`
absent).

Fix: `crates/commands/src/query.rs` now defines `FACET_CACHE_BYTES` (96 MiB) and
`FACET_CACHE_ENTRIES` (32) and uses them for both the base snapshot and the
computed-field overlay, so a suggestion's working set stays resident. This is
still a bounded, evictable derived cache: nothing here is a value or write
authority, `Cache::insert` still refuses an entry larger than the whole budget,
and the 64 MiB per-facet working/cancellation budget is unchanged.

Measured, 100k rows, 15 iterations each, identical suggestions:

| state | cold | median | p95 |
|---|---|---|---|
| before | 301.4 ms | 500.9 ms | 504.7 ms |
| after | 225.9 ms | **6.4 ms** | **6.7 ms** |

20k rows is unaffected (1.2 ms to 1.1 ms warm): there the whole working set
already fit in 32 MiB.

Edge cases: a single facet larger than the budget is still built and returned
uncached; the cold build remains O(rows × log(distinct values)) and is reported as a
known limit below; the overlay index gets the same budget so computed columns
cannot inherit the old thrashing cap.

### 2. A colon-heavy search text rescanned every row per candidate prefix (100k rows: 40.7 ms to 8.3 ms)

Trigger: typing or repeating a search text whose colon-separated candidates are
not standard field names — a time such as `01:23:45:67:89`, a timestamp pasted
from a comment, or an absent custom field.

Cause: `QueryIndex::text_scope` resolved each candidate prefix (longest first)
with a fresh full-library scan of every row's value map before falling back to
the built-in field list. Five candidates over 100k rows is five complete scans
per request, on every keystroke.

Fix: a bounded per-snapshot memo (`field_keys`, at most 256 names of at most 128
bytes) remembers whether any row stores an exact key, positive and negative, so
each candidate name is scanned once per snapshot instead of once per request;
names past the bound still resolve by scan. Resolution order keeps stored-key
precedence over a case-folded alias, so a computed field literally named `DATE`
keeps resolving to itself rather than to `year` (regression test added). The
standard-name check now runs before the row evidence for exactly spelled names
and skips the case-folding allocation when a name is already remembered.

Measured medians (p95 in brackets):

| rows | before | after |
|---|---|---|
| 20k | 6.085 ms (6.84) | **0.690 ms** (1.27) |
| 100k | 40.745 ms (44.09) | **8.329 ms** (9.30) |

The remainder is the genuine full-text scan of the same text (7-8 ms at 100k
rows), which the baseline also paid. Every other measured query/suggestion
result was byte-identical by checksum (`matched`, index checksum, suggestion
list); the full-text query's p95 at 100k rows moved from 12.97 ms to 7.87 ms as
a side effect of lower allocation pressure.

### 3. An initial stage delta cloned and serialized the whole plan before refusing to inline it (43.7 MB stage: 46.5 ms and +45 MiB inside the state lock, to 2.4 ms and no copy)

Trigger: the first `shared_stage_delta` (initial load, reconnection) or any
resync of a stage with up to 4096 files whose payloads are large — lyric or
comment edits, for example.

Cause: `resync_delta` built the owned `SharedStageSnapshot` (a full clone of the
staged plan) and then serialized it into a `Vec` to decide whether it fit the
8 MiB inline limit. A stage that cannot inline paid two complete copies of the
plan, transiently, while holding the shared state mutex that every stage
operation needs.

Fix: the decision is now made on a borrowed view (`SnapshotView`) through a
byte-counting writer (`ByteBudget`) that stops as soon as the inline limit is
crossed; the owned snapshot is only built when it will actually be returned, and
its derived-field list is computed once instead of twice. The measured bytes are
provably the same bytes the answer carries (test asserts view and owned
serialization are equal). Session, revision-gap, structural and overflow resync
decisions, and the >4096-file fast path, are unchanged.

Measured: 46.5 ms and +45 MiB `VmHWM` before, **2.4 ms and no measured increase in process
high-water memory** after; the inline/required answer is unchanged.

### 4. Stage budget accounting serialized the whole staged plan to measure it (43.7 MB plan: 23.8 ms and +42 MiB to 12.7 ms and no copy)

Trigger: every full-plan publication and every agent stage (`publish_human`,
`stage_agent_locked`), and the re-measure path after an estimated overflow —
`stage_budget` runs while the state mutex is held.

Cause: `stage_budget` used `serde_json::to_vec(plan).len()`, materializing a
second complete copy of the plan just to count its bytes.

Fix: the same streaming `json_len_within` helper counts the serialized length
without retaining it, with the same 128 MiB refusal point and the same
`quota`/message behaviour (byte-for-byte equal to the previous wire length;
tested). The per-publication `added` estimate in `publish_human_patch` used the
same `to_vec(&plan.changes)` shape and now streams with an early abort, so an
oversized patch is refused before it is rendered.

Measured: 23.8 ms and +42 MiB before, **12.7 ms and no measured increase in process high-water memory**
after; the `added` estimate for a scope-sized patch no longer allocates its own
copy.

### 5. Claim digests copied each staged value twice (40,960 fields: 56.6 ms to 46.3 ms)

Trigger: any publication that re-digests a plan's tag claims — full-plan
publish, agent stage, apply completion — with long values.

Cause: `atoms` built a `serde_json::Value` per field (cloning the value into a
map) and then a temporary `Vec` from `to_vec` for `hash`.

Fix: a borrowed `TagAtom` is serialized straight into the SHA-256 state. The
digest is byte-identical to the previous construction (the struct's field order
matches the sorted keys `json!` emitted, pinned by a test that also covers
non-ASCII, quotes, newlines, `None` and `invalid` fields), so no stored claim
fingerprint changes meaning.

## Reviewed and rejected as findings

- **Direct posting lookup for `equals`/`member` facets.** Measured at 0.10 ms
  for a 100k-distinct-value equals filter; the cost is the posting-list
  sort/dedup, not the value scan. Rejected as unnecessary churn.
- **Deduplicating the suggestion context JSON keys.** Serializing the query
  model per field costs well under a millisecond and is already amortized by the
  membership `Arc`. Kept.
- **`known_field` row scans.** Only reached for names that are neither built-in
  nor `custom:`-prefixed (for example `media`); at most ~58 string comparisons
  otherwise. Now shares the bounded memo anyway.
- **`with_updates_if_changed` content comparison.** Full row equality runs only
  for the refreshed dirty paths, not the library, and the no-op snapshot reuse
  from the earlier checkpoint is intact (`query_snapshot_reuse` still passes).
  Reordering the cheap fingerprint ahead of the map comparison would not remove
  the required comparison.
- **`stage_get` per-page scan.** Counting visible files and locating an offset
  in a path-ordered stage is inherently O(changes) per page; the earlier
  checkpoint already moved canonicalization and grant I/O out of the lock and
  reuses one path decision per request. Unchanged.
- **Member parsing allocations.** `members`/`known_artists` work is cached per
  distinct credit and per facet build; measured cold member facets are 23-28 ms
  at 100k rows only on the first build.

## Deferred findings (reported, not changed here)

- **Cold facet construction.** 73-76 ms per 100k-distinct-value facet, dominated
  by two B-tree lookups per row plus per-value `Vec`/`String` overhead. A
  compact sorted-arena representation (shared posting arena, inline singleton
  postings) would cut both rebuild time and retained bytes roughly in half and
  raise the thrash threshold; it is a representation change with its own review
  and test burden, so it is not folded into this checkpoint.
- **Derived cache memory per live snapshot.** The facet budget is per
  `QueryIndex`; the desktop keeps the current and pending snapshot and a
  computed-field overlay, so worst-case retained derived data is a multiple of
  the cap. No process-wide derived-memory budget exists and none is claimed.
- **Row storage.** Each row holds raw and case-folded values plus a concatenated
  search string (about three copies of the tag text). This is the dominant
  retained memory for lyric-heavy libraries; changing it alters matching
  semantics and needs its own measurement.
- **Cancellation granularity inside one field-key scan.** The memo makes it
  amortized; a single scan is still not interruptible, like one facet stride.

## Verification

On this worktree, with the shared toolchain and build cache:

- `cargo test -p tagrex-commands -p tagrex-service`: 352 + 4 + 1 + 148 passed,
  0 failed, 7 pre-existing ignored.
- `cargo clippy -p tagrex-commands -p tagrex-service --all-targets
  --all-features -- -D warnings`: passed.
- `rustfmt --check` on the three changed files and `git diff --check`: passed.
- New regression tests (12): scoped-resolution precedence and memo bounds,
  computed `DATE` identity, hot-facet retention across suggestions, default
  facet budget floor and both construction sites, borrowed-view byte equality,
  inline resync, over-budget resync without building, session resync, budget
  wire-length equality and early abort, and the claim-digest equivalence. The
  facet-budget floor, hot-facet retention and scope-precedence tests were each
  verified failing against the pre-fix behaviour before being restored.

This is a scoped local result: the parent reruns the combined workspace suite,
and no hosted CI, platform matrix, release, RSS bound or whole-app speedup is
claimed.

## Routing

Reusable query/cache and stage-projection fixes: candidates for a curated
upstream `main` PR after the parent's review. No fork-only storage, identity or
protocol behaviour changed, so no service/storage compatibility review is
required beyond the normal one.


## Parent review follow-up

The 96 MiB setting is a deliberate per-index retention trade, not a universal
speedup or whole-process cap. No memory is preallocated to that cap. The 100k-row
measurements are retained; larger-library performance is not verified here.
VmHWM deltas do not prove zero allocations: the accepted improvement is removal
of complete cloned plans/serialized buffers, not all allocation in resync.

Parent caught the derived-reference Vec being collected before the >4096-file
fast path and before the byte-budget decision. The inline view now streams those
references borrowed from claims too; only an accepted inline snapshot owns the
reference strings. Byte-equivalence tests cover both derived and ordinary claims
and artwork references. The larger-plan fast path does not collect the list.
