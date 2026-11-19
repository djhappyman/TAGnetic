# Independent recovery adversarial review

Date: 18 November 2026. Review baseline: `9da71249` plus the first uncommitted
recovery/resource/playback wave in `feat/native-field-foundations`. This reviewer
did not author or edit the production recovery implementation. Owned additions
are `crates/core/tests/recovery_adversarial.rs` and this report. Other agents'
source and test changes were preserved; no commit or publication was performed.

The change intends to preserve damaged-journal evidence, make completion retries
idempotent and expose sidecar cancellation boundaries. The unique recovery
namespace, explicit companion-move error handling, transactional outcome
replacement and real process-exit tests are useful improvements. Independent
negative testing nevertheless found four actionable gaps. **Verdict at this
review point: request changes; not ready to certify the recovery foundation.**

## Confirmed findings

### P1 — copy Undo can delete an unrelated replacement

Location: `crates/core/src/plan.rs`, `ensure_untransfer_safe` (reviewed lines
1065–1086), followed by `untransfer` (1365–1375); `CopyStamp` in
`crates/core/src/journal.rs`.

Reproduction: Apply a copy of a valid small PCM WAV, remove the copied target,
write another valid WAV with different sample bytes but the same length, and
restore the recorded modification time with `File::set_times`. Ordinary strict
Undo succeeds and deletes the replacement. Size and mtime are not ownership
proof, including on filesystems where users preserve timestamps deliberately.

`copy_undo_refuses_external_replacement_even_when_size_and_mtime_match` fails on
the reviewed implementation. New copies need persisted strong identity/content
evidence and an authoritative check before destructive removal. Missing or weak
historical evidence requires an explicit safe compatibility policy. A check only
at early preflight leaves a further replacement window before deletion.

### P1 — Undo move overwrites a file created after preflight

Location: `crates/core/src/plan.rs`, `Executor::undo_with_mode` transfer loop
(reviewed lines 643–654) and `untransfer` (1375).

Reproduction: Apply a pure WAV rename. During the public Undo progress callback
for the transfer, after preflight but before its mutation, create an unrelated
WAV at the original path. Undo calls ordinary `fs::rename(target, original)`,
overwrites the new file on Unix and reports success. This deterministic callback
models a concurrent external writer during the same validation-to-mutation gap.

`undo_move_never_overwrites_original_recreated_after_preflight` fails. The
mutation must enforce no-clobber semantics; another early existence check alone
does not close the race. The same boundary warrants inspection for sidecars,
Apply and rollback. The review did not reproduce every such operation variant.

### P2 — moving a relative main-database symlink breaks recovery evidence

Location: `crates/core/src/journal.rs`,
`quarantine_damaged_database_with` (reviewed lines 233–268).

Companion files are checked with `symlink_metadata`, but the main database is
not required to be a regular file. A journal path that is a relative symlink to
damaged data passes damage inspection. Moving that symlink into the newly
reserved recovery directory changes its parent, so its relative target no longer
resolves. A fresh database is created at the old alias and the returned recovery
path is broken. The actual damaged target remains elsewhere; this is broken
evidence location, not a demonstrated destruction of its bytes.

`damaged_main_database_symlink_is_refused_without_retargeting_recovery` fails.
Reject nonregular main paths before quarantine, validate identity, and refuse
replacement rather than move an unverified main entry into recovery.

### P2 — unknown future schema is accepted by an older writer

Location: `crates/core/src/journal.rs`, `SqliteJournal::open` (reviewed line 469
onwards), before migration statements.

A healthy current journal is marked `PRAGMA user_version=2000000000` and given
an additional future-native-identity table. `open_or_recover` accepts it instead
of refusing the unsupported version. No version gate protects the interpretation
of journal evidence. This is a compatibility readiness gap; this probe did not
demonstrate that an unknown payload was destroyed.

`future_journal_schema_is_refused_without_mutation` fails. Establish an explicit
supported journal schema version before applying migrations, preserve legacy
unversioned data deliberately, and make upgrades transactional so failed
migration attempts do not leave partial changes. Unsupported versions must fail
without quarantining or changing the original database.

## Negative conditions that passed

- A real held SQLite `BEGIN EXCLUSIVE` lock made opening fail without changing
  the healthy database bytes, directory names or saved batch. The call took
  approximately ten seconds because both opening and damage inspection waited
  for SQLite's busy timeout. This verifies preservation, not interactive latency.
- Actual Unix read denial (mode `000`, running as uid 1000) made opening fail
  without moving or replacing the journal. Restoring permissions allowed the
  original batch to be read. The test identifies privileged environments that
  bypass mode bits rather than describing those as permission-failure evidence.

The read-only `PRAGMA quick_check` damage classifier correctly avoided
quarantining the locked/permission-denied healthy journals in these cases. The
parent's initial concern was tested rather than reported as a confirmed bug.

## Commands and results

Using the existing environment and shared build cache:

```sh
source /mnt/Games/Development/tagrex/dev-env.sh
cargo test -p tagrex-core --test recovery_adversarial -- --nocapture
cargo test -p tagrex-core --test recovery_adversarial copy_undo_refuses_external_replacement -- --nocapture
cargo test -p tagrex-core --test recovery_adversarial undo_move_never_overwrites -- --nocapture
cargo clippy -p tagrex-core --test recovery_adversarial --all-features -- -D warnings
rustfmt --edition 2021 --check crates/core/tests/recovery_adversarial.rs
git diff --check
```

The initial four-test run produced two passes (lock and permissions) and two
expected review failures (future schema and main symlink). The later independent
copy and Undo-move canaries each failed with the preservation assertion above.
All six tests compiled. Targeted test Clippy passed after correcting an octal
permission literal; formatting and whitespace checks passed. These are failing
regression canaries requiring production fixes, not a passing acceptance suite.

Fixtures use task-owned temporary roots under the existing task `TMPDIR`, with
RAII cleanup even on assertion failure. The WAV envelope and sample bytes are
constructed directly by this independent test, so the two destructive scenarios
do not depend on a metadata writer reporting its own results. No real media,
runtime profile, existing Undo history or application process was touched.

## Review coverage and remaining evidence

Tier 2 graph discovery used project `tagrex-personal-app`, generation
`2026-11-13T16:10:57Z`, at the other personal checkout. Search for journal
open/finish/retain symbols returned all eight matches; a one-hop both-direction
trace of `open_or_recover` returned its caller and damage/open callees. Symbol
snippet bounds were stale. Exact-path coverage reported modified/missing source
for journal, plan, core manifest and crash tests, so this review read actual
branch source and diffs rather than relying on graph completeness. Newly added
adversarial tests are unindexed and were read directly.

The author's crash matrix was inspected, but this reviewer did not independently
repeat all thirty process exits or its FFprobe probe. Abrupt death within library
writes or quarantine moves, power failure, directory flush durability,
filesystem failures on network/removable volumes, Windows/macOS behaviour and
all possible source-replacement schedules remain unverified. The partial
quarantine sequence deserves a durable recovery protocol before claiming that
every crash preserves a coherent database/companion group.

The parent has received all four reproductions and owns the production fixes.
A fresh review of the resulting design and a combined gate are still required.
Reusable fixes are candidates for curated `tagrex/tagrex:main`; fork schema and
storage compatibility requires separate assessment, retaining upstream inline
journal-image compatibility. No merge, release or complete-backend claim follows
from this review.
