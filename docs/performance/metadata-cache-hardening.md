# Metadata cache eviction checkpoint

Date: 17 November 2026. Base: `75fc07fc`, branch
`feat/native-field-foundations`. This is one checkpoint in the
[backend programme](../design/backend-parity-programme.md), not completion of
the remaining parity features.

## Behavior and scope

The native display/preview cache now evicts least-recently-used snapshots until
the next entry fits, rather than dropping every cached file. Its production
limits remain 65,536 entries and 64 MiB of estimated retained metadata. The
recency index has exactly one record per retained snapshot, so repeated hits do
not accumulate an unbounded access log. Index updates are logarithmic in entry
count. Oversized reads are returned to the caller without being retained and
without evicting unrelated cached entries.

Estimates include three owned paths, string/custom-name capacities, tag-block
storage and conservative index/entry overhead, using saturating arithmetic.
This bounds the cache's accounting; allocator overhead and process RSS are not
guaranteed by the estimate. It does not limit a parser's transient allocation.

File fingerprints are checked before every hit and before/after a new read.
Failures and visibly concurrent reads are not cached. Snapshots are owned clones:
staged mutations cannot alter saved cached data. Existing invalidation on
settings, refresh, writes and Undo remains in place. The executor's authoritative
write/Undo checks do not use the preview cache.

Unix fingerprints include device, inode and change time as well as size and
modification time. A dedicated regression fixture checks a same-size edit whose
modification time was restored. Windows still has the existing size/mtime
limitation; this checkpoint does not claim to solve identity collisions there.
Usage-clock rollover rebases the cache instead of wrapping ordering keys.

Owned implementation: `crates/commands/src/read_cache.rs`. The roadmap also
removes duplicate multiple-value progress entries. No UI source, running
application, user media or existing profile/history is changed.

## Checks

```sh
source /mnt/Games/Development/tagrex/dev-env.sh
cargo fmt --all --check
cargo test -p tagrex-core -p tagrex-commands -p tagrex-service
cargo clippy -p tagrex-core -p tagrex-commands -p tagrex-service --all-targets --all-features -- -D warnings
cargo test -p tagrex-commands --lib native_batch_benchmark -- --ignored --nocapture
```

The full Rust run passed 899 tests; 15 manual/fixture-dependent tests were ignored.
Clippy passed with warnings denied. The manual benchmark was run separately.

Five cache tests use disposable WAV fixtures: hot-entry retention; byte-pressure
eviction/oversized admission; external writes/failed reads/snapshot isolation;
same-size/restored-mtime invalidation on Unix; disabled cache, clear and clock
rollover. They assert accounting and recency-index invariants.

The existing 10,000-file filename-import/numbering probe passed on this cache.
Its warm median was 129.264 ms and first preview 461.564 ms on the existing Linux
i7-12700K development environment and tiny generated FLACs. This is a native
preview measurement, excluding UI, Apply and journal writes. Smaller cases warm
overlapping files. It is not a disk-throughput or cache-under-pressure timing
claim; the pressure tests establish retention behavior separately.

## Review and route

Review checked eviction ordering, exact accounting removal on stale/error paths,
oversized admission, bounded index growth and counter overflow. Clippy identified
test-module placement during development; the tests were moved after production
items. Remaining programme work includes parse-policy-aware identities, stronger
platform fingerprints, aggregate operation budgets, persistent caching where
measurements justify it, and the feature/storage migrations in the requirements
matrix. Those are not marked complete here.

This reusable backend change is eligible for a curated upstream `main`
contribution. The parent branch contains dependent fork/service work, so upstream
publication requires an isolated compatible base. No PR, hosted result, merge,
Windows/macOS runtime result or release is claimed by this local checkpoint.
