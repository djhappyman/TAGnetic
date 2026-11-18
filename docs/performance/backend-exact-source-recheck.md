# Fresh exact-read source-authority review

Review date: 18 November 2026. Existing dependent foundation checkout; no commit
or publication by this reviewer. Scope is the held-source correction in
`model/exact/source.rs`, `exact/raw.rs`, `exact.rs` and public `TagEngine` readers.
Exact writes, stage/journal migration and whole-backend readiness are excluded.
Reusable read changes are candidates for curated upstream `tagrex/tagrex:main`.

## Finding and correction

**P2, corrected in the parent: reentrant public readers inherited another
request's source.** The original mixed-source correction pinned the exact read
correctly, but its thread-local authority also affected a new public legacy
request started by the cancellation callback. After an owned fixture parent
selected `Nested`, `TagEngine::read(path)` still returned the outer file's
`Outer`. `TagEngine::read_block(path, VorbisComments)` had the same problem.
The outer exact result itself remained internally consistent. This finding is
limited to reentrant public requests; no ordinary native-session trigger was
established.

Both regressions were retained unchanged in
`core/tests/exact_source_authority_fresh_review.rs`. Before correction the fresh
suite reported two passed and two failed. The parent added a scoped suspension
at all 21 public legacy `TagEngine` entry points. Private compatibility passes
inside the exact read still use the held source. Nested public requests read
their own path and restore the previous authority on exit, including errors and
panic unwinding. Source inspection confirms that the public exact entry retains
its own separate bind and private selection path.

## Different conditions tested

The fresh suite passed all eight probes after correction:

- Both original public legacy/block request regressions, without changing their
  expected source.
- An unsuccessful nested public read, followed by a visible third fixture source
  during the outer parser phase. The outer held source is restored before parsing.
- Same-logical-path nested exact success, cancellation and a caught callback panic;
  native and compatibility text remain consistent and charges return to baseline.
- A separate thread sees the current selected fixture instead of another
  thread's held source.
- Non-UTF8 logical filenames with 96 nested exact reads plus nested legacy reads;
  no shared-cursor confusion and byte-identical source files.
- Independent PCM/WAV envelopes: nested public duration, properties and full-row
  readers observe the 16 kHz/four-second source while the outer exact read keeps
  its 8 kHz/three-second source.

The eighth portable nested-reader probe passes on Linux and ensures Windows
will execute a meaningful test even though link-swap/non-UTF8 probes are
Unix-specific. It exercises twelve rounds of nested exact, full-row and legacy
requests, a failed public request and a separate thread, with unchanged source
bytes and released charges.

Final current-source run: **8 passed, zero failed or ignored**. Scoped
all-feature Clippy with denied warnings, owned rustfmt and whitespace checks
pass. No new confirmed defect remained in this corrected, bounded read scope.
Native Windows execution is still unverified; the portable source path is not
itself a Windows runtime result.

## Verification and limits

```sh
source /mnt/Games/Development/tagrex/dev-env.sh
cargo test -p tagrex-core --test exact_source_authority_fresh_review -- --nocapture
cargo clippy -p tagrex-core --test exact_source_authority_fresh_review --all-features --no-deps -- -D warnings
rustfmt --edition 2021 --check crates/core/tests/exact_source_authority_fresh_review.rs
git diff --check -- crates/core/tests/exact_source_authority_fresh_review.rs
```

Tier 2 graph project `tagrex-backend-readiness`, generation
`2026-11-18T12:48:55Z`, was used for discovery. Coverage reported changed source
and untracked new files, so current source was read directly. The graph's broad
resolver edges were not treated as complete call evidence. Public guard coverage
was checked against the actual `TagEngine` implementation.

All paths and media belong to disposable fixtures removed by their owners. No
runtime, profile, real media, journal or production source was edited by this
reviewer. Tests are finite evidence; native Windows/macOS checks, arbitrary
concurrent content writes and dependency parser allocation behavior remain
outside this review. The existing scalar writer preservation blocker is unchanged.
