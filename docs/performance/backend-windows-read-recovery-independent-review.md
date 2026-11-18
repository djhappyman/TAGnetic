# Windows read/recovery and macOS compatibility review

Status: reciprocal static review accepted for CI publication; runtime verification
is pending hosted CI.
This report does not approve a merge or claim current-head Linux, Windows or
macOS execution. The user prohibited further local tests, builds and benchmarks
during this correction. Only source/log inspection and lightweight diff/hash
checks have run since that instruction.

## Scope, ownership and evidence

The candidate is the existing `native-flac-publication` checkout, based on
`b7e19f2a`. The reader worker owns `model/exact/raw.rs`, its new witness tests and
the final proof-format/policy guard in `model.rs`;
the Undo worker owns the optional-directory correction in `plan_paths.rs` and
its new probes. The reviewing worker authored the separate Darwin terminal-entry
hunk in that shared file and the macOS fixture corrections below. Root owns the
two journal busy-timeout changes, publication and CI. The Undo worker independently
reviewed the Darwin entry hunk and all eight fixture-file changes, including the
matching expiry-waiter setup correction. The reader worker also independently
accepted those fixture changes and the Darwin entry boundary by static review.
Authored changes are distinguished from independent review in this report.

`tagrex-personal-app` graph generation `2026-11-13T16:10:57Z` is ready but rooted
at the personal checkout. The exact publication source/test paths submitted
to coverage returned `outside_project` / freshness unavailable. Findings therefore
use the actual publication diff and source, not graph verification of these new
modules. The older parked-foundation graph also does not cover this checkout.

The retained hosted logs are `tmp/flac-ci-docs-windows-65c53ff.log`,
`tmp/flac-ci-windows-b7e19f2a.log` and `tmp/flac-ci-macos-b7e19f2a.log` under the
workspace parent. The first Windows run missed a named-reader final-callback
mutation; the second missed a held-reader mixed-snapshot mutation. Both failed
the original interrupted-pruning SQLite recovery canary. Neither assertion has
been relaxed. A historical Linux baseline, before the no-local-execution
instruction, passed eight held-reader independent tests and thirty SQLite tests;
it does not verify the present corrections or Windows behavior.

## Ranked findings and corrections

### P2: equal-stamp metadata writes can evade an exact read

