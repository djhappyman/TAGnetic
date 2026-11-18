# Independent service lifecycle and history recheck

**No new blocker found in the reviewed Linux correction.** The P1 revoked-history
regression and P2 private-handle lifecycle corrections pass their original tests
and this separate review's public-service probes. This is bounded evidence, not
approval of the full service, backend roadmap or platform matrix.

Only two new test files, this report and a `#[cfg(test)]` module registration in
`service/src/lib.rs` were authored. No production behavior changed. Reusable
backend corrections belong in curated upstream `tagrex/tagrex:main`; fork storage
compatibility and upstream inline journal images remain separate requirements.
No live app, profile, user media, commit or PR was changed.

## Independent conditions

`service/tests/service_lifecycle_history_adversarial.rs` contains four tests:

- A real five-file write is accepted, reaches preflight and waits for the held
  App mutex. Revocation, disable, actual grant expiry and explicit operation
  cancellation are exercised separately. After releasing the mutex, every file
  and the native history remain exactly unchanged; the operation is terminal
  without success. This challenges the asynchronous worker's final admission,
  not only the synchronous outer response check.
- Twenty provider responses use 190,000 NUL characters in a title. JSON escaping
  exceeds the wire limit although the string is much smaller. Each refuses
  before retaining a receipt. Seven earlier image handles remain releasable; exactly
  the remaining 249 slots can still be admitted, and releasing them permits a
  full 256-image batch again.
- Four rounds start eight differently sized provider image batches concurrently.
  Whole successful batches fit the shared limit; refused batches leave no partial
  owners. The exact remaining capacity is fillable. All returned handles can be
  released and the full capacity reused after every round. These varied batches
  challenge aggregate admission without assuming a scheduler-specific winner.
- A filesystem fault prevents initial operation persistence after an earlier
  successful write. No unpublished queue owner remains. After repair, eight
  simultaneous retries with the same key all name one actual operation, create
  one history entry, and preserve the earlier operation receipt unchanged. The
  durable operation file contains exactly those two operations.

`service/src/lifecycle_history_independent_tests.rs` adds one test with two
history directions. Four Undo requests and four Redo requests start together
with the App mutex held. An observational `try_lock` proves at least one request
in each direction passed initial authorization and reached the real history lane
before expiry; it does not assert an unobserved ordering for every sibling.
There is no injected production fault hook. The grant actually expires before the App
mutex is released. Every request refuses. All three fixture files (including an
unaffected sibling), complete native history, Redo count/revision/token and
`operations.json` remain byte-exact. Expiry is checked at final admission, rather
than inferred from a response returned after media changes.

The fixtures own their WAV bytes, journal, service directory and paths. Full-byte
comparisons establish refusal preservation, including encoded audio, without
relying on the production tag reader. The successful retry is an operation and
idempotency control; it does not certify new container writer semantics.

## Verification

```text
cargo test -p tagrex-service --test service_lifecycle_history_adversarial --locked -- --nocapture
# 4 passed; 0 failed
cargo test -p tagrex-service --lib --locked lifecycle_history_independent_tests:: -- --nocapture
# 1 passed; 0 failed (8 actual expiry waiters across Undo/Redo)
cargo test -p tagrex-service --all-features --locked
# 154 unit + 11 author lifecycle + 4 independent lifecycle
# + 7 snapshot independent + 3 snapshot policy/resource = 179 passed; 0 failed
cargo clippy -p tagrex-service --lib --test service_lifecycle_history_adversarial --all-features --no-deps --locked -- -D warnings
# passed
rustfmt --edition 2021 --config skip_children=true --check <owned Rust paths>
# passed
git diff --check -- crates/service/src/lib.rs
# passed
```

Compact logs remain under the task-owned external `tmp` directory:
`service-lifecycle-independent-integration.log`,
`service-lifecycle-independent-history.log`,
`service-lifecycle-independent-combined.log`, and
`service-lifecycle-independent-clippy.log`. The owned fixture prefixes have no
remaining directories after the verified runs.

Source SHA-256:

- `service/src/lib.rs`: `dcebf163d9a4aa090438143e2735fed856e5c8c9acd55ff7c01776bf46581147`
  (author behavior unchanged; the new review module registration changes hash).
- `service/src/domains.rs`: `c43392248ba25f9e6cf093972bc61f6a474a6963560ab3b9d1280777bb356d94`
- `service/src/plans.rs`: `c75ac7f67467f3d1106e10ad33c1d51e1ca899abd4ec47ab279dc07641cd54f4`
- `service/src/json_accounting.rs`: `015f1fce06428342f0faaa1d13288bc12247c5b14275e4a73e1b82b42d1a250e`
- `service/tests/service_lifecycle_history_adversarial.rs`: `7cb426afd838d5067a16c2e1c7d2f89994d805728c40e03d0e59b80b588611cc`
- `service/src/lifecycle_history_independent_tests.rs`: `4100fb5364698cd43780789660bc503d152ebdd00ae7d5b732cfb30b9d585f20`

## Discovery and limits

Verify-tier graph discovery used `tagrex-backend-readiness`, generation
`2026-11-18T12:48:55Z`, rooted in this checkout. The bounded history/grant/provider
search returned all 24 matches. A both-direction history trace was paged to
completion (79 outgoing and two incoming rows); some resolver edges point to
unrelated UI names, so they are discovery hints rather than source proof. The
history outline also uses older lines. Coverage marks every material production
path changed and new tests untracked. Current source was read for authorization,
receipt construction/admission, query publication, operation acceptance/execution,
persistence and Undo/Redo ordering. No negative architecture claim rests on the
graph's stale generation.

This review does not prove interruption of a history action already admitted and
running while holding the state lock. Expiry during such a long-running action,
analysis `SCANS` owner lifecycle, general DTO/JSON/RSS ownership, arbitrary
external filesystem races, actual ENOSPC/power loss and native Windows/macOS
execution remain outside its approval. The filesystem fault is an owned
operations-file path made unavailable; it is not a power-failure simulation.
Full backend and exact-writer readiness remain separate gates.
