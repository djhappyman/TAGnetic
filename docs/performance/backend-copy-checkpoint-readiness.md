# Completed-copy recovery checkpoint candidate

Scope: reusable standalone backend, curated to `tagrex/tagrex:main`; fork-only
artwork blob storage remains separate and upstream inline image/waveform contracts
remain unchanged. Production ownership is `crates/core/src/plan.rs` and `journal.rs`;
`plan_paths.rs` and its independently reviewed cleanup indexes are unchanged.
This is an implementation candidate awaiting a new independent review, not an
enterprise-wide readiness claim.

## Behavior and ordering

Previously a completed copy's strong evidence remained only in `Written` until
`finish` or best-effort error retention. A hard exit during later batch work left
an ordinary intact copy indistinguishable from an unknown partial output. Undo
preserved all data and history but refused automatic recovery.

`UndoJournal::checkpoint_copy` now commits one completed output immediately after
the exclusive copy, file flush, parent-directory flush and bounded streaming hash,
before its completion callback or any later step. The final copied-file metadata
unit (fields, covers, whole blocks) flushes and refreshes that checkpoint before
its completion callback. Copied sidecars receive the same evidence. The enclosing
batch remains interrupted until normal `finish`; no callback is emitted when the
checkpoint fails. No environment-controlled production fault hook was added.

Both journal implementations bind each outcome to exactly one copy destination
in the originally recorded plan, including copied sidecars. Outcomes require a
32-byte digest and a current-platform typed regular-file native identity with
birth generation. A refresh must name the same native object and match the exact
previous persisted checkpoint. Identical retries are idempotent. Unknown paths,
duplicate approvals/rows, weak/future/directory/link identities, move rows and
copy-removal intents cannot be overwritten. SQLite validates and writes in one
transaction. Two additive indexes bound destination lookup rather than rescanning
all prior files per checkpoint; bounded row/cell reads reject oversized evidence.
There is no journal schema-version bump or image representation change.

## Foreign journal compatibility and legacy ownership

`supports_copy_checkpoints` defaults to false and `checkpoint_copy` defaults to
refusal. An unsupported foreign adapter is rejected before recording a copying
batch or creating any output. Adapters that opt in must actually implement and
forward checkpoint durability; the default no-op `retain_interrupted` does not
serve as a checkpoint. `VecJournal` explicitly implements the same logical
contract but remains an in-memory journal, with no cross-process durability.
SQLite and the three existing copy-capable fault adapters explicitly opt in.
Other adapters preserve their existing behavior for plans without copies.

A distinct ownership defect was reproduced: an older strong-digest copy row with
`identity=NULL` allowed ordinary Undo to delete a byte-identical replacement inode
(with matching restored mtime), return success and discard history. Severity is
P1 for deleting a foreign entry, even though this reproducer preserves the same
content in a backup. With only the old permissive predicate restored, the actual
public Executor/SQLite test fails with `foreign ownership accepted, legacy=true`.
The corrected predicate requires typed native ownership as well as digest, size
and mtime. Historical missing/weak/ambiguous native identity remains readable,
but cannot authorize copy deletion in any Undo mode. Ordinary old metadata
history remains readable. Filesystems without a usable native birth generation
cannot successfully establish this stronger new copy contract; no content-only
fallback is provided.

Two existing negative legacy-copy fixture builders now explicitly remove the
completed-copy evidence after interruption. Their preservation assertions remain
unchanged: those probes still exercise genuinely missing older evidence rather
than asserting that the new executor must omit its checkpoint.

## Failure semantics and limits

An error before or after commit attempts rollback with the live strong proof;
uncertain successful commits cannot emit a false completion. If SQLite remains
locked/full and neither proof retention nor removal intent can commit, rollback
returns incomplete and preserves source, output and the original interrupted
record. Previous batches survive. The locked-database probe takes about 15 seconds
under existing SQLite busy handling across sequential attempts; cancellation is
not polled inside those synchronous calls. This scope does not change that
latency policy. Once storage recovers, a genuinely missing
proof remains unknown; observing current bytes alone does not confer ownership.

A process death during a copy, before its checkpoint, or during copied metadata
before its refresh still fails closed. It preserves partial/unknown output and
history. Completed outputs can recover automatically; arbitrary partial writes
are not falsely declared completed. A kernel file-size-limit test exercises a
real partial sidecar write/process death, separate from synthetic API errors.
The SQLite-full test uses the actual journal connection's pager page limit and
a pressure insert to produce real `SQLITE_FULL`; it does not exhaust a host disk.

This work retains native no-clobber copy removal, durable removal reservations,
whole move-stage collection validation, directory/native-role seals, callback
ancestry protection and strict Undo baselines. It does not claim a filesystem
transaction against an independent process editing the same held inode during
third-party metadata writing. Newly created directory cleanup still depends on
the batch's directory outcomes; a crash before final outcome persistence can
leave empty approved directories. Power-loss behavior across filesystems and
native Windows/macOS execution remain separate verification gates. Linux
process-death evidence does not substitute for those gates.

## Verification

Candidate run on disposable fixtures only, existing dev toolchain/cache/profile:

