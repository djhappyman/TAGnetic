# Independent playback ownership/router recheck

18 November 2026. Base `9da71249` plus the shared backend programme edits.
This reviewer did not author the implementation or previous reviews. Production
source, runtime, profile and user media were untouched. Owned outputs are this
report and `player/tests/ownership_final_recheck.rs`.

Intent: preserve bounded source-specific seek ownership across supersession and
terminal cancellation, keep obsolete decoder output from escaping after refusal,
and let replacement controls proceed while an old source is destroyed.

**Verdict: the inspected ownership/router corrections are acceptable for parent
integration. No new confirmed production defect was found in this bounded
recheck.** This is not approval of the whole backend roadmap, physical playback,
all platform implementations or a hard real-time guarantee.

## Review and distinct probes

Source review covered `controlled.rs` request state and ownership flags, actor
handoff/acceptance/deadline/Stop handling in `lib.rs`, fixed physical-format
conversion and seek buffer reset in `fixed.rs`, pending/current replacement and
destruction in `router.rs`, and latest-preview permit/cancellation handling in
`app/src/blocking.rs`. The locked Rodio 0.23.0 converter implementation was read
locally. Lock ordering was checked around pending/latest/ticket state; the
inspected consumer does not hold those request/control locks across decoder I/O.

Two new deterministic probes challenge different schedules:

1. Complete an old seek but do not accept its acknowledgement. Supersede it,
   enter the newer seek into blocked decoder I/O, then attempt stale acceptance
   of the old ticket before terminally cancelling the new one. The source emits
   no sample after release, and subsequent requests are refused. A stale
   acknowledgement cannot clear ownership of the different in-flight request.
2. Block destruction of an old active source, then perform 10,000 replacements
   from another control thread. The burst completes within its two-second test
   deadline while destruction remains blocked. Only the final pending counted
   source remains, the eventual output contains the final source or queue
   bootstrap silence, and all counted sources drop after cleanup. Both owned
   threads are released and joined before assertions on responsiveness.

The first version of the second probe accidentally retained an additional
external Rodio player handle while asserting only router-owned retention. It
failed that overstrong fixture assertion. The probe was corrected to release
superseded external handles, matching the claimed ownership boundary; no
production change or false leak finding resulted. The final handle remains
alive to verify actual final-source output. The original reviewer canaries and
their expected outcomes were not weakened.

The existing real-converter matrix and transition tests were also rerun unchanged.
They compare actual Rodio channel/rate conversion, pause/resume, seek/reset,
refusal/prefetch and end conditions. Existing blocked-I/O supersession, latest
success/refusal/timeout, acceptance and router destructor canaries remain green.

## Verification

Using the existing development environment/cache:

```sh
source /mnt/Games/Development/tagrex/dev-env.sh
cargo test -p tagrex-player --test ownership_final_recheck -- --nocapture
cargo test -p tagrex-player --test fixed_pipeline_recheck --test seek_ownership_stress --test adversarial_controls --test control_span_transitions -- --nocapture
cargo test -p tagrex-player --lib --all-features --quiet
cargo clippy -p tagrex-player --test ownership_final_recheck --all-features --no-deps -- -D warnings
rustfmt --edition 2021 --check crates/player/tests/ownership_final_recheck.rs
git diff --check
```

Observed results:

- New recheck binary: **10 passed**, comprising the two new probes plus six
  included original control tests and two included original router tests.
- Existing binaries: **11**, **12**, **8** and **16 passed**, respectively.
  Included module unit tests repeat across these binaries; these counts are not
  all distinct new cases.
- Player library: **57 passed, two ignored**. The parent owns explicit manual
  adapter/performance probes; they were not redundantly rerun here.
- New test Clippy with `--no-deps`, owned-file formatting and whitespace checks
  passed. An ordinary Clippy attempt was blocked by unrelated in-progress exact
  metadata code in `core/model/exact*` (unused items/type complexity). No foreign
  source was edited or warnings hidden. The combined workspace gate remains
  required after that component is wired.

No generated media or temporary filesystem trees were needed for these probes.
All test-owned threads were joined. No commits, publication or PR work occurred.

## Limits and remaining readiness work

The request slots and inspected processing turns are bounded; this is not a
whole-player RSS or allocator bound. Source decoding, decoder opening/prefetch
and destruction can still consume time on their executing thread. In particular,
releasing the router mutex does not make arbitrary source destructors nonblocking.
The replacement test proves control-lock independence, not physical callback
latency or audible gapless quality.

Runtime output-loss detection/reconnection, retry lifecycle, physical devices,
Windows/macOS sound stacks and network filesystem behaviour remain separate
validation/implementation gates. Overall player readiness must retain those
limitations. This review introduces no release-profile performance claim.

Discovery used Tier 2 graph `tagrex-personal-app`, generation
`2026-11-13T16:10:57Z`, rooted at the other checkout. Coverage marks `lib.rs` and
the app queue changed, and the new adapters/tests missing. The graph's Player
snippet/seek trace did not match current source, so findings rely on actual
branch reads rather than graph absence or completeness claims. Reusable Rust
changes route to curated upstream `tagrex/tagrex:main` after parent integration;
no upstream or platform acceptance is inferred.
