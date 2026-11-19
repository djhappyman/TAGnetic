# Native FLAC admission — independent review

Status: scoped admission review accepted on the frozen publication candidate.
This is Linux-only component/integration evidence, not a whole-backend, release,
RSS or cross-platform native-write readiness claim.

## Scope and method

Reviewed the private compound IO phase in `plan/native_flac.rs`, the two public
Executor reentry guards, read-only Budget ceilings, the held-file Store adapter's
initial admission retry, and the native writer's unchanged working charge.
Read the participating exact-reader, codec and journal admission lifetimes.
The graph project `tagrex-backend-readiness`, generation
`2026-11-18T12:48:55Z`, points at the separate parked source checkout. Its symbol
search did not cover the new admission symbols; exact-path coverage for all
publication files used here reported outside-project/unavailable. Conclusions
therefore use current publication source reads and actual probes.

The IO gate limits private Apply/Undo/retirement phases to two at default
ceilings, derived from existing job and byte limits. It preserves each Store,
reader, writer and journal lease. It does not guarantee admission against an
unrelated owner exhausting the process budget. Waiting is bounded; Apply observes
cancellation. Public progress callbacks execute outside this private phase.
The guard is thread-bound and releases its atomic count/TLS marker on unwind.
Only the private held-file adapter waits for its initial Store admission, before
creating an entry or invoking an editor. Public Store primitives still refuse
immediately. No mutation or SQL acknowledgement is replayed by these retries.

## Ranked findings

**P2, corrected and independently rechecked:** a private Prepared-state journal
sink could reenter Executor, issue public progress callbacks and create peer
intent history before the nested IO gate refused. The initial independent canary
observed two callbacks. The author was already preparing early reentry guards;
the final guards refuse both Apply and Undo at their public Executor entry,
before callbacks or history mutation. The same canary now passes for both paths.
No further scoped admission defect was found.

Retaining interrupted history under persistent external resource pressure is an
explicit safe refusal, not successful rollback. The independent probe forces
that case after the first file was written, checks the untouched second file,
releases the external owner, and verifies complete later Undo and history removal.

## Verification

`native_admission_independent.rs` contains one controlled sequence rather than
parallel tests that accidentally compete with each other's deliberate exhaustion.
It changes no production serialization or existing test scheduling. It checks:

- A real public Store-held working job plus seven externally held jobs; native
  capture cancels without changing source or recording history.
- All jobs briefly occupied at the write callback; private Store admission waits
  before effects, then Apply and exact Undo succeed after release.
- Persistent job exhaustion after the first owned file; explicit failure,
  interrupted history retained, later exact recovery after release.
- Public Store refusal before its editor under full job pressure.
- Private Prepared-sink Apply and Undo reentry: no progress callbacks, no changed
  peer bytes and no changed peer history.
- Panic in that sink, followed by successful fresh Apply/Undo on the same thread.
- Nested peer Apply/Undo from a public progress callback remains usable.
- Original complete fixture bytes restored and all Budget owners released after
  their sessions and Stores are dropped.

The fixtures contain repeated/literal/empty/non-ASCII values, an unrelated
comment, an application block and a synthetic encoded audio payload. Assertions
compare complete original bytes after Undo and use an independent raw comment
reader for the edited title. No live library/profile/app writes occurred.

Fresh final commands (all exit 0):

```sh
cargo test -p tagrex-core --all-features --test native_admission_independent --test native_flac_executor --test media_held_file --test media_mutation
cargo clippy -p tagrex-core --all-targets --all-features -- -D warnings
cargo fmt --all --check
```

54 outer tests passed, zero failed, three existing optional/manual tests ignored.
Nested process-helper output is not added to that count. The independent sequence
passed in 1.40 seconds. Evidence logs live under the task's retained
`/mnt/Games/Development/tagrex/tmp/native-admission-independent-*` prefix; the
initial red reentry evidence is `native-admission-independent-first.log`.

## Frozen source

| File | SHA-256 |
| --- | --- |
| `crates/core/src/plan.rs` | `930c9843ee0148e70f56122549cc228333785c616ac62c57e937149b2c36a2f1` |
| `crates/core/src/plan/native_flac.rs` | `9b099cff59c406d4c80606e33e0ce48fb8c9df711ca527cb2b7d9d7757a11c63` |
| `crates/core/src/media_mutation.rs` | `a649306f23bc8a4c254ccbb32cbe5b020b1ef6879ea86a77f906acbd640d64ef` |
| `crates/core/src/resources.rs` | `a416ae31c50d45ec3aeaee114dea0b00a48aa04827ec0a35e9ec05ca1df0c4e2` |
| `crates/core/src/model/native_patch.rs` | `9f64ce7ca94d083cae86760e8d5148fb2ff8f9386b2839bdb35d50595582c348` |
| `crates/core/tests/native_admission_independent.rs` | `b7d554214ee93d0e7c7de90170000582a441f5978650bfefee47e100b98d0fcd` |

Production hashes match the implementation author's admission freeze. Separate policy and journal reviews,
combined downstream gates and exact-head Linux/Windows/macOS/advisory CI remain
the orchestrator's publication requirements. No commit, PR or merge was made by
this reviewer. Eligible upstream route is `tagrex/tagrex:main`, waiting for the
compatible native-field/text-codec foundation; upstream compatibility differs.
