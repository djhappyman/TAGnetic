# Independent process-death harness review

Status: checkpoint ready for integration. No additional scoped defect found in
the frozen harness changes. All retained targets pass on Linux.

## Scope and intent

This review is limited to the interruption fixtures in
`crates/core/tests/sqlite_journal.rs`, `move_context_recheck.rs` and
`move_staging_adversarial.rs`. The reviewer changes only this report. The shared
policy/rollback production correction is unchanged from its preceding independent
review.

Surviving a Rust panic now invokes automatic rollback. Catching that unwind
therefore no longer models a process dying with durable interrupted history.
The fixture conversion must terminate an actual child without unwinding, then
reopen its real SQLite journal and retain the existing restart, foreign-object,
whole-collection and path-authority assertions.

The nested Executor context test has a different purpose. It must remain a
same-thread caught unwind, prove automatic cleanup of the inner operation and
then prove the outer operation's context and directory authority were restored.
Replacing that nested call with process death would remove the behavior it tests.

The graph project `tagrex-backend-readiness`, generation
`2026-11-18T12:48:55Z`, indexes a different checkout. Exact coverage checks for all
three publication test files return `outside_project` with unavailable freshness.
Review uses their actual source/diffs, including the original safety assertions.

## Review checks

- The SQL seed is a separate disposable database and uses the existing journal
  codec for complete field, cover and block plans. The Apply-only
  `prune_empty_dirs` flag travels explicitly because it is not journal plan data.
  Source, execution history and seed database are not interchangeable.
- Child dispatch names and fixture-root arguments must route only to the
  requesting parent's disposable fixture. Exact status 73 distinguishes the
  intended interruption from a panic, an ordinary exit or a missed boundary.
- A 30-second child deadline must kill and reap a stuck child before failure.
  Output must not deadlock a filled pipe; owned logs stay inside the fixture.
- Journal capability and media selection must preserve the intended route:
  legacy finish wrappers do not advertise native capability; the SQL bare FLAC
  fixtures have no selected Vorbis block, and move fixtures use WAV/transfer
  plans. Native recovery has its own retained test coverage.
- All original recovery assertions must remain, including refusal before an
  earlier valid row moves, preservation of foreign/edited media and siblings,
  retained unresolved history and cleanup of only owned empty directories.
- No new ignore or platform exclusion may turn a failing safety assertion into a
  pass. Child dispatcher tests that return immediately during a normal run are
  counted transparently as dispatchers, not additional restart scenarios.

## Verification and limits

Final frozen test-source SHA-256 values, verified before and after execution:

| File | SHA-256 |
| --- | --- |
| `crates/core/tests/sqlite_journal.rs` | `7a158d21bee53a414fa4e4de5aebd535ed12407f3df9ca316d6903f344165cb3` |
| `crates/core/tests/move_context_recheck.rs` | `5c39623ad782c41ebbd83575e2e284883b38cf81d5435010a5a0a2d9b2956ab0` |
| `crates/core/tests/move_staging_adversarial.rs` | `3d27ee066c05ce5400f390efc4aa8ade970899d0cac4c6274e048b73fc5602db` |

The four production policy/plan hashes match the accepted values in the
[portable policy review](backend-portable-policy-independent-review.md), including
`plan.rs` `daeb4b0505a3b0123247dc4b67f6b8042d811d8de2c8a4e9be828424201061b8`
and `plan/native_flac.rs`
`cba3c092206ce2da824f09fd942496047e2dcf536ecc4f857a536800ae9d73fe`.
No production code changed for this harness adaptation.

Using `source /mnt/Games/Development/tagrex/dev-env.sh`:

| Command/check | Result |
| --- | --- |
| `cargo test -p tagrex-core --test sqlite_journal --test move_context_recheck --test move_staging_adversarial` | 30 + 9 + 7 passed; 0 failed/ignored |
| `cargo clippy -p tagrex-core --test sqlite_journal --test move_context_recheck --test move_staging_adversarial -- -D warnings` | Passed |
| `rustfmt --check --edition 2021` on those three files | Passed |
| `git diff --check` on those three files | Passed |

The raw total is **46**: **43 existing behavior tests plus three dispatcher
tests**. Each dispatcher returns immediately without its parent's environment;
that pass alone is not restart evidence. The parent behavior tests require exact
exit 73 and then assert actual reopened journal/media state. Child invocations
are not counted again. Targets ran with default test concurrency, taking 0.86,
0.15 and 0.05 seconds respectively on this run.

Logs are
`/mnt/Games/Development/tagrex/tmp/flac-process-death-independent-final-tests.log`
and `flac-process-death-independent-final-clippy.log` in the same directory.

The final source/diff review confirms all original foreign-object and
whole-collection assertions remain. The nested context case still catches an
unwind on the same thread, now requiring immediate inner journal/media cleanup
before the original outer recovery assertions run. No test ignore, platform
gate or safety assertion was weakened. Child roots use bare PathBuf values;
only the parent owns fixture destruction. All three helpers use a 30-second
deadline, kill and wait on timeout/wait error, exact exit-code checking and
fixture-local files instead of bounded stdout pipes. Parent log handles close
before deletion, including the explicitly dropped SQL Command builder.

Every file mutation belongs to disposable fixtures. No live library, application
profile, installed runtime, portable build or user Undo history is used. The
broader backend roadmap remains paused after the authorized FLAC slice.
Final inventory found no remaining SQL, move-context or move-staging fixture
directories from the test runs.

An exit without Rust unwinding proves recovery after actual OS process death.
It does not prove behavior after machine power loss, storage-controller cache
loss, arbitrary hardware corruption or on platforms not executed here. Workspace
and exact-head hosted results remain separate integration gates. The watchdog
timeout/kill/reap branch was source-reviewed; this review did not inject a stuck
child to execute that branch. These shared backend test changes follow the
upstream curation route in `docs/WORKFLOW.md` with their compatible foundation.
