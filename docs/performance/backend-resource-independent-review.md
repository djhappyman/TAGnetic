# Independent resource/cache adversarial review

Date: 18 November 2026. Integration baseline: `9da71249` plus the uncommitted
backend resource/cache wave. This reviewer did not author the implementation.
The review owns only the two `tests/resource_adversarial.rs` files and this
report. Production corrections belong to the parent/implementing agents.

Intent: coordinate participating retained/transient memory and expensive work,
keep metadata caches coherent after settings/file changes, and establish a
reliable boundary for later UI exposure. Initial verdict: **request changes**
for the three reproduced conditions below. This is a scoped review, not a
whole-application security or memory certification.

## Confirmed findings

### P1 — dead weak artwork encodings retain request-sized payloads

`crates/commands/src/cover_encoding.rs`, `Pool::encode`, inserts every newly
encoded string into `weak`. The periodic sweep visits only `identity`. A dead
`Weak<str>` still owns the allocation containing its former string bytes, so
clearing `hot` and releasing its leases does not free those base64 allocations.
The weak map grows with distinct artwork over the whole request.

The independent test compiles the exact private encoding-pool source without
changing production visibility. An isolated subprocess wraps the system
allocator with a counter of live allocated bytes (not RSS/high-water estimates).
It encodes 64 distinct 1 MiB images and immediately drops every caller result.
Observed outstanding bytes: **102,076,192**; participating retained charge:
**29,362,464**; difference: **72,713,728**. The test permits 24 MiB for bounded
identity cadence/bookkeeping and fails beyond that margin. No old caller Arc
explains the excess. Dropping the request scope frees the allocations; this is
request-lifetime growth, not a permanent process leak.

This weak-index limitation was disclosed by the author and predates the new
leases. Nevertheless it remains a concrete obstacle to treating the artwork
resource boundary as ready for large batches. Prune dead weak strings, bound
keys/candidates, and account for any deliberately retained allocations. Preserve
live-result sharing; do not solve it by duplicating base64 for every file.

Reproduction:

```sh
cargo test -p tagrex-commands --test resource_adversarial dead_encoding_weak_entries -- --nocapture
```

### P2 — pressure suppresses the oversized-artwork diagnostic

`crates/core/src/cover.rs`, `is_over_max`, returns `false` when shared admission
fails. `resize_cover` correctly preserves the original art on that same failure,
but callers such as `CoverEmbedOptions::read_image` use the subsequent boolean
to decide whether to include `CoverNotShrunk`. A busy backend therefore reports
no warning for a valid, oversized image that it did not process.

The public-API test creates a 200×100 PNG and verifies its dimensions exceed a
100-pixel limit. It occupies all eight job slots, confirms resize retained the
original Arc, then fails because `is_over_max` has become false. There is no
malformed image, decoding ambiguity or exhausted physical memory in this case.

Use an explicit header-inspection outcome (known dimensions, unknown or busy)
and retain processing-skipped diagnostics. An unknown/busy result must not be
treated as proof that an image fits. Avoid indiscriminately claiming that small
or malformed images are oversized. Existing API wrappers can remain compatible.

Reproduction:

```sh
cargo test -p tagrex-core --test resource_adversarial oversized_artwork_warning -- --nocapture
```

### P2 — one recipe can publish values interpreted under two policies

`crates/commands/src/field_operations.rs`, `App::run_field_steps`, does not bind
the entire batch to a parse-policy revision. Per-read cache checks can each be
correct while two files within a single recipe use different settings.

The public-API test creates two WAVs carrying separate artist values. A second
thread changes the multi-value separator at the existing cancellation probe
between files. Both original files remain byte-identical, but a single
artist-to-comment recipe succeeds with `First; Second` for the first file and
`First / Second` for the second, both marked `invalid=false`.

The author disclosed whole-operation policy snapshots as unfinished. This test
makes that limitation executable: rejecting an individual parse race does not
prevent mixed-policy publication. At minimum capture the operation's starting
revision and refuse a changed generation before publication; checking between
files avoids wasting work. Broader immutable operation policy and settings/source
identity across preview and Apply remain separate requirements.

Reproduction:

```sh
cargo test -p tagrex-commands --test resource_adversarial field_recipe_rejects -- --nocapture
```

## Other independent checks

A separate 50,000-operation model-based test exercises arbitrary acquire,
resize, refusal and drop sequences against an independently maintained sum of
owner charges. It checks TooLarge versus Busy and exact accounting after every
operation. This passed. Source review found no lock-order cycle involving the
coordinator: its guard is not retained while callers acquire cache/artwork locks.
That is bounded source evidence, not proof against every future deadlock.

The exclusive lease design, checked combined admission and preservation of the
old charge on failed resize are useful, clear invariants. Optional retention
refusal preserves read correctness. Strong Windows identity fallback is
appropriately conservative when APIs cannot provide a file ID. Windows runtime
behaviour was not tested by this reviewer.

Explicit remaining limitations are still material: cache-owner leases do not
follow outstanding Arc allocations after eviction; base query rows, computed
overlays, parser temporaries and stage/journal owners are not all covered by
shared accounting; raw `list_tracks` is not a frozen-policy operation. No
whole-process RSS ceiling or complete UI-readiness claim is supported.

## Verification and ownership

Commands were run with the existing environment:

```sh
source /mnt/Games/Development/tagrex/dev-env.sh
cargo test -p tagrex-core --test resource_adversarial -- --nocapture
cargo test -p tagrex-commands --test resource_adversarial dead_encoding_weak_entries -- --nocapture
cargo test -p tagrex-commands --test resource_adversarial field_recipe_rejects -- --nocapture
cargo clippy -p tagrex-core -p tagrex-commands --test resource_adversarial -- -D warnings
git diff --check
```

Initial results: model test passed; the three regression conditions failed as
described; targeted Clippy and whitespace checks passed. A passing lint result
does not clear the behavioural failures. The failing tests intentionally assert
the required outcome and must pass after production fixes before this review is
closed. No production code, app process, profile or existing Undo history was
changed. Generated WAV fixtures used a task-owned directory beneath the project
temporary root and were removed by their owner after each isolated run; PNG and
allocator probes used memory only.

Graph evidence was Tier 2 discovery using `index_status`, `search_graph`,
`trace_path` and `get_code_snippet` for read snapshots/cache. The indexed project
`tagrex-personal-app` points to another checkout at generation
`2026-11-13T16:10:57Z`; snippets did not match symbol bounds. Coverage was checked
for all relied-on source paths and the new test paths. Stale/missing/untracked
ranges were read directly from this implementation checkout. No absence or
exhaustive impact conclusion is inferred from that graph.

Reusable corrections are candidates for curated `tagrex/tagrex:main` after the
parent's integration gate. Fork service/storage compatibility remains separate.
No commit, PR, merge, release or full-platform certification is claimed here.
