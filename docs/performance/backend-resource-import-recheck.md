# Fresh independent resource and import recheck

Date: 18 November 2026. Review base: `9da71249` plus the active uncommitted
backend programme in `native-field-foundations`. This reviewer did not author
the implementation or its first adversarial review. Production files were not
edited. Reviewer ownership is this report and
`commands/tests/resource_import_fresh_recheck.rs`.

Intent: preserve bounded participating resource ownership, useful cache contents,
coherent native previews and atomic table-import staging under pressure and
cancellation. Verdict on the reproduced candidate: **request changes** for two
additional P2 conditions. Passing first-round canaries do not waive these.

## P2 — failed global cache admission discards useful hot entries

`commands/src/bounded_cache.rs`, `Cache::insert`, evicts its local LRU entries
before obtaining the new shared retained reservation. A distinct newcomer can
still fail global admission after that eviction. The old hot value is then
lost without any replacement, worsening cache behaviour precisely under pressure.

The isolated public-budget/private-store source canary
`rejected_global_cache_admission_preserves_existing_hot_entry` reproduces this
without allocating hundreds of megabytes:

1. A derived cache with a local ten-byte budget retains a useful eight-byte
   entry.
2. Another owner reserves the remainder of the production retained allowance.
3. A distinct nine-byte newcomer evicts the eight-byte entry locally, but still
   cannot fit globally.
4. Both newcomer and old hot entry are absent. The expected retained old entry
   assertion fails.

This is a performance/policy defect, not data corruption or an RSS claim. Secure
admission before mutating useful existing entries, or use an atomic cache-total
reservation/transition that can account for planned eviction. A conservative
optional-retention refusal may keep old data. Same-key replacement of invalid
data needs its separate invalidation semantics; stale values must not be kept.

The same eviction-before-admission ordering is visible in
`commands/src/read_cache.rs`, `commands/src/cover_encoding.rs` (hot-cache clear)
and `core/src/cover/resources.rs`. Those are source-backed analogous paths; the
executable failure in this recheck targets the derived cache. Correct and test
all participating stores rather than only the demonstrated one.

## P2 — rename and move swallow native preview cancellation

`commands/src/lib.rs`, `render_target_under`, maps every cached-read failure to
`Err(None)`. `preview_rename_with_working` and `preview_move_with_working` treat
that as an ordinary skip, without separately checking the inherited cancellation
context. They can return `Ok` with a shortened/empty plan after cancellation.

The two public-API canaries use valid disposable tagged WAV files and an already
cancelled `with_preview_cancellation` callback. Both returned an accepted empty
plan instead of an error. Native media bytes remained unchanged. These fail:

- `cancelled_rename_preview_cannot_publish_a_shortened_native_plan`
- `cancelled_move_preview_cannot_publish_a_shortened_native_plan`

Check/propagate cancellation before and after rendering and before returning a
plan, while retaining legacy ordinary unreadable/missing-value skip behaviour.
The desktop queue's final supersession check can reject a result before UI
publication; that does not establish the exported native cancellation contract
or stop the old work at its next file boundary.

The parent accepted these findings and added explicit checks around rendering,
path validation and folder-extra precomputation. A fresh filtered run then
passed both cancellation canaries (two passed, zero failed). The corrections
belong to the parent; this reviewer did not edit production logic. Before that
correction, `validate_track_paths` could canonicalize a complete selection before
the next check, and folder-extra recursion lacked checkpoints. The filtered run
does not independently time every recursive/filesystem boundary. Native
filesystem calls and parsers already executing remain noninterruptible.

## Corrections independently exercised

Fresh runs passed the original corrected commands resource adversarial binary
(20 tests) and core resource adversarial binary (two tests). The conditions
include bounded/dead weak encoding ownership, typed busy artwork inspection,
command warning preservation, policy changes during a recipe and a separate
accounting model. Code inspection confirms bounded weak key/candidate/byte
limits, charged weak payload lifetime, bounded pruning scans, 4 KiB content
comparison scratch, and recipe generation checks between files/steps and before
publication. These are participating-owner guarantees, not a process-memory cap.

Additional passing probes in the new review file cover:

- Shared job-pressure refusal before table parsing, zero leaked transient/job
  admission, clean retry, unchanged saved metadata and cache release on App drop.
- An alternate semicolon/single-quote dialect with escaped quotes, plus explicit
  empty-cell clearing blocked by field locks while another mapped value remains.
- Header-only out-of-range mappings and ambiguous filename selection refusal
  before media reads.
- Cancellation-scope restoration after unwinding and separation between workers.

The new review binary initially ran eleven tests: eight passed and three failed
(the two cancellation canaries and one cache canary). Three passing cache tests
are included unchanged from the production private store source; they are not
new discoveries. Test fixtures are tiny generated PCM WAVs under the development
environment's task temporary directory and are removed by their guard. No real
media, profile, live app or existing Undo data was touched.

Commands used the existing environment/toolchain/cache:

```sh
source /mnt/Games/Development/tagrex/dev-env.sh
cargo test -p tagrex-commands --test resource_import_fresh_recheck -- --nocapture
cargo test -p tagrex-commands --test resource_adversarial -- --nocapture
cargo test -p tagrex-core --test resource_adversarial -- --nocapture
cargo clippy -p tagrex-commands --test resource_import_fresh_recheck --all-features -- -D warnings
rustfmt --edition 2021 --check crates/commands/tests/resource_import_fresh_recheck.rs
git diff --check
```

The targeted test Clippy command passed with warnings denied; the owned test's
format check and workspace whitespace check passed. The parent's concurrent
corrections require another full fresh canary run. The two cancellation canaries
were freshly rerun and passed after the parent correction. The cache refusal
finding still requires a correction and recheck at handoff; the first failing
results must not be presented as the state of a later candidate.

## Evidence boundary and route

Tier 2 discovery used `tagrex-personal-app`, generation
`2026-11-13T16:10:57Z`, rooted at the other checkout. Search and a complete
depth-one `App.read_snapshot` trace were used only to locate structural seams.
The snippet did not match its symbol bounds. Exact cited paths were checked;
most are changed, missing or not tracked in that generation. Conclusions rely
on source read in the implementation checkout, not exhaustive graph coverage.

Windows/macOS runtime identity, whole-operation immutable settings through Apply,
all outstanding Arc ownership, fairness, aggregate unintegrated owners and
physical-device behaviour remain outside this approval. Table arbitrary repeated
values/index mappings still depend on the storage migration. A passing native
engine is not desktop feature completion. The final source check observed the
queue wrapping admitted latest jobs in `with_preview_cancellation`, while normal
jobs bypass that context and both retain their worker permit. This new queue
integration was not runtime-tested by this reviewer; its final combined gate
remains necessary. No broader readiness/certification is inferred.

Reusable core/command fixes target curated upstream `tagrex/tagrex:main` after
integration; service/fork compatibility needs separate curation. No commit, PR,
merge, publication or installer was produced by this recheck.
