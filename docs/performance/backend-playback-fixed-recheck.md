# Fresh fixed-output playback recheck

18 November 2026. Independent reviewer; base `9da71249` plus the shared backend
programme working changes. No production changes, commits, runtime/profile or
user-media access. Owned output: this report and
`crates/player/tests/fixed_pipeline_recheck.rs`.

Intent: keep source-specific seeks and Stop responsive without corrupting
conversion, exposing stale seek results or accumulating stalled output work.
Verdict: **request changes** for the two reproduced conditions below. The
previous dynamic-channel/rate defect is corrected on the inspected fixed-output
path, but that passing evidence does not waive request ownership or locking.

## P2 — Superseded in-flight seek loses actor ownership

Locations: `player/src/lib.rs:430` (`pending_seek.take()`),
`player/src/controlled.rs` (`Control::seek`, `take_request`, `prepare`).

New seek handling discards the actor's previous ticket without cancelling it.
Replacing the source's pending slot only cancels a ticket which has not already
been taken by the consumer. If the old decoder seek is blocked, that slot is
empty; the old request remains live without an actor-owned deadline.

The independent canary uses the production `Fixed` and `Controlled` modules and
a deliberately blocked valid Source seek. It reproduces the actor handoff:
drop the old actor ticket, submit another seek, cancel the latest ticket at its
deadline, then allow the earlier decoder call to finish. The source emits
`Some(32000.0)` from the old two-second target even though the only current
actor-owned request has been refused. The expected terminal output is `None`
for this cancelled/refused case. No physical device is involved.

Keep ownership of in-flight requests until completion or explicit cancellation.
Define supersession separately from Stop/timeout: a normal rapid seek must not
simply terminate the usable current track, while cancellation/refusal cannot
leave an earlier target free to resume audio. Add an actor-level test for the
same ordering as well as source-level acknowledgement tests.

## P2 — Replaced active source drops under the router control lock

Location: `player/src/router.rs:67–74`, `Router::next`.

The temporary guard in the `if let pending.lock().take()` scrutinee remains live
through the body. Assigning `self.current = Some(next)` drops the previous
output and its source while that guard still holds the shared pending mutex.
This is separate from `Handle::replace` correctly dropping a superseded pending
output outside its lock.

The independent canary activates an old source with deliberately blocked Drop,
then replaces it from the consumer. While destruction is blocked, another
`Handle::replace` cannot finish within the 200 ms test interval. Releasing Drop
allows both threads to finish and be joined before the assertion. This proves
control-lock dependence, not a physical slow-file reproduction or a universal
latency bound. A slow destructor can therefore delay the actor's fresh-sink
replacement and Stop path through the shared mutex.

Take the pending value in a separate inner scope, release its guard, and only
then replace/drop the active output. This does not eliminate all destructor work
on the audio callback; that remaining real-time cost needs profiling or a bounded
retirement design if required.

## Independent checks

```sh
source /mnt/Games/Development/tagrex/dev-env.sh
cargo test -p tagrex-player --test fixed_pipeline_recheck independent_ -- --nocapture
cargo clippy -p tagrex-player --test fixed_pipeline_recheck -- -D warnings
rustfmt --edition 2021 --check crates/player/tests/fixed_pipeline_recheck.rs
git diff --check
```

The test run produced **one passed, two failed**. The passing test compares exact
sample vectors/counts against the locked real Rodio `UniformSourceIterator`
across 24 combinations: 1/2/4/6 physical channels and
8/11.025/22.05/44.1/48/96 kHz, with a source changing both native channel count
and sample rate. It uses the complete production fixed-format control pipeline.
The two deliberately failing canaries above are retained for corrections.
Targeted test Clippy, owned-file formatting and whitespace checks passed.

The author’s actual-adapter and development-throughput results remain scoped
author evidence. No independent release-profile timing, physical-device loss,
reconnection, audible/gapless result or Windows/macOS runtime claim is made by
this recheck. Parent requested finishing the current defect report before a
release timing build. Queue source inspection confirms latest workers enter the
parent cancellation context while ordinary mutation jobs bypass it; command-side
cancellation is being changed by the parent and is not certified here.

Tier 2 graph discovery used `tagrex-personal-app`, generation
`2026-11-13T16:10:57Z` at the other checkout. Coverage marked old player/app
sources changed, new modules missing and mailbox untracked. The seek snippet
and trace did not describe the current body; findings rely on the actual working
source and locked Rodio 0.23.0 conversion/Source implementations. No exhaustive
negative graph claim is made.

Reusable Rust fixes route to curated upstream `tagrex/tagrex:main` after parent
integration. This report provides no full-roadmap or UI-readiness approval.
