# Independent exact-read review

Status: **changes required**, 18 November 2026. Scope is the first read-only
exact metadata component in `feat/native-field-foundations`, following
`9da71249`; the working checkout contains other active backend work. This is
not approval of exact writes, stage migration, journal compatibility, Undo,
platform runtime behaviour or the full backend programme.

The reviewer owns only `crates/core/tests/exact_read_adversarial.rs` and this
report. No production source, user media, live app/profile or history was changed.
Fixtures are generated under `std::env::temp_dir()` and removed by their owner.
Reusable core corrections are eligible for a curated upstream
`tagrex/tagrex:main` contribution after the parent integration gate; fork storage
compatibility remains separate.

## Ranked confirmed finding

### P1 — one exact read can return projections from two different files

`TagEngine::read_exact_with_cancel` captures raw metadata through one path open,
then calls the legacy `read_selected(path)` parser through separate path opens.
`raw::Input::ensure_unchanged` compares a reopened final-file witness with the
initial witness. It does not establish that the legacy parser read the same
source as the raw capture.

The retained regression
`changing_owned_parent_link_cannot_mix_legacy_and_exact_snapshots` constructs two
disposable fixture subdirectories, each with its own valid FLAC named
`track.flac`. One has `TITLE=First`; the other has `TITLE=Other`. A fixture-owned
parent directory link initially selects the first. The cooperative callback
changes that link immediately before legacy parsing and restores it immediately
afterwards. The final file itself has never changed, so its inode/change-time
witness still agrees. The function returns `Ok` containing native exact title
`First` and legacy title `Other`.

Actual failure:

```text
assertion `left == right` failed:
one returned read must describe the same file in both projections
  left: "First"
 right: "Other"
```

This violates the component's central same-read consistency guarantee. Later
consumers could cache a mixed result or show one baseline while preparing from
another. The current pure prepared intents remain non-executable, so this test
does not demonstrate an applied write or media loss.

Use one held source or a source authority covering every parse phase, and reject
inconsistent authority before publication. A final size/mtime/inode comparison
alone does not address this case. Preserve the failing assertion; do not weaken
the expected result to allow mixed projections. The orchestrator accepted this
finding and will assign the implementation to the component author, followed by
a fresh review.

## Other independent probes

The same public-API test binary exercised:

- **2,000 deterministic malformed-input mutations** of raw FLAC metadata/audio
  envelopes, including truncation, inserted NUL, mutated bytes and four-byte
  `0xff` ranges. Every call completed without an escaping panic, released its
  participating admission, and left fixture bytes unchanged. This is a bounded
  corpus, not a fuzzing coverage claim.
- **Cancellation at every observed cooperative checkpoint** of a real successful
  FLAC read. Each interrupted run returned `Cancelled`, returned no partial
  owner and restored current shared charges to their baseline.
- **Repeated/empty/Unicode/literal punctuation values** from an independently
  built raw comment block, plus an intent retained after dropping its source
  snapshot. The intent preserved all old members, stayed explicitly
  non-executable and retained a charge until its own drop. File bytes remained
  unchanged.

These three probes passed. The source-consistency canary failed. Success cases
in this binary are serialized only with each other to avoid conflating deliberate
shared-budget refusal with a representation assertion; production admission is
unchanged.

## Evidence and limits

Commands run with the reused toolchain/cache from
`/mnt/Games/Development/tagrex/dev-env.sh`:

```sh
cargo test -p tagrex-core --test exact_read_adversarial -- --nocapture
# 3 passed; 1 failed (confirmed consistency finding above)
rustfmt --edition 2021 crates/core/tests/exact_read_adversarial.rs
cargo clippy -p tagrex-core --test exact_read_adversarial --all-features --no-deps -- -D warnings
# passed
git diff --check -- crates/core/tests/exact_read_adversarial.rs
# passed
```

The parent asked this review to close after the P1 reproduction so the original
author can implement the correction. No full core/workspace test or final
all-target gate is claimed. Existing author ID3/MP4/Mutagen/ffprobe results were
read as context and were not independently rerun by this reviewer. Additional
raw ID3/MP4 fuzzing, allocator accounting, custom codepage representability,
Windows/macOS runtime checks and checks after the production correction remain
future verification scope. In particular, the exact-read component cannot clear
the separate deliberately failing scalar writer preservation canary.

Structural discovery used `codebase-memory` Tier 2,
`tagrex-backend-readiness`, generation `2026-11-18T12:48:55Z`, rooted in this
checkout. `search_graph` located exact types/adapters and a bounded outbound
trace from `read_exact` established the read phases. Material source was read
directly. Coverage checks reported `metadata_match` for `model.rs`, `exact.rs`,
`exact/raw.rs` and `values.rs`; existing `tests/exact_read.rs` had changed since
indexing and was read directly. No exhaustive graph coverage or absent-defect
claim follows from those signals.
