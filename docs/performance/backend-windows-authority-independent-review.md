# Independent Windows authority fixture review

Status: the two fixture-constructor corrections are accepted for integration.
No remaining scoped issue found. This is not a new production or Windows runtime
acceptance claim; exact-head hosted Windows verification remains required.

## Ranked diagnosis

**P2: internal fixtures bypassed public root admission without satisfying its
canonical-root precondition.** Windows job `135128333274` on `f663bd6` failed 17
move-staging tests at the shared initial `CopyStamp::for_move` call, before their
safety bodies ran. Evidence is retained in
`/mnt/Games/Development/tagrex/tmp/flac-ci-windows-f663bd6.log`.

The private `pinned_paths::Scope::enter` stores supplied roots. Its resolved-path
check canonicalizes paths and performs component-prefix containment against those
roots. The fixtures supplied raw temporary paths. Rust's
[canonicalize documentation](https://doc.rust-lang.org/std/fs/fn.canonicalize.html)
states that Windows results use extended-length syntax. Raw and canonical
representations can therefore disagree before any intended move race is reached.

The public Executor already runs `canonical_roots` before creating its Apply and
Undo scopes. The bounded call-site inventory found direct internal fixture
callers in move staging and descriptor I/O; the remaining unqualified Scope
fixture is inside the Unix-only cleanup-index test module. This evidence supports
a fixture precondition correction, not weakening production root handling.

Both constructors now canonicalize their newly created, owned directory before
deriving any file, journal or child-process path. Production containment, reparse
checks, source identity checks and sharing flags are unchanged. No original test
assertion, ignore or platform gate changed, and no extra test file was necessary.

The author's controlled Linux symlink-TMPDIR probe demonstrates the same private
caller precondition class: the old fixture failed during no-follow ancestry
walking, and the corrected fixture ran its safety body. Its Linux `NotADirectory`
error is not the Windows prefix-mismatch error, nor Windows execution evidence.
The separate retained logs are `flac-windows-fixture-path-red.log` and
`flac-windows-fixture-path-green.log` in the task log directory.

## Review of the previously hidden Windows bodies

All 17 bodies were read beyond setup. Microsoft's
[CreateFileW sharing documentation](https://learn.microsoft.com/en-us/windows/win32/api/fileapi/nf-fileapi-createfilew)
confirms the delete-sharing semantics relevant to rename. Transport entry handles
use read/write/delete sharing; ancestor and metadata handles deliberately deny
replacement. The distinctions below preserve the original ownership outcomes.

| Original failing test | Static body and handle-lifetime check |
| --- | --- |
| `source_swap_after_final_native_stat_is_restored_without_false_ownership` | Transport file handles share delete access, so the injected source rename remains a valid attack. Original owned bytes, captured foreign bytes and blocked restoration assertions remain. |
| `destination_race_cannot_clobber_and_owned_private_payload_can_recover` | Creating a new colliding child name is independent of deleting its held parent. No-clobber refusal, owned restoration and foreign-target preservation remain. |
| `blocked_publish_restoration_retains_owned_and_both_foreign_entries` | Both colliding names are created while the owned payload is staged. All three payload and retained-history assertions remain. |
| `private_source_swap_after_publish_stat_cannot_be_reported_as_success` | The private file's transport handle permits the rename; the sealed directory remains discoverable. Preserved owned bytes, foreign publication and recovery refusal remain. |
| `replaced_private_directory_after_restart_is_refused_and_retry_is_safe` | The interrupted scope unwinds and the journal is dropped before directory replacement. The refusing Undo drops its scope before fixture repair/retry. Both directory seals and all preservation assertions remain. |
| `process_death_at_each_move_phase_leaves_recoverable_original_object` | Child roots inherit the canonical fixture spelling. Exact exit 83, all seven phase boundaries in both directions, bounded child wait and restart assertions remain. |
| `process_death_with_an_unverified_foreign_capture_keeps_every_payload_and_history` | File transport permits the injected child rename. The child still exits at the staged boundary, and original/foreign payload plus retained-history checks remain. |
| `actual_crash_with_completed_and_pending_stages_validates_even_completed_context_before_recovery` | Both recorded intents and the child use the same root representation. Corrupt completed context must still prevent earlier recovery effects; valid context must restore both files. |
| `real_sqlite_full_during_stage_intent_does_not_move_any_payload` | Deep paths derive from the canonical root, retaining Windows long-path syntax before directory creation. Real pager-full failure and no-effect assertions remain. |
| `a_foreign_journal_without_explicit_staging_support_refuses_before_payload_changes` | Unsupported staging still refuses before an attempted move, with no private reservation or payload mutation. |
| `duplicate_and_malformed_later_stage_rows_refuse_before_cleanup_or_restore` | Paused scopes release their handles before fixture mutation. Whole-collection validation must still preserve earlier reservations and every later malformed/foreign payload. |
| `matching_but_malformed_persisted_proof_and_stage_identity_never_authorize_recovery` | Windows-specific proof mutations and type/birth checks remain. Matching corrupted database cells cannot gain authority. |
| `foreign_journal_duplicate_collection_is_rejected_before_any_filesystem_effect` | Duplicate rows remain observable through the foreign adapter, and both source absence and owned staged bytes are still asserted. |
| `invalid_stored_stage_context_stops_rollback_before_reversing_any_completed_move` | The completed move and later damaged context remain distinct. Rollback must still report failure and preserve the completed target/history. |
| `stale_empty_intents_over_retry_budget_refuse_without_cleaning_any_reservation` | All 65 reservations remain under the same canonical root; recovery refusal still preserves every directory and history. |
| `new_intents_are_admitted_against_the_same_retry_budget_used_for_recovery` | Refusing the 65th intent still leaves exactly 64 reservations and no leaked extra directory. |
| `oversized_stored_stage_cells_are_refused_before_loading_or_recovering_payloads` | Oversized SQL source/identity cells still refuse without moving staged media or dropping history. |

Directory cleanup was also traced: the inner capture/publication Step drops
before cleanup, and Windows removal retires matching directory-map handles
before removing an empty private directory. The fixture correction does not
change those lifetimes or replace safety assertions with accepted failures.
These are source conclusions; Windows hosted results must still execute them.

## Verification

The graph project `tagrex-backend-readiness`, generation
`2026-11-18T12:48:55Z`, indexes the parked source checkout. Exact publication paths
report `outside_project`; review uses actual source and diffs. Only this report
was changed by the independent reviewer.

Frozen SHA-256 values, rechecked after verification:

| File | SHA-256 |
| --- | --- |
| `crates/core/src/plan_move_staging_tests.rs` | `67c9c447ace50f8635af43cf9257fb71ee49a8d2ecc2d6c906b2228fc02a78c7` |
| `crates/core/src/plan_descriptor_io_tests.rs` | `be9dd8c011a425bcb5667da835a48ecee746771b8272b1d30c88795d724346ae` |
| Unchanged `crates/core/src/plan_paths.rs` | `d6e0de8cfd1da5a86c707767f30f239785d55cf1ec423069c60337b886618473` |
| Unchanged `crates/core/src/plan.rs` | `8cc196f1202bceb2564cce0000da5cae06de8f102617defe2ede8cdfe0eca5be` |
| Unchanged `crates/core/src/journal.rs` | `c29348b001a1b80ba4809a5307da8f36cff012cfcb48a3f78aebb63d0c206fce` |

Using `source /mnt/Games/Development/tagrex/dev-env.sh` on Linux:

| Check | Result |
| --- | --- |
| `cargo test -p tagrex-core --lib plan::move_staging_tests` | 26 passed, 0 failed; one existing manual profile ignored |
| `cargo test -p tagrex-core --lib plan::descriptor_io_tests` | 4 passed, 0 failed/ignored |
| `cargo clippy -p tagrex-core --lib --all-features -- -D warnings` | Passed |
| `rustfmt --check --edition 2021` on both changed fixtures | Passed |
| `git diff --check` on both changed fixtures | Passed |

The raw 30 passes include two existing child-dispatch entries; their no-op parent
passes alone are not crash-recovery evidence. All original phase/recovery parent
tests still check their child exit and filesystem outcomes. Logs are
`flac-windows-authority-independent-{move,descriptor,clippy}.log` under
`/mnt/Games/Development/tagrex/tmp`.

No live media, profile, application runtime or user Undo history was touched. No
new production acceptance, Windows red-to-green execution, power-loss guarantee or
broader roadmap completion is claimed. Exact pushed-head Windows, macOS, Linux
and applicable audit checks must pass before merge. Shared backend eligibility
remains `tagrex/tagrex:main`, waiting on the compatible foundation while retaining
upstream inline images and its stable waveform API. Broader implementation stays
paused after the authorized FLAC slice.
