# Native FLAC journal: independent review

Independent review complete for this bounded Linux durable-journal scope. Four
P2 state/recognition/completion/collection defects were reproduced, corrected by
the parent/implementation worker and rechecked with the retained probes. No
remaining scoped defect was found. This is not all-platform, all-format or
whole-backend approval.

## Scope and method

Reviewed the durable native ledger, initial path marker and batch routing,
intent/receipt/source/root binding, schema recognition and public history loading.
Reviewer owns only `crates/core/tests/native_flac_journal_independent.rs` and this
report. The parent/implementation worker owns all production corrections. No
runtime, live profile, real music, existing Undo, commits or PRs were changed.
All writes target disposable synthetic FLAC and SQLite fixtures.

Graph Tier 2 used project `tagrex-backend-readiness`, generation
`2026-11-18T12:48:55Z`, with the correct programme root. NativeJournal/native-FLAC
symbol discovery returned no matching node. Exact-path coverage reported changed
journal source and untracked native modules, codec/media and tests. Actual current
source was read directly; the graph cannot verify this newly added integration.

Reusable backend fixes are eligible for curated `tagrex/tagrex:main` with their
required foundations and compatibility review. Preserve upstream inline journal
images and its stable waveform API; fork-specific storage remains separate.

## Findings and retained evidence

### P2: impossible Apply recovery payload was accepted

A normal two-file Apply leaves operation zero with its original selective
receipts. Inserting another file's valid receipt into the later row's `recovery`
column created an impossible initial-Apply state. Undo accepted it, changed the
files and discarded the history rather than refusing the ambiguous collection.
This reproducer does not show foreign-file deletion; it demonstrates a missing
state-role binding and failure of the declared strict collection validation.

The parent added an early rejection of operation zero with any recovery payload.
The unchanged corruption matrix now passes: twelve late-row corruptions across
Strict, KeepChanged and Overwrite refuse with zero callbacks, unchanged media,
unchanged logical SQLite rows and unchanged recovery artifact names/bytes.
Original red: `tmp/flac-journal-independent-first.log`; corrected matrix:
`tmp/flac-journal-op0-green.log`, under the shared tagrex directory.

### P2: final completion callback could remove durable Undo proof before success

The final public Tags completion callback ran after that file's image retirement.
Deleting its durable selective receipts at this boundary still let Apply return
success and mark the batch complete. The callback is observed in-request work,
so the final commit must not certify a known-invalid Undo collection. Since full
images have already retired, refusal may require retained interrupted history and
explicit recovery rather than claiming automatic rollback.

Retained test:
`final_completion_callback_cannot_publish_success_with_known_broken_undo_receipts`.
Original red: `tmp/flac-journal-final-callback-red.log`. The implementation now
validates the complete persisted proof inside the existing IMMEDIATE completion
transaction, after its outcome writes and before commit. The original probe
passes: Apply refuses success and retains interrupted history.

### P2: legacy schema version bypassed unknown native schema recognition

The opener checked native schema signature only when `user_version == 2`.
Existing native history plus an unknown signature and global version zero/one
was accepted as a legacy migration; zero was even changed to one. The same
unknown signature with version two correctly refused. Native-format recognition
must not be bypassed merely by downgrading that one global integer, while
ordinary empty/native-free version-one history must retain its compatibility.

Retained test:
`legacy_version_cannot_bypass_unknown_native_schema_with_existing_native_history`.
Original red: `tmp/flac-journal-schema-downgrade-red.log`. Pre-migration
recognition now checks declared native signature/history/header state regardless
of the global version number. Both zero and one downgrade controls refuse with
unchanged database/media/artifact bytes; ordinary native-free legacy version-one
history still works. The unreleased corrected native signature is
`tagrex-native-flac-journal-v2`.
The unchanged historical 9da reader does not set `user_version`, so that supported
historical-reader refusal path does not require downgrading the new schema.

### P2: deleting the final owner silently shortened the immutable native batch

Deleting the last `file_changes` row with ordinary foreign-key cascade also
removed its native ledger row. The reopened reader compared the two remaining
collections with each other, accepted the shorter plan, and public Undo returned
success instead of refusing. A separate durable expected collection count/digest
and native batch marker are needed independently of the mutable child rows.
Otherwise an incomplete plan can be treated as complete and its history removed.

Retained test:
`missing_last_plan_owner_cannot_shorten_the_native_batch_and_complete_partial_undo`.
The first run compiled and failed because public Undo returned Ok. Its original
assertion was read from tool output; a later diagnostic rerun encountered a
concurrent transitional compile error and overwrote that raw log. The clearly
labelled `tmp/flac-journal-missing-owner-initial-observation.txt` records that
transcript-derived observation; it is not a reconstructed raw run log. The
transitional compile output is kept separately. A durable `native_tag_batches`
commitment now records expected count, ordered plan/root digest and ordered
intent/source digest independently of child rows, atomically with initial
recording. Loading, headers, entries and completion validate this commitment.
The unchanged missing-last-owner probe now refuses before callbacks or effects.

