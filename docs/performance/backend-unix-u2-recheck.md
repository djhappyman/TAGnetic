# Independent Unix U2 recovery recheck

**No new blocker found in the typed-identity correction on Linux.** The former
whole-collection role regression and the prior staging regressions pass unchanged.
This is a bounded review of the explicit Unix file/link/directory roles, U1
compatibility and refusal ordering. It does not approve the full recovery or
backend roadmap.

Only `crates/core/tests/unix_u2_recovery_recheck.rs` and this report were authored.
No production code, live app, profile, media, commits or PRs changed. Reusable
backend corrections belong in curated upstream `tagrex/tagrex:main`; fork storage
compatibility and upstream inline journal images remain separate requirements.

## Independent probes

Seven new test functions include one subprocess fixture entry point. Fixtures use
public Executor and SQLite APIs. The child persists a real move intent, captures
its entry, then exits without destructors. The parent opens the database afresh.
Each mixed fixture contains earlier completed transfers and a later pending
capture, plus an unrelated sibling.

- **39 audio-proof refusals:** every position in the three-row collection is
  exercised with a link/directory/wrong role, ambiguous U1 non-link, U1 link in
  a regular-audio intent, future version, truncated/extended/empty bytes, zero
  inode, unrepresentable birth time, and a syntactically valid Windows identity
  on Unix. Matching corruption is installed in both persisted proof collections,
  so refusing merely because two copies differ cannot satisfy the test.
- **22 directory-seal refusals:** regular-file/link/unknown roles, both U1 forms,
  zero inode and unrepresentable birth time are tested for completed and pending
  reservations. A genuine file identity relabeled as a directory is also checked
  against the existing pending directory.
- **Six additional refusal controls:** genuine directory proofs substituted for
  regular-file ownership at each position; ambiguous U1 non-link proofs with the
  entire stage collection removed. Legacy history remains readable.
- **Four positive controls:** U1 and U2 link entries after completed mixed-copy
  Apply and after interrupted move/link Apply. The link referent is removed
  before Undo. An independent hardlink edits the first moved audio object.
  Recovery restores the link entry and current same-object audio bytes, preserves
  the hardlink and sibling, removes the owned targets/reservations, and retires
  history only after success.

Every refusal compares the complete fixture filesystem tree outside SQLite:
relative names, device/inode, mode, file bytes and link text. It also compares the
full loaded batch and stage rows before/after, rather than only checking that a
batch still exists. Thus an earlier move, empty-directory cleanup or lost proof
would fail a refusal test.

A syntactically well-formed directory seal cannot be checked against an already
removed directory and is not itself authority for an absent entry. The native
seal-mismatch test therefore targets the occupied reservation; malformed roles
remain rejected even on completed rows. A first exploratory assertion demanded
refusal for the no-op absent-directory case and was corrected to this scope.

## Confirmed broader limitation: unfinished copy ownership

A separate normal probe hard-exits after a copy has completed but before a later
sidecar capture finishes the batch. The initial journal contains no durable hash
for that newly created copy. Undo restores the pending link capture, then refuses
with `Stale(target-2.wav)` rather than deleting an unproved copy. Both copy and
source bytes, all moved audio bytes, the referent, sibling and batch history are
preserved.

This is safe refusal, not complete automatic crash recovery. Durable copy outcome
recording before proceeding to later batch work remains a roadmap requirement.
Completed copies have positive coverage above. The initial combined success
expectation for unfinished copies was split into this explicit preservation
probe; the implementation was not changed or given ownership it cannot prove.

## Verification

```text
cargo test -p tagrex-core --test unix_u2_recovery_recheck --test move_context_recheck --test move_staging_adversarial --locked -- --nocapture
# 7 + 8 + 6 = 21 passed; 0 failed; 0 ignored
cargo clippy -p tagrex-core --test unix_u2_recovery_recheck --all-features --no-deps --locked -- -D warnings
# passed
rustfmt --edition 2021 --check crates/core/tests/unix_u2_recovery_recheck.rs
# passed
```

The review binds these production SHA-256 hashes:

- `plan.rs`: `2d2e9c0cd864fba115bb24a5ab79155cfa8b629f43136caa33e1f168b4201e2a`
- `journal.rs`: `e1326010c208ef8ebb9b3d255644412b112e87771c725716d4222cb42bd75331`
- `plan_paths.rs`: `153c839b665791c5a80bb6d85aee7e9754f06e1f78a3596c6c8a585980f08027`
- New test: `a634466cca7fd49ab53f58ee13fcba5d21a287ec747b43ff7ef89c648da9ae2a`

Tier 2 discovery used `tagrex-backend-readiness`, generation
`2026-11-18T12:48:55Z`. Native-identity searches had no indexed matches; a broader
Undo search found the older journal interface. Its trace had no edges, and its
snippet lines were visibly stale. Coverage marks all three production sources
changed and both prior test sources untracked. Current source was therefore read
for the native codec, intent compilation, whole-collection checks, directory and
entry capabilities, staging, recovery ordering and SQLite loader. No complete
architecture claim rests on the stale graph.

These fixtures test Linux process exit and native filesystem operations. Native
macOS/Windows, actual power failure or ENOSPC, network filesystems, arbitrary
concurrent adversarial writers and throughput optimization remain unverified by
this review. Fixtures clean themselves on success and assertion failure; compact
verification logs remain under the task-owned external `tmp` directory.
