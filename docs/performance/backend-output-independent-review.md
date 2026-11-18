# Independent confined-output publisher review

The bounded in-process publisher correction passes this independent Linux
review. Two confirmed findings have retained normal regressions; the parent
corrected production code and all seven new tests pass. This is a separate
review from its implementation, with the limitations below. Shared backend fixes belong on curated upstream
`tagrex/tagrex:main`; fork storage and upstream inline journal images remain
separate concerns. No commit, PR, desktop, profile or user media changed.

## P2: replaced logical root could still report successful publication

`moved_library_root_is_not_reported_as_success_at_its_old_logical_path` moves the
library directory after staging and creates a plain replacement at its old
public name. The original publisher returns `Ok(())` while the report exists in
the moved-away directory and is absent from the requested location. The foreign
replacement contents survive; this finding does not claim a foreign overwrite.

`Directory::under_root` returned the pinned root directly for an empty relative
path, and child traversal also started from that same root. This compared the
held object to itself rather than verifying the current public root identity.
The parent now reopens and compares that public root to the held native object
before traversal. Additional independent controls replace the root at
`BeforeCommit`, with both root-level and nested outputs: refusal restores the
old report in the pinned original tree and preserves the replacement tree.

## P3: partial cleanup returned deleted private-directory pseudo paths

With two output parents, fail the second `BeforePrivateRemove` after the first
private directory was successfully removed. Both new reports remain committed,
but recovery paths included the first removed directory, surfaced on Linux as
`.tagrex-output-empty-<nonce> (deleted)`. That is an unusable location for a user
following the recovery error. The retained normal probe requires inspectable
paths and a manifest in each remaining private directory. A pathname string
suffix must not decide ownership, since ordinary real names can contain it.
The parent now records successful private-directory retirement explicitly, skips
retired owners when writing evidence/rescue copies, and omits definitely absent
paths while keeping inaccessible paths for inspection. The unchanged diagnostic
probe passes after that correction.

## Independent passing probes

- An 18-case failure matrix spans first/second staging, sync observation,
  candidate capture, publication, backup capture, commit, rollback capture and
  restoration, and postcommit cleanup. Before commit both original payloads
  remain accessible somewhere in the fixture. After commit both new outputs
  remain. Recovery errors retain at least one inspectable path. Budget and
  descriptor reservations return to their prior values after dropping the error.
- Forty rounds of public API calls run two concurrent batches in opposite
  destination orders. Each round ends with one coherent committed pair or no
  pair, without mixed or partial public contents.
- A later symlink or duplicate hard-linked original refuses the entire batch
  before creating the earlier fresh output or artifacts. Untargeted original
  bytes and the link objects remain unchanged.
- Distinct non-UTF-8 output names and a nested Unicode/quoted path publish
  independently, without lossy-name collisions or leftover staging directories.

The matrix injects ordinary I/O errors at existing fixture-only observation
points. It does not reproduce real kernel ENOSPC or power loss. The author's
separate RLIMIT short-write and hard-exit probes remain separate evidence.

## Verification state

```text
cargo test -p tagrex-commands --lib output::independent_review_tests --test output_publisher_adversarial_review --all-features --locked -- --nocapture
# 4 independent unit tests passed; the shared filter selected 0 integration tests
cargo test -p tagrex-commands --test output_publisher_adversarial_review --all-features --locked -- --nocapture
# separately ran all 3 integration tests: passed; 0 failed
cargo clippy -p tagrex-commands --lib --tests --all-features --no-deps --locked -- -D warnings
# passed
rustfmt --edition 2021 --check --config skip_children=true crates/commands/src/output.rs crates/commands/src/output/independent_review_tests.rs crates/commands/tests/output_publisher_adversarial_review.rs
# passed
git diff --check -- crates/commands/src/output.rs crates/commands/src/output/ownership.rs
# passed
```

The original root-staleness and deleted-directory probes were observed failing
before their respective production corrections. Their assertions are unchanged.
No task-owned fixture directories remain after these runs. Reviewed SHA-256:

- output.rs: `65caf14b4b41b32386efabae3ceff88573fadf64ee11c53ee0b6b836554dce4a`
- output/ownership.rs: `7a01d9014d030ce7325833aa68f5f9770eef9a1332c8aa4e4bcd16e2517712eb`
- output/independent_review_tests.rs: `46a53a0afcd14cc84a6cf4e9fd2816bbbe8da0642f242ceb68ccca1e9e0592d1`
- tests/output_publisher_adversarial_review.rs: `062320ce421176e7ba61fc3a8b2265f51d775ee5afc1ede70cc753ee3b212153`

Tier 2 discovery used `tagrex-backend-readiness`, generation
`2026-11-18T12:48:55Z`, with the correct foundation root. Search and both-direction
trace found the public command and service export callers; the graph snippet was
stale. Coverage marked production paths changed and the new modules untracked,
so current source was read for Directory, Proof, Private, Entry, staging,
rollback, cleanup, resource admission and public/service error boundaries.
This is a bounded review, not an exhaustive call-graph claim.

Native Windows/macOS rename/sharing behavior, durable restart discovery/replay,
append, actual power loss, service-level injected recovery-error delivery, and
the entire backend roadmap remain unverified. The public command wrapper and
service error mapping were read; this review does not claim an end-to-end service
fault test.
The private namespace is not a security boundary against another privileged or
same-account actor arbitrarily destroying or replacing all copies. Existing
held-proof checks and private captures reduce ordinary race risk; finite tests
are not proof of immunity to every filesystem interleaving.