- Public integration suite: completed audio, copied sidecars, field/cover/block
  edits, mixed copy/move/link-entry hard exits; initial checkpoint before/after
  SQL, crash before metadata refresh, identical foreign replacement, same-inode
  external edit, cancellation, errors before/after commit, stale tokens and
  malformed/duplicate profiles, genuine locked SQLite and previous history.
- Same-connection SQLite-full unit: source/output/prior and interrupted history
  retained; no completion callback.
- Streaming 8 MiB sidecar: exact source/output bytes, reopen and safe cleanup.
- 256-folder persistent SQLite copy observation: Apply 482.83 ms, Undo 573.92 ms,
  process FDs 5 before / 8 peak; exact source metadata/audio and absence of copied
  targets/history checked. Tiny synthetic FLAC fixtures, warm machine and normal
  OS caches: this is a bounded observation, not a throughput SLA or cold-cache
  benchmark. Every copy performs its own durable checkpoint transaction.
- Independent raw FLAC/Vorbis readback verifies artist, unrelated Unicode title
  and the unchanged encoded frame; cover bytes are checked independently in the
  FLAC bytes. No result depends exclusively on the writer's own readback.

Full retained regression gate and final source hashes are recorded below after
its final run. New separate independent review remains required.

### Frozen candidate gate

Final retained source run: **442 passed, 0 failed, 5 existing ignored**. This includes
17 public copy-checkpoint tests, the same-connection SQLite-full unit, core unit
and Executor/SQLite suites, storage stress, and all retained authority, staging,
context/U2 and cleanup-index regression suites. No staging/index canary assertion
was weakened. The final default Clippy run passed without lint exemptions;
formatting and `git diff --check` passed.

```sh
source /mnt/Games/Development/tagrex/dev-env.sh
cargo test -p tagrex-core --lib --test executor --test sqlite_journal --test recovery_stress --test recovery_authority_final_review --test recovery_durable_recheck --test recovery_adversarial --test move_staging_adversarial --test move_context_recheck --test unix_u2_recovery_recheck --test move_cleanup_index_adversarial --test copy_checkpoint --quiet
cargo clippy -p tagrex-core --tests -- -D warnings
cargo fmt --check -p tagrex-core
git diff --check
cargo test -p tagrex-core --test copy_checkpoint many_folder -- --nocapture
```

Compact local evidence retained under `/mnt/Games/Development/tagrex/`:
`copy-checkpoint-final-gate.log`, `copy-checkpoint-clippy.log`,
`copy-checkpoint-scale.log`, and `copy-checkpoint-ownership-red.log`.
The red ownership reproduction restored only the old permissive matching
predicate for that run and immediately reinstated the corrected source.
Graph generation `tagrex-backend-readiness` / `2026-11-18T12:48:55Z` reports
changed production/tests and untracked new tests; current source reads were used
instead of relying on stale graph snippets.

SHA-256 at source freeze:

```text
f2d1f1f02b0564497dfb534b57f7f86a5ede5ddaf2c00f26146921a63bd5194d  crates/core/src/plan.rs
fdd30da46aabddc5fd908e678d28ae4342af614977a19c53285b76fd347c0a57  crates/core/src/journal.rs
bdd4d844b6c3f53a5638aa23c82eaa8f4e9648cd0a9f5ccc658f3956a6976544  crates/core/src/plan_paths.rs
2f2a7c9c61277cec739b899b1cffdd8dca7f1e4542e86c8bcc12c00d05dd4e65  crates/core/tests/copy_checkpoint.rs
2d05a3fb6d5bfa41e7b1f1696bde2cde4bea7b44214001cae1ed3d325d8253a8  crates/core/src/journal_copy_checkpoint_tests.rs
9ddfaccaa96629de564a3a26e5ed038b3421739b45e0ab924e2e052dc8d2a587  crates/core/tests/sqlite_journal.rs
4721df62cb24c6e0b6707f34dee005f3f65750e2f1de258794cf48db118641ef  crates/core/tests/recovery_stress.rs
8742d67ee34735ca3fbeb5d6b321be06afb0ef2cdcadab5513a07bb8e8967d30  crates/core/tests/unix_u2_recovery_recheck.rs
```

New independent review is pending. No commits, pushes, UI source edits, live
profile changes or real-media writes were made in this slice.


## Fresh independent review and orchestrator corrections

The [separate review](backend-copy-checkpoint-independent-review.md) is now
complete for the bounded Linux copy-checkpoint scope. It found a P2 beyond the
original author gate: duplicate copy-evidence rows could be silently collapsed
by Undo and cause refusal only after earlier copied outputs were removed.
SQLite reservation refusal also changed both duplicate rows, and VecJournal
accepted the ambiguity. All three independently retained regressions pass after
whole-batch duplicate validation before any recovery, transactional SQLite
reservation and matching in-memory uniqueness/idempotency checks.

Final independent retained gate: **437 passed, 0 failed, 5 existing ignored**,
with default core-test and scoped all-features Clippy and formatting passing.
This is a different bounded collection from the earlier 442-test author gate;
the counts are not additive. The reviewer report records the updated final
plan/journal hashes, original red evidence and all three Undo-mode controls.
The earlier frozen hashes above remain historical evidence. No remaining scoped
review defect was found; low-latency busy cancellation, native platforms,
power loss and protected exact-media integration remain separate gates.
