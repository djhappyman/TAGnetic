# Shared backend resource and cache readiness

Implementation baseline: `feat/native-field-foundations`, `9da71249`, 18 November
2026. This is a scoped implementation checkpoint for the backend programme,
not a process-memory guarantee, complete roadmap or platform certification.

## Implemented boundary

`tagrex_core::resources` provides a shared nonblocking coordinator with retained
bytes, transient bytes and expensive-job admission. One atomic reservation checks
all class limits and the combined byte limit. An exclusive RAII lease releases
its charge on normal return, error, cancellation-owner drop or panic; resizing
is atomic and leaves the existing charge intact on refusal. Errors distinguish
a request too large for policy from retryable concurrent pressure. Usage and
per-class high-water snapshots contain no user metadata.

Production participants share limits of 256 MiB retained, 384 MiB transient,
512 MiB combined and eight expensive jobs. These are conservative policy choices,
not an empirically optimal configuration or an allocator/RSS ceiling.

Actual integrations include metadata cache entries, derived query cache entries,
request-local hot base64 artwork, core encoded artwork results and artwork
header/decode/resize admission. Artwork reserves before decoder allocations;
row decoding reserves the larger permitted fallback path when full decoding is
also allowed. Optional cache retention can be refused without failing the read
or flushing existing hot entries solely to retry global admission. Each cache
still enforces its own byte/count/LRU policy. Artwork's existing best-effort
resize API retains the original image if processing cannot be admitted, preserving
its current unsupported/oversized fallback contract.

Cache leases cover their owner's retention. Other Arc owners can keep the same
allocations alive after cache eviction; their lifetimes require separate
accounting. Shared allocations held by multiple participating caches may be
conservatively charged more than once. Artwork weak indexes are now charged through their actual retained payload
lifetime following independent review. Parser temporaries, base query rows,
service stage/journal state and other operations are not all charged by this
checkpoint. Therefore the shared limit must not be described as a
whole-app memory bound. Admission also does not interrupt an in-progress parser
or filesystem syscall. Further owners should reserve before building buffers
and keep reservations through their actual allocation lifetime.

## Cache interpretation and identity

Process-global read priority, multi-value separator and legacy-codepage setters
advance a revision when their stored policy changes. Fingerprints capture policy
revision before filesystem observation; cache misses compare it after parsing.
A policy change during parsing refuses that result. Pure reads and row batches
retry interruption at most three times, never repeating a mutation. Failed row
attempts do not publish partial query updates. Persistent churn fails explicitly.

Query snapshots retain their originating policy revision, including derived
snapshots; stale snapshots fail `is_current`, and App query retrieval rebuilds
after a policy change. Existing settings/write/Undo invalidation remains. This
closes metadata-cache and query-reuse gaps between different App sessions using
the same process-global settings. Field recipes now bind the whole operation to its initial revision and refuse
a changed policy between files/steps and before publication. This is not an
immutable policy snapshot across preview and Apply. A raw `list_tracks` return can reflect a
concurrent settings change, but its index retains the old generation and cannot
be certified current; this broader operation contract remains a follow-up.

Windows fingerprints now use an open handle's volume serial/file ID and native
change time together with length/mtime; metadata-only Windows query identity
also follows that handle path. If the filesystem/API cannot supply identity,
optional caching/reuse is refused instead of falling back to size/mtime authority.
Unix retains device/inode/change-time identity. These remain display identities;
Executor write/Undo authority still bypasses preview caches.

## Verification evidence

Existing environment, dependencies and build cache were reused:

```sh
source /mnt/Games/Development/tagrex/dev-env.sh
cargo test -p tagrex-core resources:: --lib
cargo test -p tagrex-commands read_cache --lib
cargo test -p tagrex-core cover:: --lib
cargo test -p tagrex-core --lib
cargo test -p tagrex-commands --lib --quiet
cargo test -p tagrex-commands --lib shared_resource_cache_pressure_benchmark -- --ignored --nocapture
cargo clippy -p tagrex-core -p tagrex-commands --lib --all-features -- -D warnings
```

The initial resource suite passed seven tests, including existing artwork resource
checks; the full core library subsequently passed 235 tests. The command library
passed 364 tests with eight ignored on its first complete default-parallel run
after retry integration. A later command-library rerun passed 365 tests with nine ignored; the manual
pressure probe was then rerun successfully (86.829 ms first pass, warm
1.946/1.693/1.492/1.492/1.492 ms, 83.510 ms under pressure, same retained charge).
Core/commands production-library Clippy passed after that rerun. The corresponding
core-library rerun was blocked by another worker's in-progress rusqlite u64 test
compilation; it does not replace the earlier 235-test core result. Shared
work continued concurrently, so the parent must run the final combined gate on
the reviewed candidate; these counts do not certify later unrelated edits.

