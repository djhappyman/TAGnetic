# Fresh whole-collection move-context recheck

**Changes required: P2 native role ambiguity remains on Unix.** This is a
separate review of the whole-collection correction, using public Executor calls,
reopened SQLite and disposable PCM fixtures. Only this report and
`crates/core/tests/move_context_recheck.rs` were authored; no production edits.
Reusable corrections belong on curated upstream `tagrex/tagrex:main`. Fork
storage compatibility and upstream inline journal images remain separate gates.

## P2: a directory identity can authorize a file recovery move

The new collection validator binds the persisted stage identity to its typed
move proof. On Unix, however, `TRX-U1` records device, inode, birth time and only
an `is_symlink` boolean. `NativeIdentity::file_role(false)` and
`directory_role()` both accept a genuine non-link directory identity.

The retained normal canary
`genuine_directory_identity_cannot_be_promoted_to_file_move_authority` creates a
real pending stage through the public Executor. It copies the stage's actual
`directory_identity` into both the persisted moved-file proof and the stage's
`identity`, then closes and reopens SQLite. It does not invent native identity
bytes or alter the immutable plan endpoints.

Undo accepts this metadata context and moves the pending regular payload to its
original source before detecting that its identity differs. It returns an
error and retains history, but the filesystem has already changed despite the
invalid file-role evidence. The canary fails on the assertion that the source
must remain absent; the original payload bytes are not deleted in this case.
This is a corruption/refusal-ordering defect, not a claim of arbitrary remote
access or P1 byte loss.

Recommended correction: encode explicit regular-file/link/directory roles in
Unix native identities, or provide equivalent independently validated role
proof before recovery starts. An old untyped encoding must not silently count
as new typed move authority. Keep legacy history readable and fail closed where
its role cannot be proved. Preserve the unchanged regression; do not treat the
move-before-error as acceptable whole-collection validation.

## Passing independent probes

Seven other normal probes pass:

- Oversized later SQLite source/path/identity cells refuse without recovering
  an earlier completed transfer or the pending transfer.
- A duplicate native reservation expressed through a distinct textual path
  alias refuses the entire mixed collection. The existing SQLite unique
  constraint independently prevents literal duplicate-path insertion.
- A later proof changed to copy-kind cannot authorize its pending move; earlier
  completed material and history stay unchanged.
- Future native versions and unknown role bytes refuse before any recovery
  effect, including a mixed completed/pending batch.
- Ordinary mixed recovery preserves same-object external content edits and an
  unrelated sibling file, then removes only the owned empty reservations.
- Recovery preserves an independent hardlink name and its same-object edits.
- A nested public Executor interrupted while its context is active can recover
  separately; the outer recovery then completes with its original context and
  directory authority restored.

The pending fixture adapter records real SQLite intent before simulating a
process boundary with an unwind. The completed first transfer follows ordinary
production code. No production fault environment or user media is involved.
All fixture trees clean themselves on success or assertion failure.

## Exact local verification

```text
cargo test -p tagrex-core --test move_context_recheck --locked -- --nocapture
# 7 passed; 1 failed; 0 ignored
cargo clippy -p tagrex-core --test move_context_recheck --all-features --no-deps --locked -- -D warnings
# passed
rustfmt --edition 2021 --check crates/core/tests/move_context_recheck.rs
# passed
```

The expected fixture-interruption panic messages are caught intentionally. The
remaining uncaught assertion is the preserved native-role regression. Owned
whitespace checks pass. No commit, PR, live profile, app or media was changed.

Reviewed source SHA-256:

- plan.rs: `9d9678451dd6303ec16185ccc57819b990b9de9c49c2a4456ee7ce5b4a9e737c`
- journal.rs: `7e02c37532994d11cea46a25186a681e77544f5167a33cebd138bbaba96d2fad`
- plan_paths.rs: `153c839b665791c5a80bb6d85aee7e9754f06e1f78a3596c6c8a585980f08027`

Tier 2 discovery used `tagrex-backend-readiness`, generation
2026-11-18T12:48:55Z. Coverage reports the production paths changed and new tests
untracked. Current source was read for the native codec, collection context,
SQLite loader, recovery ordering and mutation paths; this is not an exhaustive
architecture or path-authority audit. Native Windows/macOS behavior, actual power
loss, throughput optimization and the whole backend roadmap remain outside this
review's approval.
