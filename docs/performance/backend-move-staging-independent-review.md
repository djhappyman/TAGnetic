# Independent durable move-staging review

**Changes required.** Scope: the new private move staging protocol, public
Executor recovery, reopened SQLite rows and cleanup. No production files changed.
Owned deliverables are `core/tests/move_staging_adversarial.rs` and this report.
Reusable correction route: curated upstream `tagrex/tagrex:main`; fork journal
storage compatibility and upstream inline images remain separate requirements.

## P2 — persisted stage context is not validated before recovery mutations

`SqliteJournal::move_stages` decodes stored rows without binding their endpoints,
private path or identity to the immutable batch plan and its typed move proof.
`Executor::undo_with_mode` calls `recover_move_stages` before validating the
ordinary plan. That helper restores a stage entry to `stage.from`, including
foreign material, before checking native ownership.

A one-cell change to `move_stages.source` redirects either approved media or a
foreign fixture payload to a vacant, never-planned name inside the approved
root. Undo then reports an error, but the unintended move has already happened.
Malformed stage identity bytes likewise still allow a payload restoration
before refusal. These are recovery-metadata corruption probes, not a claim that
a normal application callback alone edits SQLite or that data was deleted.

The collection probe adds a second independently produced native stage row to
the first batch, whose immutable plan does not contain that row's target. It
also changes that row's source endpoint. The current implementation reports
successful Undo and drops history, after recovering the earlier valid stage and
moving the later payload to the unplanned name. This demonstrates that checking
one row immediately before its move would be insufficient: validate the entire
collection before the first recovery mutation.

Retained, normal, unchanged failing tests:

- `damaged_stage_source_cannot_redirect_foreign_material_to_an_unplanned_name`
- `damaged_stage_source_cannot_redirect_owned_material_to_an_unplanned_name`
- `malformed_staged_identity_refuses_before_restoring_any_payload`
- `complete_stage_collection_is_validated_before_recovering_an_earlier_valid_row`

Recommended correction: validate every loaded row against one batch move intent
and its exact persisted native identity; accept only its original-to-target or
reverse endpoints; validate the private entry/directory relationship and strict
identity encoding; reject duplicate/conflicting/unbounded recovery sets before
filesystem effects. Apply the same validation to rollback and foreign journal
implementations, not only SQLite insertion. Refusal must retain every payload,
existing path and journal entry. Keep corruption distinct from permission to
repair or move unknown material.

## Passing controls and evidence

Two actual public-Executor/SQLite controls pass: an owned pending stage reopens
and restores its original while preserving an unrelated sibling directory; a
foreign pending payload whose original endpoint is occupied remains private,
with the original and history unchanged. Interruption uses a fixture journal
adapter after durable intent; it does not install a production fault hook.
Native identities come from actual Executor requests, not a reimplementation of
private encoding.

Fresh run:

```
cargo test -p tagrex-core --test move_staging_adversarial --locked -- --nocapture
# 2 passed; 4 failed; 0 ignored
cargo clippy -p tagrex-core --test move_staging_adversarial --all-features --no-deps --locked -- -D warnings
# passed
```

Owned rustfmt and whitespace checks pass. Fixtures clean themselves on success
or assertion failure. No live app, real media, profile, commits or PRs changed.

Reviewed source SHA-256:

- plan.rs: `cade8dcf7dfa8e81146373ea19ec84901f618c37abe7a4ebcb224a7f25087ec0`
- journal.rs: `350d6c23ee1c06afe150f96e7b5f25f538589bcc010ec26b4ac7f91e98b28a7d`
- plan_paths.rs: `153c839b665791c5a80bb6d85aee7e9754f06e1f78a3596c6c8a585980f08027`

Tier 2 discovery used graph `tagrex-backend-readiness`, generation
2026-11-18T12:48:55Z. Exact coverage reports the three production files changed
and new staging tests untracked; current source was used for the relied-on
recovery, loading, scope and mutation paths. No exhaustive graph claim is made.

Native Windows/macOS behavior, true power loss, full writer preservation and
throughput optimization remain outside this approval. The known 5,000-file
staging cost is not waived. This finite review is not whole-backend certification.