## Independent coverage

Nine normal test functions cover:

- Twelve malformed/substituted later native rows across all three Undo modes:
  source identity, root/plan digest, unknown/out-of-range operation, ordinal,
  operation ID, oversized record, missing receipt, foreign intent/receipt and
  impossible recovery state. No earlier file/history/artifact effect is accepted.
- Mixed/missing native path markers, foreign batch/file owner joins, copied or
  rename flags and future marker encodings. A separate corruption connection
  disables its own foreign-key enforcement only to create otherwise impossible
  persisted evidence; the production connection keeps normal constraints.
- Future global version, unknown native signature and missing signature table:
  refusal preserves database bytes, both media files, recovery artifacts and
  avoids quarantine of a structurally healthy but unsupported database.
- Relative/traversing/NUL/empty marked paths refuse both public headers and Undo.
- A loaded native batch copied into VecJournal cannot fall back to scalar Undo;
  refusal retains its in-memory history and both source images.
- Legacy typed scalar history remains version one and Undo works; a subsequent
  native batch uses version two, loads headers and survives reopened Undo.
- The final callback, schema downgrade and missing-plan-owner regressions above.

Synthetic FLAC is built independently from byte records with literal separators,
empty and case-distinct values, private application bytes and a fixed encoded
frame. Refusal assertions compare complete source bytes. This is durable-ledger
and authority evidence, not a new all-audio-format or physical decoder claim.

## Final verification

On the frozen corrected source, using the existing dev environment:

```sh
source /mnt/Games/Development/tagrex/dev-env.sh
cargo test -p tagrex-core --test native_flac_journal_independent --test native_flac_executor --test metadata_preservation_recheck --test sqlite_journal --locked -- --nocapture
cargo clippy -p tagrex-core --lib --test native_flac_journal_independent --all-features --locked -- -D warnings
rustfmt --edition 2021 --check crates/core/tests/native_flac_journal_independent.rs
git diff --check -- crates/core/tests/native_flac_journal_independent.rs docs/performance/backend-native-flac-journal-independent-review.md
```

**61 passed, 0 failed, two existing manual probes ignored**, across four
integration binaries. The independent binary contributes nine actual parent
probes, none ignored. Subprocess summaries are not counted twice. All four
original semantic regressions are green. Scoped all-features Clippy and owned
format/whitespace checks passed.

Final inspected evidence: `tmp/flac-journal-independent-final.log`,
`tmp/flac-journal-independent-clippy.log` and the original red observations above
under `/mnt/Games/Development/tagrex/`. Initial foreign-key constraint failures
were corrected in the corruption fixture, not production; malformed-data probes
use their own foreign-key-disabled connection while the application connection
retains its normal constraints. All fixture directories are removed by their
own guards, including panic paths. No historical-reader executable or external
media process was created by this reviewer.

The corrected journal also gives recovery receipts an explicit persisted role.
This lets an interrupted rollback-begin preserve its prior Undo proof until the
new preparation, while rejecting impossible operation-zero recovery payloads.
The separate coordinator review owns its direct process/fault recheck; it is not
counted as a new probe in this journal report.

### Reviewed hashes

```text
c29348b001a1b80ba4809a5307da8f36cff012cfcb48a3f78aebb63d0c206fce  crates/core/src/journal.rs
060c9d920204296134f6fb6903685990205d35dedc01b043bdede8c3bebbf512  crates/core/src/journal/native.rs
db0a348d54547326ecc0ce6012549fd84f2d12a9e71fe7de98d0d3360ff17d34  crates/core/src/plan/native_flac.rs
4cd7b00264a0773dcbee0c780af3a446de1fa5b013988d5e90b3e5e0808880b4  crates/core/src/plan.rs
7d3188099fb9e77b8d95e2db7c9c573cda232c4bc06eca8e5ef23fcdd144612d  crates/core/src/model/native_patch/codec.rs
b49cc1c672855bb48c04891310dbc5c42aed0eb8f9c5a58fbf5bc95610fc1cd2  crates/core/src/media_mutation.rs
88690f1be883932cece6345d5cf677249221c92c56ce8ca566a42754a53376ab  crates/core/tests/native_flac_journal_independent.rs
```

## Limits

Linux local fixtures only. No native Windows/macOS, power loss, NAS, physical
audio devices, full-process RSS or all-format preservation is verified. Separate
review covers coordinator callback/rollback edge cases beyond those directly
needed to test journal commitment. Arbitrary malicious alteration of every local
journal field is not cryptographic authority protection: receipt checksums detect
integrity problems and must not be described as authentication.