The shared `Input` check already compares length, modification time and native
identity/change fields before and after the last callback. The actual hosted
failures show those checks did not suffice. Microsoft documents separate
[data-stream and metadata timestamps](https://learn.microsoft.com/windows/win32/api/winbase/ns-winbase-file_basic_info)
and [deferred last-write updates](https://learn.microsoft.com/en-us/windows/win32/api/fileapi/nf-fileapi-writefile).
This supports a supplemental content proof; it does not establish the precise
timestamp collision mechanism of either hosted run.

The candidate retains those checks and replays bounded observed metadata spans
through the held source before returning. It retains offsets/lengths and hashes,
not another metadata image. Replay uses an 8 KiB buffer, has no public callbacks,
and is bracketed by the original source/name checks. The ceilings are
1 MiB of span bookkeeping and 64 MiB of observed bytes per proof under the
existing 64 MiB operation admission. Outermost MP4 container payloads are proved
once, avoiding multiplication by nesting depth; parser observations remain
included. Large repeated top-level metadata may conservatively return `Limit`.
Initial observation plus the three publication checks perform at most four
bounded hash passes: at most 256 MiB of observed/proof IO per exact request,
usually much less for small metadata. Fixed-size MPEG trailer-marker probes can
touch an untagged tail, but audio extents are not scanned or hashed wholesale.
The bounded final replay is not asynchronously
cancellable and is not a filesystem snapshot or whole-file integrity claim.

Independent source review identified an additional required coverage gap:
MPEG trailing ID3v1/APE metadata can feed the returned legacy view while the
initial proposed witness covered only the leading header/ID3v2 block. Exact
snapshots with an unavailable selected layout still expose that legacy view.
The corrected candidate adds bounded ID3v1 and APE tail/leading-tag proofs,
deduplicates an already observed leading APE extent, and refuses unproved Lyrics3
suffix layouts. A parsed-format/layout check prevents another container being
accepted with an MPEG-only proof. A final policy snapshot comparison follows the
last replay as well as preceding it. Deterministically refreshing the private
saved stamp after an actual byte edit is collision emulation, not a Windows
runtime reproduction.

### P2: an admitted missing parent loses authority under another spelling

Windows entry IO probes a canonical parent after the original raw parent was
admitted as absent. Previously only the raw spelling was optional; validation of
the newly recorded canonical alias returned `NotFound` during otherwise authorized
recovery. The proposed fix keys absence authority by resolved target. Alias/root
binding is checked first, then the no-follow walk verifies every existing prefix
before permitting absence. Every successfully opened prefix consumes that target's
absence allowance; owned retirement removes only the retired subtree.

The review challenged redirected aliases, replaced or removed existing ancestors,
unplanned missing parents, consumption after deep creation and adjacent component
names. It also rejected a full optional-set scan per retired directory as a
potential quadratic regression. The final candidate uses a `BTreeSet` and ordered
component-prefix subtree removal. Five new pure private-Scope probe functions
cover the mechanism; they have not been executed on this candidate. A proposed
sixth synthetic public Undo probe was dropped because its hand-built history
preconditions were not independently established. This also removes its invalid
`crate::journal`/`crate::plan` imports when the capabilities source is included by
the existing integration target. The original
process-death SQLite canary retains file, sidecar, removed-directory and history
assertions. No broader permission to recreate missing ancestors is introduced.

### P2: Darwin refuses terminal link entries with the former Unix flags

Linux uses `O_PATH | O_NOFOLLOW` to inspect a link object. The former Darwin
fallback used ordinary read plus `O_NOFOLLOW`, which returns `ELOOP` for that
object. The authored correction uses `O_SYMLINK` only on Apple targets. Apple
documents that it [opens the link itself](https://developer.apple.com/de/documentation/system/filedescriptor/openoptions/symlink);
the locked rustix 1.1.5 source exposes that flag on Apple. The descriptor still
comes from the pinned no-follow parent walk. Regular-file/link type gates and
recorded inode comparisons remain intact; other Unix and Linux flags are unchanged.
The Undo worker independently accepted this source-level boundary, without a
compile/runtime claim.

## All thirteen macOS failures

| Hosted failure | Classification and candidate correction |
| --- | --- |
| Commands `each_files_sidecars_come_from_its_own_folder` | Terminal link `ELOOP`; Darwin entry correction. |
| Core `copy_removal_crash_intent_move_verify_delete_is_retryable` | Private synthetic receipt/root setup used a raw temporary-directory alias; canonicalize the newly created fixture root before deriving paths. |
| Core `real_sqlite_full_during_stage_intent_does_not_move_any_payload` | Fixture exceeded Darwin pathname limits before reaching SQLite. Initialize an empty 512-byte-page database with `VACUUM`, verify the page size, and use a shorter nested path. Keep the real pager cap, `SQLITE_FULL`, no-transfer and retained-history assertions. |
| `copy_checkpoint::mixed_copy_move_and_link_entry_recover_after_process_exit` | Child hits terminal link `ELOOP` before expected exit 81; preserve the exact exit and recovery assertions. |
| `exact_source_authority_fresh_review::non_utf8_logical_names_and_repeated_nested_same_source_reads_do_not_share_cursor_state` | APFS rejects fixture creation with `EILSEQ`. Always exercise the invalid logical name against a valid held backing, including all repeated/nested reads and budget checks. Also retain the original named-file case when creation succeeds; only Apple `EILSEQ` is recognized as an impossible setup. |
| `move_cleanup_index_adversarial::failed_directory_removal_keeps_native_parent_authority` | Direct private Scope root was raw; canonicalize only that fixture constructor. Retain failed-rmdir, foreign-parent and file-inode assertions. |
| `move_cleanup_index_adversarial::successful_retirement_forgets_all_aliases_but_keeps_adjacent_authority` | Same direct-Scope setup; retain alias retirement, adjacent authority and foreign redirect checks. |
| `recovery_adversarial::exclusive_sqlite_lock_never_quarantines_healthy_history` | Default SQLite waits exceeded the existing 15-second check. Root sets explicit 500 ms per-call busy waits on normal open and read-only damage classification; no test timeout increase. |
| `recovery_stress::non_unicode_paths_are_refused_before_any_journal_or_media_mutation` | APFS cannot create the invalid-name source/sidecar/root fixtures. Still submit all four invalid logical plan variants and require the Unicode refusal before effects, checking valid media sentinels, any representable native entry, targets and empty history. Recognize only Apple `EILSEQ` during setup. |
| `sqlite_journal::legacy_interrupted_moves_without_identity_refuse_automatic_recovery` | Source inference: the manually seeded receipt stores raw roots, unlike public Apply's canonical persisted roots. Correct only that receipt's roots; keep raw public Undo input and the original exact `Stale`, payload and history assertions. The hosted log did not print the unexpected returned error. |
| `unix_u2_recovery_recheck::explicit_u1_and_u2_link_entries_survive_missing_referent_and_mixed_copy_restart` | Terminal link `ELOOP`; Darwin entry correction, retaining explicit link-entry/missing-referent semantics. |
| `unix_u2_recovery_recheck::interrupted_unfinished_copy_refuses_deletion_and_keeps_all_payloads_and_history` | Child hits terminal link `ELOOP` before expected exit 86; retain exact exit and every payload/history assertion. |
| Service `history_request_waiting_for_app_cannot_mutate_after_disable` | Media, App history and Redo checks passed; the final durable receipt byte check failed. In-memory initial success precedes its asynchronous receipt flush. Wait for that initial durable terminal receipt before capturing the unchanged-byte baseline; preserve the real waiting-request barrier and exact post-denial receipt comparison. |

Apple explicitly states that [APFS creation requires valid UTF-8 names](https://developer-mdn.apple.com/library/archive/documentation/FileManagement/Conceptual/APFS_Guide/FAQ/FAQ.html).
The fixture adaptations exercise the same public refusal/held-name contracts;
they do not skip macOS cases or accept unexpected production errors. Initial
in-memory success preceding persistence is an observed ordering, not a newly
established durability guarantee. A broader service crash-contract audit remains
part of the resumed backend programme.

The reciprocal fixture review caught an initial authored APFS probe mistake:
the independent public reader still used the unrepresentable logical label.
It now reads the valid backing path, while every explicit held exact-read request
keeps the invalid logical label. The native named-file case remains unchanged
when the filesystem supports it. The same confirmed durable-baseline race was
corrected in `lifecycle_history_independent_tests.rs`; its original three-second
setup deadline, grant expiry, waiting-lane deadline and unchanged-byte assertions
remain intact.

## Independent review of root's busy-wait change

Both explicit waits are installed immediately after opening a SQLite connection.
Failure to configure the read-only classifier returns `false`; busy and read
errors still do not classify history as damaged. Only a non-`ok` quick-check or
SQLite corruption/not-a-database codes permit the existing quarantine route.
The normal-open wait propagates its original error. This changes contention
latency and can return `Busy` sooner; it is a per-SQLite-call wait budget, not a
hard total-operation wall-clock SLA. No automatic mutation replay is added.

## Completion gate and upstream route

Nine new witness probe functions and five new private-Scope probe functions have
been source-reviewed, not run. Static review accepts the changed boundaries for
hosted verification, with no remaining scoped source finding identified. A
lightweight `git diff --check` passed. This is not a compiler, formatter, test or
benchmark result.

The candidate needs hosted Linux
`check`, Windows, macOS and applicable audit results on the exact pushed head.
Existing historical Linux greens and source inspection do not replace that gate.
No local Rust test/build/Clippy/formatter/benchmark has run for these corrections.
There is no Windows native-media capability, power-loss, RSS, whole-backend or
UI-readiness approval here.

Shared backend corrections are eligible for curated `tagrex/tagrex:main`; the
route remains waiting for the compatible native-field/text-codec foundation.
Upstream inline journal images and the stable waveform API must be preserved.
The broader backend programme has resumed, but these foundations must satisfy
their gates before further feature work builds on them.

## Frozen source evidence

SHA-256 of the reviewed source candidate (publication commit still pending):

| File | SHA-256 |
| --- | --- |
| `crates/core/src/model.rs` | `2e0242dae069f3ceccfc2756ef56d4d4418b6f3813beac75e42bb8ee215eaab3` |
| `crates/core/src/model/exact/raw.rs` | `f605ae37a86897d66c3ce8984c5ce7ab72e920b39e76636e6f19edde03763017` |
| `crates/core/src/model/exact/raw_witness_tests.rs` | `01975ea90d4607f113d98465cf4f7ba68d17a58289a19b7c61a2869ef91d9353` |
| `crates/core/src/plan_paths.rs` | `dfbaa2a7bd3a651100d62a579961fd7ca302e0c02267cf94373481144f8dd94d` |
| `crates/core/src/plan_optional_parent_tests.rs` | `6eeff4ee6324d753762e6d406049236f88e6b6111d5806067988578f48aeae99` |
| `crates/core/src/journal.rs` | `c38e47c5c1ad1fce19ba4a9b1f43ea707a3af6d45ae72f0a653c99232d521253` |
| `crates/core/src/plan_copy_removal_tests.rs` | `7a63e27ec3be452305bbdca333963108920ef41e2ba8cf9278d97886558be248` |
| `crates/core/src/plan_move_staging_tests.rs` | `f12645a861bb746cc66ddb718c6754b3d500bd03dde17ef886c550542d54b972` |
| `crates/core/tests/exact_source_authority_fresh_review.rs` | `3ea3655f73ae3664cb04bbb7390f745b3a3b357815fa6f34342df9679569a05f` |
| `crates/core/tests/move_cleanup_index_adversarial.rs` | `5cf1825776d401373200b402aeb5125129bd7887d1470ef8c0ef3c89bd9f2608` |
| `crates/core/tests/recovery_stress.rs` | `0c8ac427e09ed0e056817fd9fde1b23b22d067e624e0e4c3876eb106918d781c` |
| `crates/core/tests/sqlite_journal.rs` | `d895ff3910d2015e8c31de6f0c504abfd89f37a8e8b5522696c1bb7911ebeb20` |
| `crates/service/src/handle_lifecycle_tests.rs` | `9ebd46f3b648e46c408a05fc50529b877370d860c986516efc2cbdd707ab9a76` |
| `crates/service/src/lifecycle_history_independent_tests.rs` | `3cc88b574585347510e9df1b24a417ddf4d923d6359cadee0b4fa6ec88e21b2b` |
