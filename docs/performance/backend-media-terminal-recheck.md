# Independent terminal-proof recheck

**Bounded approval of the small retirement-proof correction.** No new blocking
finding was reproduced in this review. This does not approve Executor integration,
the native metadata writer, the broader backend, Windows/macOS execution, or UI
exposure.

Reviewed 18 November 2026 in the existing dependent foundation checkout following
`9da71249`. Production source reviewed was `crates/core/src/media_mutation.rs`,
SHA-256 `19bcf3d824f08c87c950ef27ace84370780fa7df0458834645afa32a2d03d820`.
The reviewer changed only `crates/core/tests/media_terminal_recheck.rs` and this
report. Existing tests and production implementation were preserved. Shared core
corrections are eligible for curated upstream `tagrex/tagrex:main`; this review
does not publish or merge anything.

The prior [fresh witness review](backend-media-witness-recheck.md) found a P2:
cleanup could start after a callback corrupted its already-written root terminal
record. The correction establishes an immutable expected-content/native-object
proof before the first terminal sync callback, validates it after callbacks and
before cleanup, and releases that guard only after deliberately unlinking the
owned terminal. The old report records its earlier failing candidate; its seven
unchanged tests now pass against this corrected source.

## Fresh adversarial evidence

The new suite drives the real public `Store`, `Published::retire`, reopen and
`finish_retirement` interfaces. Fixtures contain arbitrary media bytes, because
this review concerns retirement ownership and evidence rather than container
editing semantics. It does not replicate private parsing or cleanup helpers.

- **175 terminal mutation cases:** discover a successful retirement's exposed
  event sequence, then alter the root terminal at every callback where it exists,
  for initial retirement and finishing an already-partial retirement. Mutations
  cover checksum damage, same-length damage with restored mtime, truncation,
  widened permissions, removed native marker, additional hardlink, and a replaced
  inode with identical content and a correctly constructed native marker. Each
  refusal stops at the reached callback, preserves the exact remaining entry
  names/bytes and changed terminal bytes, keeps the reservation, and leaves the
  published original unchanged. Mutations after a legitimate earlier unlink are
  compared with the evidence remaining at that boundary, not with evidence the
  implementation already legitimately retired.
- **39 callback-error/reopen cases:** inject an ordinary error at each exposed
  retirement callback on both paths. No further cleanup occurs after the error.
  Reopening reconstructs the remaining reservation and finishes cleanup, or
  recognizes that the terminal and entry were already deliberately removed.
- **7 panic/reopen cases:** unwind at terminal creation/sync, partial cleanup,
  the last terminal-bearing callback and final sync, for initial and resumed
  retirement. Duplicate boundaries are counted once. Remaining evidence is
  unchanged by destructors and a reopened store can reconcile the operation.
- **5 malformed-reopen cases:** damage the checksum, truncate, widen permissions,
  remove protection or add a hardlink to a persisted terminal after partial
  cleanup. `Store::open` refuses without deleting remaining entry evidence or
  altering media. This is conservative refusal, not repair of malformed data.
- **Confirmation controls:** a reopened partial retirement refuses a durable
  finish acknowledgement error; a successful acknowledgement that changes the
  existing terminal's bytes with restored mtime also cannot start the next unlink.
- **Admission controls:** exhausting the shared transient budget prevents cleanup
  before confirmation. Partial retirement keeps the sole configured entry slot;
  another edit is refused before reaching its editor. Confirmed successful
  retirement releases the slot and a new operation is admitted.

The injected panic messages in a `--nocapture` run are expected: those seven
unwinds are caught and assertions then verify restart behavior.

## Operational limits

After a final sync error following the owned terminal unlink, the open store
retains its conservative reservation even when no entry or terminal is visible.
The tested reconciliation route closes and reopens the store; it then discovers
no remaining operation. A caller must handle this uncertain result and must not
interpret the error as permission to recreate recovery evidence or edit media.

These checks use Linux with protection xattrs on the existing fixture filesystem.
They do not exercise actual kernel ENOSPC, a storage-controller fault, physical
power loss, or other operating systems. The source still performs terminal writes
and syncs through real filesystem calls, while the injected faults occur at its
public observer/admission boundaries. Earlier author SIGKILL and I/O-fault tests
are separate evidence, not newly claimed executions here. The prototype cannot
reconstruct recovery material deliberately destroyed by an external actor, nor
exclude every uncooperative write between a check and a syscall.

The [primitive's contract](backend-media-mutation-readiness.md), durable caller
acknowledgement, native writer preservation, platform adapters and full Executor
integration still apply. This report makes no whole-process memory, throughput,
automatic-repair or overall backend-readiness claim.

## Verification and cleanup

Executed with reused dependencies/toolchain/build cache and locked dependencies:

```text
cargo test -p tagrex-core --test media_terminal_recheck --locked -- --nocapture
cargo test -p tagrex-core --test media_terminal_recheck --test media_witness_alias_recheck --test media_mutation_adversarial --locked --quiet
cargo clippy -p tagrex-core --test media_terminal_recheck --all-features --locked --no-deps -- -D warnings
rustfmt --edition 2021 --check crates/core/tests/media_terminal_recheck.rs
git diff --check -- crates/core/tests/media_terminal_recheck.rs docs/performance/backend-media-terminal-recheck.md
```

The new suite passed **6 tests**; the combined independent suites passed
**18 tests** (6 new, 7 prior witness/alias, 5 prior image/alias). Scoped all-feature
Clippy, formatting and whitespace checks passed. Production hash was checked
before and after verification. All disposable fixture directories are owned by
guards and removed on success or unwind. No live media/profile, runtime state,
Undo history, production code, previous tests, commit or PR was changed.

Codebase-memory Tier 2 project `tagrex-backend-readiness`, generation
`2026-11-18T12:48:55Z`, was checked before structural discovery. Exact path coverage
reports the new module and tests untracked/missing, so this review read current
source directly, including terminal creation/binding, callback guards, ownership
markers, manifest parsing, inventory, reservation scan, retirement and resume.
No graph completeness claim is made.
