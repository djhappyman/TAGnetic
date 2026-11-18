# Independent playback, jobs and tabular-import review

Date: 18 November 2026. Review base: `9da71249` plus the current shared backend
programme edits. This reviewer did not author either implementation and changed
no production logic. The sole production-file edit is an approved test-only
module registration in `commands/src/tabular.rs`. No commits or publication.

Intent: preserve responsive, bounded playback controls under stalled output and
map quoted tabular data through ordinary staged plans without partial publication.
Verdict at the reviewed candidate: **request changes** for two confirmed
correctness/responsiveness findings below. These are scoped findings, not a
claim that every unmentioned path is defect-free.

## P2 — Controlled source discards native format-span boundaries

Location: `crates/player/src/controlled.rs`, `Source::current_span_len` (reviewed
lines 151–154). The wrapper always returns `512 * channels`, irrespective of
the inner source's actual span. Rodio's real `UniformSourceIterator` configures
its channel/rate conversions for the reported span. The new wrapper can
therefore continue applying the old format after the source changes it.

Independent executable canaries in `player/tests/adversarial_controls.rs`
include the production wrapper unchanged and compare it with the bare source
through the actual locked Rodio converter. Both fail on the reviewed candidate:

- A valid source changes from mono to stereo after four samples. Bare conversion
  duplicates the mono prefix and retains four distinct stereo tail samples
  (12 output samples); the controlled wrapper duplicates the stereo tail too
  (16 output samples).
- A valid source changes from 8 kHz to 16 kHz. Bare conversion to 8 kHz emits
  12 samples; the controlled wrapper emits all 20 using stale rate metadata.

This is a source-contract/audio correctness regression. The production Symphonia
adapter updates its signal specification from decoded packets, but this review
did not generate a real changing-format media file or claim a physical-device
reproduction. Preserve the underlying span boundary when emitting real audio;
paused silence and seek-induced format changes also need explicit handling.
Retain the old position/pause/cancellation tests while fixing it.

## P2 — Two-byte CSV tokens skip every cancellation checkpoint

Location: `crates/commands/src/tabular.rs`, `Records::next`, reviewed line 180.
Cancellation is checked only when the absolute byte offset is divisible by
4096. An opening quote puts the offset at an odd number; a run of escaped quotes
or CRLF pairs then advances by two and never reaches a divisible offset.

The approved test-only canaries in
`commands/tests/support/tabular_parser_adversarial.rs` pass a callback that is
already cancelled. The escaped-quote test nevertheless consumes the entire
262,146-byte record and invokes the callback **zero times**. A CRLF-only quoted
record has the same failure. Outer API cancellation checks may reject the final
result, but they do not make parsing work interruptible at the claimed interval.

Use a monotonic work/next-check threshold and test `>=`, rather than requiring
an exact modulo hit. Include the blank-line-skipping loop, which currently has
no cancellation checks at all. The fix should not weaken syntax, cell-size or
row limits. No stage/audio mutation is involved in either parser canary.

## Checks and observations

Using the existing development environment and build cache:

```sh
source /mnt/Games/Development/tagrex/dev-env.sh
cargo test -p tagrex-player --test adversarial_controls independent_ -- --nocapture
cargo test -p tagrex-commands --lib independent_ -- --nocapture
cargo clippy -p tagrex-player --test adversarial_controls -- -D warnings
```

Observed: both independent player canaries fail with the sample-count/value
differences above; both CSV canaries fail. A complete integration-binary run
also passed the six original controlled-source tests included unchanged through
the production module, while the new span canary failed. Player adversarial
test Clippy passed. Passing tests are not substituted for the failing canaries.
The parent owns fixes and fresh combined verification; this report must not be
read as certification of later edits.

Source review also checked the mailbox's finite/representable duration guards,
transport barriers, pending-source router replacement, source-bound seek tickets,
actor timeout publication, queue cancellation/permit ownership, explicit table
empty policies, aliases, native matching and service input-kind/grant routing.
No additional confirmed issue is claimed from those inspected ranges. Important
positive boundaries are preserved: mailbox rejection is explicit; seek state is
source-specific; there are no per-timeout worker threads; table parse errors
precede plan publication; imported paths do not grant service access.

## Remaining evidence limits

Physical output loss/reconnection, audible/gapless behaviour, Windows/macOS audio
stacks and legacy running-preview cancellation remain unverified or unfinished
as the implementation report already states. The fixed router bounds retained
sink slots; this is not a whole-player allocation/RSS or hard real-time guarantee.
Audio-thread mutex/drop costs and source-open/kernel-call latency need production
profiling. CSV's complete arbitrary repeated-value mappings await the storage
migration; final plan retention is a separate owner from parser admission.

Discovery used Tier 2 graph project `tagrex-personal-app`, generation
`2026-11-13T16:10:57Z` at the other checkout. Relevant paths were checked: current
player/app sources are changed, new modules missing and service modules not
tracked in that index. The Player.seek trace/snippet was stale and did not match
the current method body, so conclusions rely on actual branch source. Locked
Rodio 0.23.0 source was read locally for converter/span semantics. No exhaustive
negative graph or complete-impact claim is made.

Reviewer-owned files: the two canary files, their test-only registration and
this report. No temporary media, runtime process, user library, profile or Undo
history was used. Reusable Rust fixes route to curated `tagrex/tagrex:main` after
parent integration; service changes require separate fork compatibility review.
