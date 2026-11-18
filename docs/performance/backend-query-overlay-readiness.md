# Computed query overlay ownership and bounded rendering

Implementing checkpoint; separate adversarial review and integrated delivery gates
remain pending. No UI or whole-backend completion is implied.

## Behavior and ownership

Computed fields now validate bounded, unique definitions before serializing their
cache key. A small 1 MiB transient lookup allowance covers worst-case JSON escaping
and growth for 32 definitions (keys <=128 bytes, masks <=2048 bytes). Warm hits
reuse immutable rows without taking an expensive-job slot or rescanning source
field names. Failed cold admission, invalid input, cancellation and panics leave
the previous hot overlay intact.

Cold construction takes one shared job and grows admission before each parsed
AST, aggregate input clone, row clone, render and retained insertion. The caps
are 64 MiB retained overlay state, 128 MiB transient work, 64 MiB per-mask
AST-derived rendering work, 8 MiB aggregate cloned source input per row and 512
referenced fields. These are limits, not eager 64/128 MiB reservations: tiny
concurrent queries retain proportionately small charges. Mask compilation uses
512 source bytes plus 8 KiB per mask as a conservative allowance, with 64 KiB
bookkeeping allowance. Render-only parsing avoids the extraction regex entirely.

Computed maps are immutable shared `Arc`s, separate from the small technical
property map. Rows and indexes share a batch lease until their last owner drops;
saved base text keeps its original owner and is not charged again. Keys, row/map
structure, paths, raw/folded strings and conservative allocation overhead are
included. Large old-cache destruction occurs outside the cache mutex. Resource
refusals remain typed `resource_busy` / `resource_limit` through service routing
and `error.resourceBusy` / `error.resourceLimit` through desktop errors.

This is participating-owner accounting, not an RSS or allocator guarantee.
Result DTOs/indices and other nonparticipating owners remain a separate programme
scope. The current computed cache retains one definition set per root index;
competing definition sets can cause rebuilds and cold builds are not coalesced.
Those are throughput limitations, not silent partial-output behavior.

## Shared bounded mask seam

The existing grammar/parser remains authoritative. New `Mask::parse_for_render`
builds that same AST with `regex = None`; extraction explicitly refuses. Existing
`Mask::parse` and extraction retain their regex and behavior.

`render_work_bytes` derives a conservative cumulative allocation allowance from
the actual AST and borrowed input lengths. It includes every argument and section
buffer, string growth, known function scratch and error formatting; cumulative
allocation exceeds simultaneous live allocations. An exhaustive `Function` match
bounds Unicode casing, character-vector operations, padding/count ceilings,
replacement growth, prefix argument vectors, arithmetic and output copies.
Known literal pad counts and replacement input lengths tighten small workloads.
Overflow refuses. Quoted commas or parentheses do not invent argument buffers.

`render_with_budget` and `render_value_with_budget` check that allowance before
rendering. The caller must acquire the corresponding coordinator lease. The
first retains established filename rendering; the second is unsanitised value
rendering with missing fields empty. New bounded-render numeric diagnostics cap
retained argument details at 1 KiB plus an ellipsis, avoiding giant errors quoting
whole tags. Legacy render methods retain their old diagnostics.

A real Unicode stress check also found an existing `$cutmix` panic: comparing the
ASCII `clean` suffix against `尾尾x` split byte 2 inside a codepoint. The retained
canary failed before correction; `split_at_checked` now skips non-boundary ASCII
suffix candidates and preserves the text. This fix also protects ordinary legacy
mask rendering.

## Executed evidence

- Original parent allocation canary passed unchanged: approximately **9,678,590
  bytes** retained by the computed query, with **9,686,134 bytes charged**. Its
  original observation was 9,668,470 bytes with zero charge.
- Public overlay suite: **8 normal tests passed**; the manual cold/warm probe
  also passed explicitly. It covers busy warm reuse, preserved hot entries,
  cancellation at multiple construction checkpoints, panic release, real AST
  argument growth, aggregate input refusal, bounded numeric diagnostics, typed
  desktop errors and four contending workers with two available job slots.
- Independent System-allocation observations: **2 passed**. A malformed 2 MiB
  definition refuses before allocating a JSON cache key (<64 KiB observed work),
  and nine 1 MiB fields refuse before cloning their aggregate (<1 MiB observed).
- New core mask suite: **4 passed**, covering original Unicode panic, shared
  grammar/render equivalence and explicit extraction refusal, all function
  families/nested/error allocation bounds, and a 100-argument/1 MiB source bomb
  refusing before argument-buffer allocation.
- Query unit suite: **28 passed**, including an escaped final row clone retaining
  both base and batch ownership. Existing independent base suite: **7 passed**.
- Existing mask unit suite: **96 passed**. Commands library: **382 passed,
  10 ignored**. Service library: **150 passed** at its local checkpoint.
- Core/commands/service all-target, all-feature Clippy with denied warnings and
  desktop all-target/all-feature `cargo check` passed. Locked dependencies and
  the existing development environment/cache were used.

Representative commands:

```text
cargo test -p tagrex-core --test mask_render_budget --all-features --locked
cargo test -p tagrex-commands --test query_overlay_resources --test query_overlay_allocation --test query_overlay_memory_recheck --all-features --locked -- --include-ignored
cargo test -p tagrex-commands --lib --all-features --locked
cargo test -p tagrex-service --lib --all-features --locked
cargo clippy -p tagrex-core -p tagrex-commands -p tagrex-service --all-targets --all-features --locked -- -D warnings
cargo check -p tagrex --all-targets --all-features --locked
```

The synthetic debug-build measurement used 64 rows with roughly 32 KiB comments
and seven definition sets. Native query median cold time was **88.046 ms**, warm
**4.827 µs**; maxima 88.879 ms / 6.178 µs. Participating transient high-water was
about 9.88 MB; no transient/job charge survived the call. This is not UI/IPC
latency, an old/new speed comparison or a library-size guarantee.

## Review and route

No commits, publication, live app/profile changes or real-media writes. New tests
use synthetic rows and isolated subprocesses; no disposable media tree remains.
The new core helper, mask panic fix and query ownership are reusable backend
candidates for curated upstream `tagrex/tagrex:main`. Fork service integration
requires its normal compatibility review. Independent review must challenge the
AST/function allowance, clone ownership, concurrency, cache refusal and error
routing before this checkpoint is accepted. Unrelated recovery/native writer
blockers remain active and are not waived by these checks.