Tests include 16 simultaneous threads performing 2,000 acquisition/resize attempts
each, mixed retained/transient owners, combined/class/job saturation, arithmetic
overflow, failed resize preserving the old reservation, errors, unwinding and
return to zero charges. Cache tests exercise two independent caches competing
with transient work, pressure refusal preserving existing hot entries and reads,
and capacity recovery after owner drop. Artwork pressure preserves original bytes,
type and description, then successfully resizes after capacity returns.

A subprocess isolates process-global settings from unrelated parallel tests.
It verifies peer-cache invalidation, a setting change injected between parse and
publication, refusal without retention, retry and query-generation rebuilding.
The first full parallel command run exposed a real integration issue: strict
policy refusal surfaced an Interrupted error in an existing staged-import test.
Bounded pure-read retries were added; the next full default-parallel run passed.
The fix is not a claim that all operation-level policy races are solved.

The ignored manual probe generated 2,500 tiny tagged WAV files under the task's
temporary root, checked every title on six complete passes, then ran a reverse
read through a separate cache while a competing job occupied all remaining
combined admission. First pass: 92.200 ms; five warm passes:
2.353/2.191/2.313/2.053/2.088 ms. Pressure pass: 88.500 ms. Retained metadata charge:
2,398,060 bytes. The pressured cache retained no new snapshots; values remained
correct and charges returned to zero after owner drop. This is native metadata
read/cache timing on tiny synthetic fixtures, excluding UI/IPC, Apply, journal
writes and cold-disk claims. It is not a baseline speedup or peak RSS result.
The fixture owner removes its files on drop.

`cargo check -p tagrex-commands --target x86_64-pc-windows-gnu --locked` was attempted
using the existing target and stopped in SQLite/ring build scripts because
`x86_64-w64-mingw32-gcc` is unavailable. No toolchain/dependency installation was
performed. The exact Windows fingerprint struct/implementation was separately
compiled to Windows GNU metadata against the existing `windows-sys` rmeta; this
checks Rust/API types only, not the full crate, linking, Windows runtime or
filesystem behaviour. The disposable compile probe was removed.

## Remaining gate and route

Parent review and combined tests/Clippy/formatting are required. Windows runtime
identity tests, unsupported-filesystem behaviour and wider platform checks remain
pending. Whole-operation immutable policy snapshots, fair resource admission,
per-owner diagnostics, total allocations/Arc lifetime accounting and additional
operation integrations remain explicit follow-up work. No user audio, live
profile, Undo history, app process or UI was changed.

Reusable core/cache changes are eligible for curated `tagrex/tagrex:main` after
parent integration; service/fork compatibility must be assessed separately.
No commit, publication, PR, merge or installer is claimed here.


## Corrections following independent adversarial review

The separate [resource review](backend-resource-independent-review.md) found
three real defects. Its original report remains the record of the independent
findings, not a claim that the original implementation passed. All three original
failure conditions were reproduced before correction:

- Dead weak base64 allocations: the independent allocator probe reported
  102,076,192 live allocated bytes against 29,362,464 participating retained bytes,
  leaving 72,713,728 bytes outside that charge.
- Artwork processing pressure preserved the source image but suppressed its
  size warning through the boolean inspection API.
- A two-file field recipe accepted artist strings read under two different
  global separator policies in one valid plan.

Corrections are in `commands/cover_encoding.rs`, `commands/field_operations.rs`,
`core/cover.rs` and the narrow `PlanMessage`, `CoverEmbedOptions` and cover-note
regions of `commands/lib.rs`.

Weak encoding references now retain their resource lease until the reference is
actually removed, including after the last strong Arc dies. Limits are 4,096
weak candidates overall, eight collision candidates per key and 64 MiB charged
payload/bookkeeping. The raw-pointer acceleration index has its own 256-entry,
32 MiB charged lifetime limit, including both weak payloads. Dead references are
pruned on the 16-call cadence and after hot-cache eviction. Bounded indexes are
not rescanned on every capacity refusal. Saturation skips optional new index
admission rather than discarding admitted live identities. Lookup does not
allocate a decoded image: exact base64 comparisons use 4 KiB stack scratch.

Hot, weak and identity owner charges can overlap the same allocation. This is
conservative admission, not exact unique-allocation accounting. After correction,
the same independent 64-by-1-MiB workload reported 29,367,936 live allocated bytes,
75,504,480 participating charge and zero uncharged excess by its metric. This is
an allocator counter in an isolated synthetic test, not RSS or a general peak
memory result. Scope drop releases all its leases. Existing live canonical-string,
collision, Arc mutation and nested-scope tests remain. A new 20,000-item workload
checks bounded candidate/byte counts with 8,000 retained caller strings and
continued reuse of an admitted canonical string.

