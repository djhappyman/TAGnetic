# Independent service snapshot ownership recheck

**Scoped correction passes after one P2 finding.** This review owns only
`crates/service/tests/snapshot_ownership_adversarial_review.rs` and this report.
It used public Service calls, paired principals, actual App sessions and disposable
FLAC fixtures. Production was changed only by the parent after the finding.
Shared backend corrections are candidates for curated `tagrex/tagrex:main`;
fork storage compatibility and upstream inline journal images remain separate.

## P2 found and corrected: a revoked request retains an unreachable snapshot

A warm snapshot build can pass authorization and acquire the snapshot-build lock,
then wait on the desktop App mutex. Revoking that principal removes its currently
owned handles. On resumption, the old implementation inserted its new snapshot
without rechecking authorization under the insertion lock. The outer call refused
the result correctly, but the revoked principal could never release its new owner.

The retained canary uses no production hook or timing sleep: it holds the shared
App mutex and starts two same-principal snapshot calls together. A `busy` response
from one proves the other owns the build while blocked. It then revokes the
principal and releases the App mutex. Before correction, both callers returned
errors but retained charge rose from 892,002 to 893,410 bytes. The unchanged
assertion that usage return to the warm baseline failed.

The parent added `grant_in_state` under the same final State lock as snapshot
insertion. The unchanged race canary now passes. This verifies snapshot ownership
at this revocation boundary; it is not an audit of every other handle constructor.

## Independent passing conditions

Seven new tests now pass, including:

- A sparse scope over two nonadjacent groups, with a hidden group between them.
  All 34 selected titles are queried independently; each native query result
  maps to the same saved DTO path. Releasing the full and sparse handles and
  cancelling one lane does not invalidate another principal's query state.
  Another principal's snapshot handle cannot be paged.
- Fifty repeated resource refusals after the scoped-view reservation can fit but
  the native client index cannot. Usage returns to the exact pressure baseline
  after every failure. Earlier handles remain queryable and retry succeeds once
  pressure is released.
- Eight clients creating, querying and releasing four snapshots each concurrently.
  Read-only build contention is retried with a bounded deadline; each independent
  client sees its own 12 approved rows.
- A cold source file changed to an outside symlink does not return freshly read
  outside metadata. A cached scoped parent moved outside its grant and replaced
  with a symlink produces an empty approved snapshot.
- Three Unicode/punctuation separator changes across full and scoped clients.
  Historical pages keep their saved interpretation, old native query indexes
  refuse as `cancelled`, and new DTO/native rows use the new policy consistently.
- The deterministic revocation race described above, including exact charged
  owner release rather than merely expecting an error response.

The original three public-service resource/policy regressions remain unchanged
and pass in the combined run.

## Exact local verification

```text
cargo test -p tagrex-service --test snapshot_ownership_adversarial_review --test snapshot_resource_policy_recheck --all-features --locked -- --nocapture
7 + 3 passed; 0 failed; 0 ignored
cargo clippy -p tagrex-service --test snapshot_ownership_adversarial_review --all-features --no-deps --locked -- -D warnings
passed
rustfmt --edition 2021 --check crates/service/tests/snapshot_ownership_adversarial_review.rs
passed
```

Owned whitespace checks pass. Fixture directories removed themselves on both
passing and failing runs; no task-owned fixture directories remain. Compact logs
are in external `tmp/snapshot-ownership-independent-first.log`,
`tmp/snapshot-ownership-independent-final.log` and
`tmp/snapshot-ownership-independent-clippy.log`.
The first log preserves the actual pre-correction failure. No commit, PR, app
restart, live profile or user-media mutation was performed.

Reviewed source SHA-256:

- service/lib.rs: `5c7ef5275415a4cc49ab0cdb939532c83ed434217dc19995fda65f62f4e3b04c`
- service/saved_rows.rs: `4337ebd8f0441be97c4ec7ccd7d0af87a32672613d1149a1c325092915bece15`
- commands/query.rs: `47da14c6f9ca0389bba5e4758a84b6f0ca804482286ed1e50f645118c1a0478c`
- commands/lib.rs: `864522ce3b7179eda07a213da783c17ec3a7bcfefe164a386a99cc74e0a0657b`
- service/domains.rs: `b6768f9f11284b9dda98257ae63f42c53abf80b82ceb854a95fd307a7d4e585c`
- service/outputs.rs: `ac44ecbbc2931457ccdcf6fed6431e406d7c1d2e34a2e8c33e0a29350b8a8923`
- independent test: `3d806de5444c3cd687f07c73e6560f3b51cd0a2c6a6fe6ef908273dda2f95297`

## Limits

This is Linux local evidence. The two Unix symlink probes are conditional;
native Windows/macOS execution, expiry timing, physical memory/RSS guarantees and
all other handle-creation races remain unverified. Original DTO/JSON/IPC owners
are not all admitted by this change. The test suite does not certify the entire
backend or a fixed maximum production library size.

Scoped views currently reserve position capacity for every source row even when
only a small subset is approved. This is bounded and accounted; proportional
sparse storage is a possible later efficiency improvement, not a reproduced
accounting failure in this correction.

Graph discovery used Verify tier project `tagrex-backend-readiness`, generation
`2026-11-18T12:48:55Z`. Snapshot/get_snapshot/query/query_snapshot_id searches and
both-direction snapshot tracing were complete for their requested result pages.
Coverage marked changed production files and untracked new files; current source
was read for all material claims. The graph's old `from_tracks` call relationship
is stale and was not treated as evidence of current construction.
