# Recovery authority recheck: changes required

18 November 2026. Independent review of the current working tree above
`9da71249`, limited to Executor directory/file authority and move ownership.
The review owns only `core/tests/recovery_authority_final_review.rs` and this
report. It did not edit production code, journal schema, real media, live app
state or runtime profiles. Fixtures are generated PCM WAV files inside owned
short-lived temporary directories; destructors remove them after each probe,
including assertion failures.

## Ranked findings

### P1: rollback and reopened Undo claim an unrelated move target

Still reproduced in two normal, unignored tests:

- `reopened_move_undo_does_not_claim_a_replacement_as_its_own_transfer` applies
  a pure move, closes the journal, retains the moved file under another fixture
  name, creates a different WAV at the target, then reopens the journal. Ordinary
  Undo returns success and moves the unrelated replacement to the original path.
  It removes the batch instead of retaining an unresolved recovery entry.
- `failed_apply_rollback_does_not_relocate_a_later_move_target_replacement`
  replaces the first completed move target at its completion callback, then
  creates a competing second destination. Apply fails at that collision, but its
  rollback moves the unrelated first replacement to the original path. It returns
  the ordinary collision rather than retaining the unproved transfer for recovery.

The approved file's exact bytes remain in a separate fixture backup in both
probes. This is therefore a change of file identity, not an in-place text edit.
The destination collision fixture is also preserved. Per-step capabilities only
prove the source observed at that step; they cannot prove that a reopened Undo
or later rollback still owns a previous move. Pure moves currently have no
persisted ownership outcome comparable to copied-file evidence.

Required correction: capture ownership of successful media and sidecar moves
before callbacks, carry it into rollback and persistent recovery, and refuse to
relocate an unproved replacement while retaining journal evidence. Historical
records without sufficient evidence need an explicit compatibility/refusal
policy. Do not infer ownership merely from the current pathname or accept a
replacement because the current step happened to open it.

### P1: cross-directory rename skipped the held source check — corrected locally

Initially reproduced twice: cross-directory Apply move and reopened-journal
Undo move accepted a regular source replaced after `Step::enter` and returned
success. Same-directory moves and cross-directory copies correctly refused.

Unix `Authority::rename` returned from its parent-identity comparison before
running the regular-file and held-source identity checks. The parent corrected
this small production finding during review: those checks now run before the
same-directory case-only-name branch. Windows gained a corresponding held leaf
check in source; native behavior is not verified here.

The four original unchanged canaries now pass:

- cross-directory Apply move replacement refusal;
- same-directory Apply move refusal (control);
- cross-directory copy refusal (control);
- cross-directory Undo move refusal with retained history.

This correction does not resolve the preceding durable/rollback ownership gap.
It also does not provide an atomic file-identity compare-and-swap against a
separate process changing a leaf between the final check and the kernel rename.

## Verification and scope limits

The final run of
`cargo test -p tagrex-core --test recovery_authority_final_review -- --nocapture`
produced **four passed, two failed, zero ignored**. Both failures are the retained
move-ownership regressions described above. The initial run before the parent's
small correction produced two passes and two failures for the callback cases.
No assertion was weakened to accommodate production behavior.

Owned Rust formatting and whitespace checks passed. Scoped all-feature
denied-warning Clippy (`--test recovery_authority_final_review --no-deps`) passed.
Its first attempt was briefly blocked by a concurrent exact-read visibility
mismatch; that source owner corrected the mismatch, and the reviewer reran the
check successfully without editing other workers' source.

Discovery used Tier 2 graph `tagrex-backend-readiness`, indexed at
`2026-11-18T12:48:55Z`, rooted in this checkout. Coverage reported metadata changes
for `plan_paths.rs`, `plan.rs` and `recovery_stress.rs`; current relevant source
was read directly. `journal.rs` and the earlier durable review tests matched the
index. The graph was used for bounded discovery and the `Scope::enter` callers,
not as proof of the subsequently changed implementation.

This review does not repeat or certify the author's 5,000-folder timing probe,
all file formats, physical power loss, active in-place writes inside a metadata
library, network/removable filesystem semantics, or Windows/macOS runtime
behavior. It does not approve the entire backend or declare merge readiness.
Shared standalone correctness changes are eligible for curated upstream
`tagrex/tagrex:main`; incompatible persistent fork storage changes need a
separate compatibility classification. No commit or PR was created.
