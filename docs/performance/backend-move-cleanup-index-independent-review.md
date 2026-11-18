# Independent indexed move-cleanup recheck

**No new confirmed blocker found in the bounded Unix cleanup-index correction on
Linux.** This reviews exact subtree retirement, retained authority and preserved
recovery work. It does not approve the entire recovery/backend roadmap, native
platform matrix or throughput target. Only the new test and this report were
authored; production content hashes match the author's frozen checkpoint.

Reusable backend route: curated `tagrex/tagrex:main`. Fork storage compatibility,
upstream inline journal images and the waveform interface remain separate and
preserved. No live app, profile, media, commit or PR was changed.

## Independent conditions

Six new normal probes in `move_cleanup_index_adversarial.rs` pass:

- Failed nonempty-directory removal retains the native directory proof. Replacing
  that ancestor afterward is refused even when an independent hardlink preserves
  the media inode exactly.
- Successful retirement through one symlink alias forgets all recorded aliases
  of that owned subtree, allowing its legitimate new generation to be learned.
  Adjacent component names and an unrelated ancestor retain their authority;
  redirecting the latter to the same file inode still refuses.
- Linux native byte names containing invalid UTF-8, spaces and adjacent prefixes
  preserve each other's directory proofs. This exercises the internal capability
  module, not a claim that Executor accepts non-UTF-8 journal paths.
- A real public Executor first completes a transfer and retires its private
  reservation. A later ancestor is then redirected through a symlink to the
  same file inode. The next transfer refuses, foreign/current bytes remain intact
  and the earlier payload remains at exactly one ordinary endpoint.
- A journal adapter inserts an unrelated sibling into a real persisted private
  reservation. Failed Apply and reopened Undo preserve that sibling, media names,
  native identities, bytes, the complete loaded batch and stage rows. After
  removing only the fixture-owned blocker, ordinary Undo retries successfully and
  retires history.
- Cancellation after 31 completed transfers across tricky/Unicode parent names
  restores all 64 original payloads, removes owned targets and retires history.

The private capability checks import the **actual source module interface** into
the integration binary. No production visibility or fault hook was added. This
also compiles and runs the source module's two existing author invariant tests;
they are counted separately from the six new tests. Those check ordered prefix
ranges against a full component scan, index/map equality and retained fallback-pin
accounting. The independent tests exercise filesystem effects and authority,
rather than replacing these checks with expected implementation fields.

## Timing and unchanged work

An additional manual probe independently uses 512 nested PCM WAV files under
prefix/Unicode folder names. It closes/reopens SQLite between Apply and Undo,
counts exactly **512 ordinary intent transactions in each phase**, verifies every
payload/name/history/output-directory result, and preserves a same-object edit
made through an outside hardlink. Two consecutive full cycles pass:

| Cycle | Apply | Undo |
| --- | ---: | ---: |
| First | 1.707 s | 1.638 s |
| Repeat | 1.698 s | 1.619 s |

A fresh rerun of the existing public 1,000-file persistent-journal profile passes:

| Layout/cache label | Apply | Undo |
| --- | ---: | ---: |
| Flat/data-advised | 3.194 s | 3.016 s |
| Flat/warm-repeat | 3.124 s | 3.077 s |
| Fan-out/data-advised | 3.184 s | 3.081 s |
| Fan-out/warm-repeat | 3.146 s | 3.076 s |

Every phase still records 1,000 intent calls; intent SQL takes 0.805–0.827 s.
All cells observe four extra callback descriptors, restore original bytes, remove
owned destinations/output directories and retire history. These fresh results
match the author's corrected measurements in
[the profile checkpoint](backend-staging-cleanup-profile.md). Current source
confirms the ordinary intent transaction, no-follow/native identity checks,
private-directory grammar and directory sync calls remain. Test-only metrics
remain behind `cfg(test)`; no release timing collector was introduced.

The author's older-source baseline and 5,000-file before/after figures were
inspected, not independently rerun on the former source. Accordingly this review
confirms the current work/preservation and reproduces the corrected range; it
does **not** claim an independent precise percentage speedup. Advisory fixture
data eviction is not controlled OS metadata coldness. Concurrent machine work,
filesystem caches, hardware and network filesystems remain uncontrolled. No
machine-wide cache action, tool installation or durability reduction was used.

## Verification

```text
cargo test -p tagrex-core --test move_cleanup_index_adversarial --test move_context_recheck --test move_staging_adversarial --test unix_u2_recovery_recheck --locked -- --nocapture
8 + 8 + 6 + 7 = 29 passed; 0 failed; 1 manual test ignored
# first eight include six fresh probes plus two inherited author invariants
cargo test -p tagrex-core --test move_cleanup_index_adversarial --locked paired_prefix_layout_reopened_undo_preserves_all_work_and_aliases -- --ignored --exact --nocapture
1 passed; 0 failed
cargo test -p tagrex-core --test move_staging_profile --locked -- --ignored --nocapture
1 passed; 0 failed
cargo clippy -p tagrex-core --test move_cleanup_index_adversarial --all-features --no-deps --locked -- -D warnings
passed
rustfmt --edition 2021 --config skip_children=true --check crates/core/tests/move_cleanup_index_adversarial.rs
passed
```

Owned whitespace checks pass. Compact logs are retained in the task's external
`tmp/move-cleanup-index-independent*.log` and
`tmp/move-cleanup-index-public-profile.log`; fixtures clean themselves on success
and assertion failure. Existing U2/context/staging canaries were unchanged.

## Bound source snapshot and limits

| File | SHA-256 |
| --- | --- |
| `core/src/plan.rs` | `aaf989f50917f937ca6aaacc9a51838a88d7ce1db7bffd49e0db8266a3791516` |
| `core/src/journal.rs` | `41e78d4a7b8a322e1eb549687b9c6035bc61d008c0bb4334586c269c2c943056` |
| `core/src/plan_paths.rs` | `bdd4d844b6c3f53a5638aa23c82eaa8f4e9648cd0a9f5ccc658f3956a6976544` |
| `core/src/plan_move_metrics.rs` | `cc76d9a567297aa455f6dbcba59b3ef1970e6f3373facd402959099a1dc6dc65` |
| New test | `4932bd66d7ea5b9fe9108c58e735be1320d84ec0841df98579354a1c214bae08` |

Verify-tier discovery used `tagrex-backend-readiness`, generation
`2026-11-18T12:48:55Z`, rooted in this checkout. `Authority` search returned the
older structure at lines 70–81 and its trace had no graph edges. Coverage marks
plan/journal/path source changed and metrics/profile/test sources untracked.
Current direct source was therefore read for every material retirement, alias,
walk, validation, staging, SQL and timing claim. No completeness or absence claim
rests on the stale graph.

Secondary indexes add O(path-count) metadata under existing directory/alias caps;
complete shared-budget/RSS accounting is not certified here. Native filesystems
without birth generations, Windows/macOS runtime behavior, physical power loss,
ENOSPC and network filesystems were not verified by these probes. The retained
unfinished-copy crash limitation still safely refuses automatic cleanup without
durable ownership proof; the old tests continue to cover it. Exact writer/
journal integration and wider backend readiness remain open.