`inspect_cover_size` reports Disabled, WithinLimit, OverLimit, Unknown or Busy.
Known outcomes carry header dimensions. `is_over_max` remains a compatibility
wrapper; false is not proof of fit. User-facing native cover plans now emit
`plan.coverSizeUncheckedBusy` or `plan.coverSizeUnknown` when inspection cannot
establish size, and retain `plan.coverNotShrunk` only for known oversized output.
These new message keys have English plan-description fallbacks; frontend
localisation remains a separate UI task. No invented dimensions or blanket
oversized classification is used for malformed/small images.

The original core boolean canary was adapted to assert the new explicit Busy
outcome; its regression intent is retained by a new public command-level canary.
That test fills every processing slot, prepares and previews the image, asserts
an unchecked-size warning and no false oversized claim, verifies source audio
and image bytes remain unchanged, and verifies a successful warning-free retry.
Core checks distinguish known dimensions, disabled limits and malformed headers.
This test change follows the review's requested typed-outcome remedy, not a
weakened requirement that hidden pressure be accepted.

The field-recipe canary retains its original failing condition. Additional
cases change policy between steps and at the last cancellation/publication probe;
all must refuse the result, preserve disk bytes and allow a later stable recipe.
No partial plan is published; this minimum generation binding is not a complete
immutable policy migration or a guarantee for every other operation family.

Verified after corrections: core adversarial tests 2/2 passed, command adversarial
tests 20/20 passed, and all ten core cover tests passed. A concurrent whole-command
library run passed 371 cases with ten ignored and failed one parent-owned
in-progress legacy preview-cancellation test in `tabular.rs`; that failure was
reported to the parent rather than changed here. Clippy initially stopped on
other-owned `journal.rs` needless-borrow warnings. The parent owns the final
combined gate and a fresh separate recheck; no entire-backend completion follows
from these scoped results.


## Fresh recheck: retain useful entries when admission fails

A second independent [resource/import recheck](backend-resource-import-recheck.md)
found that local eviction happened before shared newcomer admission. The
reviewer's derived-cache canary filled the shared budget and showed an eight-byte
hot entry disappearing even though the nine-byte newcomer was still refused.
The analogous metadata, core encoded-artwork and pool-hot paths had the same
ordering. This was a performance/policy failure, not a data-corruption claim.

All four paths now obtain the newcomer's lease before evicting unrelated local
entries. If simultaneous old+new ownership cannot fit, optional new retention is
conservatively skipped. This can decline a replacement that an atomic cache-total
transition might have admitted; it preserves current useful data and avoids
releasing it speculatively. The application still receives freshly computed
results. Stale same-path metadata and intentional same-key replacements are
invalidated before reservation, so a refused new value cannot expose an old
known-stale value. Unrelated entries remain intact.

New tests exercise each participating cache under real coordinator pressure,
covering larger refused newcomers, successful admission after pressure releases,
same-key/path invalidation, unrelated-hot survival, oversized requests and
return to baseline charges. Empty core artwork still charges 128 bytes of index
metadata, empty pool artwork charges 192 bytes, and empty metadata values retain
at least the existing 640-byte file/index estimate. The generic derived store
keeps its caller-supplied inclusive-cost contract; zero estimates remain bounded
by entry count and production query callers supply metadata-inclusive estimates.
It is not an allocator-level size measurement.

The independent cache-pressure model is unchanged. Its fresh targeted run passed.
Targeted new pressure/replacement tests also passed across all four stores. The
full review binaries and core/commands all-target/all-feature Clippy are rerun
for this candidate below; the parent retains final integration ownership. No
user media, profile, runtime, unrelated implementation or reviewer model was
modified to obtain these results.


Fresh post-correction review runs passed: resource/import recheck 13/13, command
resource adversarial 21/21 and core resource adversarial 2/2. All four new cache
pressure/replacement checks passed; metadata stale-replacement and empty-value
accounting passed separately. One test-only default-field assignment lint was
corrected before the final Clippy rerun. Earlier unrelated cancellation and
journal warnings are historical intermediate results, not this candidate's
final integration verdict.

Final scoped lint gate: `cargo clippy -p tagrex-core -p tagrex-commands
--all-targets --all-features -- -D warnings` passed after the correction. The
metadata stale-replacement test was freshly rerun and passed. Owned-source
rustfmt checks and `git diff --check` passed. The parent still owns the workspace
combined gate, platform validation and separate fresh recheck.
