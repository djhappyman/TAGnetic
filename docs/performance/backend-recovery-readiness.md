# Recovery readiness: crash and storage failure checkpoint

Date: 18 November 2026. Starting checkpoint: `9da71249` on
`feat/native-field-foundations`. This is one backend hardening workstream, not
completion of the backend roadmap or a platform durability certification.

## Scope and ownership

Owned implementation: `crates/core/src/plan.rs`,
`crates/core/src/journal.rs` and `crates/core/tests/recovery_stress.rs`.
No live app, profile, music collection or existing Undo history is used.
All media is a tiny synthetic FLAC with one encoded constant-audio frame,
Unicode text and an accompanying disposable lyrics file. Fixtures are created
under the task's `TMPDIR` and removed by the parent test after child exit.

Completion requires meaningful baseline failures, actual process termination,
storage-failure checks, independent media inspection, the complete core suite,
warning-free core Clippy and formatting. Parent review and the combined workspace
gate remain separate. Reusable Rust fixes are candidates for curated upstream
`tagrex/tagrex:main`; no journal schema or inline-image compatibility change is
introduced by this workstream. No commit, PR, merge or UI completion is implied.

## Reproduced findings and fixes

1. **Sidecar cancellation/progress gap.** Sidecars were included in the total
   step count but never called the step-begin or step-completion hooks. A test
   with 129 sidecars expected cancellation after the first confirmed sidecar
   transfer; the original executor instead returned a successfully applied
   batch. Sidecar transfers now check cancellation and report completion using
   their own original source path. The service already treats those signals as
   sidecars rather than completed media files. Cancellation rolls back earlier
   media/sidecar transfers through the existing executor.
2. **Damaged journal recovery could destroy previous recovery evidence.**
   A seconds-only `.corrupt-*` name was used with ordinary rename, which
   replaces an existing file on Unix. Precreating the possible current-second
   recovery names reproduced an overwrite. Recovery now atomically reserves a
   unique sibling directory and retains the database and SQLite companions in
   that namespace. Unix recovery directories are private (0700).
3. **Ignored companion move failures.** Recovery previously ignored errors
   moving SQLite `-journal`, `-wal` and `-shm` files before opening a new
   database. It now validates companion kinds, moves them before the main
   database, and refuses fresh creation if any move fails. Best-effort companion
   restoration uses no-clobber hard-link creation. If restoration is unavailable
   or an unrelated entry appeared at the old name, evidence stays in the reserved
   directory and the error identifies its location. Existing entries are retained.
4. **Completion accumulated duplicate outcome rows.** Initial recording already
   stored allowed roots and pruning intent. Finishing appended those again, and
   retrying finish accumulated all outcome records. A baseline test reproduced
   three identical root entries after the first explicit retry. Completion and
   interrupted-outcome retention now atomically replace outcome rows; original
   field, picture and block snapshots stay untouched.

The first, second and fourth regressions were run and observed failing before
their respective fixes. Fault injection supports the third finding; it does not
claim that a physical failing disk was reproduced.

## Adversarial coverage

`real_process_death_preserves_journal_and_retry_boundaries` runs ten scenarios
three times each, using a fresh child process for every case. The child exits
without Rust destructors before/after the initial journal record, after the
first tag write, before/after journal completion, after the first media move,
after all moves, after the sidecar transfer, during partial Undo, and immediately
before journal deletion after Undo. Children have a deadline and are killed and
reaped if stuck.

The parent reopens the actual SQLite journal, checks recorded state, independently
inspects the media and retries ordinary Undo. Unknown interrupted field outcomes
are deliberately refused in all three Undo modes without writing anything. The
test then restores its exact known fixture baseline and verifies safe retry.
Completed and already partly undone batches recover through ordinary strict Undo.
Repeated Undo of a removed batch refuses with `UnknownBatch`.

Other tests exercise:

- Cancellation during a 129-sidecar batch and complete transfer rollback.
- A sidecar destination created after preflight, for both moves and copies;
  the unrelated new target survives failure and earlier transfers roll back.
- Real SQLite `SQLITE_FULL` during snapshot recording using `max_page_count`,
  followed by recovery of the connection and a successful small record.
- An actual SQLite trigger rejecting completion writes, media rollback and a
  successful subsequent Apply/Undo after removing that injected failure.
- Errors at each of the three companion moves and the main database move.
- Failed companion restoration with a newly created unrelated directory at its
  original name, retaining both that directory and the moved journal evidence.
- Nonregular companion paths and preexisting damaged-database recovery names.
- Repeated finish calls without accumulated roots or field outcomes.

## Independent preservation evidence

The normal crash matrix uses an independent bounded raw FLAC/Vorbis-comment
inspector, separate from TagEngine/Lofty. It verifies Artist, an unrelated Unicode
Title and the exact encoded audio-frame bytes both in interrupted states and
after Undo. The fixture's initial comments are also encoded independently.

The explicit FFprobe probe (`ffprobe_verifies_crash_apply_and_restart_undo`)
checks tags and encoded audio-packet SHA-256 with FFprobe n9.0.2, before Apply,
after a real child exit following committed Apply, and after reopened-journal
Undo. This checks actual packet hashes rather than decoded-audio similarity.
It is ignored by default because FFprobe is an external requirement and must
be invoked explicitly; it is not silently counted as passed by a normal run.

## Initial verification

Run using the existing development toolchain and shared build cache:

```sh
source /mnt/Games/Development/tagrex/dev-env.sh
cargo test -p tagrex-core --test recovery_stress -- --include-ignored --nocapture
cargo test -p tagrex-core --lib recovery_fault_tests -- --nocapture
cargo test -p tagrex-core --all-features
cargo clippy -p tagrex-core --all-targets --all-features -- -D warnings
rustfmt --edition 2021 --check crates/core/src/journal.rs crates/core/src/plan.rs crates/core/tests/recovery_stress.rs
git diff --check
```

The nine-test crash/failure integration suite, including the explicit FFprobe
probe, passed. All three journal fault unit tests passed. The complete core
all-feature run passed **460 tests, zero failed, nine ignored** across 18
library/integration/doc-test groups. The FFprobe probe is one of those normal-run
ignored cases and was run separately above. Core all-target/all-feature Clippy
with denied warnings and formatting passed. These checks include the concurrent
resource worker's core edits at verification time; parent integration/review and
the whole-workspace gate remain separate.

## Limits and remaining work

Process exit is not power loss, kernel crash or a storage controller that lies
about flushed data. Kill-during-library-write, power-failure durability, physical ENOSPC faults,
network/removable filesystems and Windows/macOS runtime semantics remain separate
evidence gates. The later independent-review revision below adds explicit
quarantine-move crash coverage and Unix directory flushing.
Hard-link restoration may be unavailable on some filesystems; failure retains
evidence and reports it rather than making a destructive fallback.

Interrupted metadata writes without observed outcomes still require explicit
resolution. This checkpoint does not implement a confirmed-recovery mode, infer
ownership of an unstamped interrupted copy, or weaken normal Undo. Its process
tests exercise FLAC and sidecars; existing broader preservation suites retain
their own format scope. No general claim for every container, artwork operation
or report publisher follows from these tests.

Graph discovery used Tier 2 `tagrex-personal-app`, generation
`2026-11-13T16:10:57Z`, rooted in the other checkout. Coverage marked relied-on
plan/journal/model/test paths stale and service plans untracked; current source
was read directly. Graph snippets did not match symbol bounds and were used
only for discovery, never as proof of current implementation.


## Independent review revisions

The independent adversarial review reproduced four further defects. Its six
canaries are preserved in `crates/core/tests/recovery_adversarial.rs`; all six
now pass. Genuine exclusive-lock and Unix read-denial probes confirmed that
healthy journals are not quarantined for those transient failures.

- **Strong copy identity:** newly completed copies carry streaming SHA-256,
  size and modification time. Evidence is captured from the opened destination
  before publishing progress, including after planned metadata writes. It is
  not reconstructed from arbitrary destination contents after a callback.
  Historical missing or weak size/mtime stamps remain readable but cannot
  authorize deleting an existing copy. Both missing and weak legacy cases have
  explicit refusal tests; ordinary metadata history stays readable.
- **Durable copy removal:** Undo records a private recovery path in SQLite
  before atomically moving a copy there. It verifies the moved bytes again
  before deletion. A replacement between the first digest check and movement
  is retained; a newly occupied restore destination is never overwritten. If
  restoration is blocked, the journal retains the private recovery location.
  Foreign `UndoJournal` implementations default to refusal unless they implement
  the durable-intent method. Runtime Apply rollback uses the same protocol.
- **No-clobber transfers:** moves use Linux/macOS `renameat_with(NOREPLACE)`
  through locked rustix 1.1.5; Windows uses `MoveFileW` through windows-sys 0.61.2.
  Copy creation uses `create_new`. A failed creation is not recorded as an owned
  transfer and cannot cause rollback to delete a competing destination. Case-only
  moves retain the existing same-entry handling. Unsupported no-replace semantics
  and cross-device failures have no copy/unlink fallback. Platform runtime proof
  is still required before Windows/macOS readiness claims.
- **Versioned journal migrations:** schema version 1 accepts legacy unversioned
  journals, adds nullable strong-copy/recovery columns and migrates within one
  SQLite transaction. Unknown future versions are refused before DDL. A failing
  migration leaves the original bytes unchanged. Added a composite copy-stamp
  lookup index so per-copy intent updates do not scan the whole batch.
- **Stable quarantine identity:** main journals must be regular files; relative
  symlinks are refused. Identity/content is revalidated before and after movement.
  A changed main cannot silently become the recovery target of a fresh database.
- **Complete recovery groups:** verified, flushed snapshots of the DB and all
  SQLite companions plus a manifest exist before any original moves. Each
  intermediate move may therefore leave two locations, but one complete original
  snapshot group survives in the reserved directory. Redundant copies are removed
  only after the complete moved group is verified. Faults before this point retain
  snapshots and identify their location. Unix directory entries are flushed;
  physical power-failure behaviour is not certified.
- **Native path identity:** direct Executor operations reject non-Unicode audio,
  destination, sidecar and root paths before journaling or media mutation. Direct
  SQLite snapshot/outcome writes also refuse such paths. This preserves the current
  string-based journal contract until a lossless native-path schema is implemented.

Additional stress/failure evidence:

- Ten real child exits across copy-removal intent, checked, moved, verified and
  removed boundaries, including recreation of an unrelated original target before
  retry. Retrying preserves that unrelated target and finishes the pending removal.
- Four real child exits after successive quarantine moves; every case retains a
  complete database/companion snapshot group and manifest.
- A target replaced with equal length and restored mtime after Undo preflight;
  a target replaced during failed Apply; an original path recreated before move
  rollback; and a replacement exactly between digest validation and movement.
- Sixty-four concurrent destination-creation versus atomic-rename races. Either
  operation can win; neither winner's contents are overwritten by the other.
- Non-Unicode source/destination/sidecar/root refusal and transactional legacy
  migration failure. Existing case-only rename and interrupted recovery suites
  remain part of the full core run.

Final revision verification:

```sh
source /mnt/Games/Development/tagrex/dev-env.sh
cargo test -p tagrex-core --all-features
cargo test -p tagrex-core --test recovery_adversarial -- --nocapture
cargo test -p tagrex-core --test recovery_stress ffprobe_verifies -- --ignored --nocapture
cargo test -p tagrex-core --lib copy_removal_large_batch_probe -- --ignored --nocapture
cargo clippy -p tagrex-core --all-targets --all-features -- -D warnings
```

The current complete core run passed **480 tests, zero failed, ten ignored**
across 20 groups. All six independent adversarial canaries passed, all fourteen
recovery integration tests passed when the FFprobe probe was explicitly included,
and copy/quarantine crash tests passed. Core Clippy with denied warnings,
owned-file formatting and whitespace checks passed. Parent workspace integration
and a fresh independent re-review are still required.

The explicit 256-file / 64-MiB copy-removal load probe rejected a midpoint
same-size/restored-mtime conflict without deleting any earlier copies, then
completed verified Undo after the fixture was explicitly restored. Final local
measurement: conflict refusal 23.31 ms; verified Undo 524.56 ms. This is one warm,
synthetic local-filesystem run including hashes, durable intent writes, no-clobber
moves and Unix directory flushes; it is not an app-wide speedup or cross-platform
throughput promise. Full hashing costs I/O proportional to copied bytes.

Remaining limits: no automatic deletion of unproved historical copies; ambiguous
crashes after deletion but before journal cleanup can still require explicit
resolution if another file subsequently appears at the old target. Do not infer
ownership of that file. Physical power loss, active hostile modification of the
private recovery namespace, network filesystem semantics, kill inside third-party
metadata writes and Windows/macOS runtime behaviour remain unverified. General
confirmed recovery for interrupted metadata writes is still separate work.


## Directory authority and mutation-time baseline revision

A fresh review then reproduced two ancestor-redirection defects and two
same-field lost-update defects in `recovery_durable_recheck.rs`. After public
preflight callbacks, replacing a parent with a link could redirect copy removal
or move Undo into an unrelated fixture directory, even with identical strong
copy bytes. Changing a selected Title after preflight could likewise be
silently overwritten by Apply or strict Undo. All six tests in that fresh review
file now pass; its assertions were preserved.

New owned module: `crates/core/src/plan_paths.rs`. Every Executor operation
captures approved roots and directory identities before callbacks. Each step
captures short-lived parent and regular-file capabilities before invoking its
callback. Unix mutation uses no-follow directory-relative opens, mkdir, unlink
and atomic no-replace rename. It never uses a subsequently redirected ancestor
as the mutation authority. Directory identities and approved-root aliases are
revalidated; drift refuses the operation and keeps necessary journal evidence.
File identities captured before a callback also reject a replacement leaf.

Metadata operations hold a regular-file descriptor for the complete per-file
write. On Linux the existing TagEngine opens that held descriptor through
`/proc/self/fd`; nested tag/cover/block operations use only the internally held
alias. The selected field, cover and block baselines are checked again on that
held file before marking a write attempted. Strict Undo rechecks observed
outcomes at the same boundary, using per-file indexed lookups instead of rescanning
the full batch for every file. Full-block checks include private bytes via the
recorded file hashes. Rollback refuses unproved later values and preserves
unrelated fields. Completion and failed-rollback bookkeeping no longer sample
arbitrary later external edits and promote them into app-owned outcomes.

Only successfully observed own writes/restorations update field or block
outcomes. Cover observations retain a digest, not another full cover per file.
This avoids multiplying shared artwork bytes across a large batch. Transfer
creation remains no-clobber, and cleanup uses the same bounded directory
capabilities. Final directory validation runs once before completion/history
removal; own directory creation/removal updates the authority ledger.

On Unix, reliable directory birth generations permit lightweight
(device,inode,birth) records plus root/per-step handles. Where birth generation
is unavailable, retained inode handles prevent reuse; that fallback is capped at
256 directories. The Unix identity ledger is capped at 100,000 unique directory
entries including ancestors/prospective destinations, with at most 256 approved
roots. This is not a promise that every 100,000-file selection fits. Windows
currently retains deny-delete directory handles and has a conservative 4,096-entry
cap; own prune/remove closes the corresponding cached leaf handles first.
Native Windows validation is still required, including delete sharing,
reparse points, case-only names and directory cleanup. macOS descriptor-alias
behaviour is likewise unverified. No cross-platform scale claim follows.

Additional fixture-only tests cover redirected ancestors for Apply tags,
copy/move, whole-block changes and metadata Undo; a final file link installed
after preflight; late cover and private Vorbis-vendor changes; failed rollback
with a later external field edit; and a completion callback edit that must not
become a recorded app outcome. Outside-operation fixtures remain inside each
test's owned temporary tree; no actual user or system files are used.

Scoped verification (not a full-workspace gate):

```sh
cargo test -p tagrex-core --test recovery_durable_recheck --test executor --test sqlite_journal --test recovery_stress
cargo test -p tagrex-core --test recovery_stress ffprobe_verifies -- --ignored --nocapture
cargo test -p tagrex-core --test recovery_stress mutation_authority_five_thousand -- --ignored --nocapture
cargo clippy -p tagrex-core --lib --test recovery_stress --test recovery_durable_recheck -- -D warnings
```

The four scoped suites passed **102 tests, zero failed, two manual probes
ignored**. FFprobe preservation was explicitly rerun and passed. The Linux
5,000-folder / 5,000-file real rename/Undo probe passed: Apply **3.50 s**, Undo
**2.64 s**, and **four additional descriptors observed at progress callbacks**
(asserted below 64). All source bytes were restored, destinations removed and
created output folders cleaned. This demonstrates folder fan-out beyond 4,096
on this filesystem; callback counts are not a process-wide FD high-water proof,
and timings are a single synthetic local run rather than production SLOs.
Scoped Clippy with denied warnings and owned-file formatting passed.

Remaining material gaps: TagEngine is still a path-oriented writer behind the
held-descriptor adapter. Formats requiring an extension-based fallback, missing
procfs/fdesc support and non-Linux native behaviour need explicit verification or
a native handle-aware model seam. These changes do not fix the separately
reproduced untouched repeated-metadata preservation defect in
`metadata_preservation_recheck.rs`; that remains a selective native-writer
integration blocker. There is also no general filesystem compare-and-swap
against an uncooperative process modifying the same inode *inside* a third-party
metadata write. Checked callback boundaries and pinned identities do not prove
that arbitrary concurrent in-place writers are safe. Power-loss and
kill-inside-library-write recovery remain unverified. The full backend roadmap
is not marked complete.

The final discovery/coverage check used `tagrex-backend-readiness`, actual checkout
generation `2026-11-18T12:48:55Z`. Plan/path/stress sources had subsequently
changed and were read directly; journal and fresh review tests matched that
snapshot. Parent integration and another independent review are required.


## Durable move ownership checkpoint (2026-11-18)

A fresh independent review reproduced two P1 preservation defects: reopening a
pure-move batch accepted an unrelated replacement at its target, and failed-Apply
rollback relocated a replacement installed after the first move. Both are fixed
at the checked operation boundaries. Native object identity is captured before
initial journal record and retained through interrupted/successful outcomes.
The legacy-named `copy_stamps` rows now include nullable `identity` and a `moved`
flag; copies retain their streaming digest and moves require a native identity.
Unix records device, inode, birth time and entry type. Windows records volume,
128-bit file ID and birth time. Missing birth generation refuses a new move
before mutation. This candidate schema remains unpublished version 1, including
transactional legacy-0 migration and unchanged refusal of future version 2.
Existing metadata history remains readable; unstamped historical move rows cannot
authorize automatic transfer, including after their fields are manually restored.
The history and both named files are preserved on refusal.

Move ownership uses object identity rather than content or mtime: an in-place
external edit to the same moved file is deliberately brought back by a path-only
Undo. A different inode with identical bytes is refused. Apply, Undo and rollback
revalidate identity at their mutation boundaries; rollback also checks identity
before declaring a transfer already restored, so a recreated original path
cannot cause recovery history to disappear. The six unchanged independent
`recovery_authority_final_review` canaries pass. Additional stress cases cover
same-inode edits across restart, byte-identical replacements and safe retry,
sidecar link replacement, and false rollback completion with a recreated source.

Sidecar moves now capture the link entry itself through no-follow handles on
Linux, without reading or mutating its target. The existing command test
`each_files_sidecars_come_from_its_own_folder` passes unchanged, including its
relative sidecar link. Regular audio access still rejects links. Copying a
sidecar link is explicitly refused during whole-plan preflight before any
mutation. Other Unix implementations currently refuse sidecar link entries where
no no-follow link-object handle is implemented; native Windows and macOS link
behaviour remains a required platform gate.

The current checkpoint does **not** provide an atomic conditional-source-inode
rename. The native no-clobber syscall protects destinations and pinned directory
capabilities prevent ancestry redirection, but an arbitrary same-user source-name
replacement between final identity verification and rename remains a material
TOCTOU gap. The next scoped implementation is a durable private move-staging
protocol: persist intent before staging, verify the actually staged native object,
restore mismatches without clobber or retain discoverable recovery evidence, and
publish only the proved object. No claim that a wrong source can never move
transiently, or that arbitrary concurrent writers are fully handled, is made.

Checkpoint checks are recorded after candidate verification below; the overall
backend and native platform release gates remain incomplete.

Fresh candidate evidence for this checkpoint:

```sh
cargo test -p tagrex-core --test recovery_stress --test recovery_authority_final_review --test sqlite_journal --test executor --test recovery_adversarial --test recovery_durable_recheck
cargo test -p tagrex-commands --lib each_files_sidecars_come_from_its_own_folder -- --nocapture
cargo test -p tagrex-core --test recovery_stress -- --ignored --nocapture
cargo clippy -p tagrex-core --lib --test recovery_stress --test sqlite_journal --test recovery_authority_final_review --test recovery_durable_recheck -- -D warnings
cargo fmt --check -p tagrex-core
git diff --check
```

Six scoped suites: **118 passed, zero failed, two manual ignored**. The unchanged
command link-workflow test passed **1/1**. Both manual probes then explicitly
passed: independent FFprobe metadata and encoded-packet preservation after crash
and restart Undo; and the 5,000-folder/5,000-file rename/Undo probe, Apply **3.80 s**,
Undo **3.40 s**, **two extra callback-observed descriptors**, with original bytes
restored. The descriptor assertion remains below 64; this is a synthetic local
observation, not a process-wide high-water proof. Scoped Clippy denied warnings,
core formatting and whitespace checks passed. These are Linux scoped results;
full-workspace/Windows/macOS gates and independent re-review remain required.

## Durable private move staging checkpoint (2026-11-18)

This supersedes the identity-only checkpoint's source-stat/rename gap for the
race scenarios exercised below. A concurrent source replacement can still move
transiently into a private reservation; the protocol detects it before claiming
or publishing ownership. It never deletes a moved payload.

- Native move proofs are now captured before **any** public preflight callback,
  with cancellation checked during capture. A later row's callback cannot
  approve a replacement of an earlier row by making its metadata look the same.
- Each move reserves a private sibling directory (Unix mode 0700), flushes its
  reservation and records immutable `MoveStage` intent **before** moving the
  source. The intent contains the transfer key, source, destination, private
  entry path, native object identity and private directory native identity.
- Source and private parent operations use the existing pinned, no-follow
  capabilities. After source→private movement, the actually captured native
  object is verified. A mismatch is returned without clobber if its original
  name is free; blocked restoration retains the private entry and journal.
  Both outcomes report recovery required and never confer app ownership.
- Only a proved private object is published using native no-clobber transfer;
  publication is verified again. A destination collision restores the owned
  private object to its pre-staging endpoint immediately when possible. A
  blocked restoration retains all locations and returns `RecoveryRequired`.
- Restart recovery checks the sealed private-directory identity and returns an
  owned private object to the recorded pre-staging endpoint before ordinary
  Undo validation. Foreign staged objects are restored without overwrite or
  retained; neither is accepted as the batch's object. Nonempty reservations
  refuse cleanup and retain history. Cleanup removes only empty directories;
  no move payload is unlinked. Step directory handles are released before own
  cleanup, including the Windows recovery path, which still needs a native gate.
- `move_stages` is an append-only table indexed by batch, independent of outcome
  rows that `finish`/`retain_interrupted` replace. Its rows cascade only when the
  batch is deleted. The new `prepare_move_stage` trait method defaults to refusal
  so foreign journals cannot silently claim durability. Middleware fixtures
  deliberately forward this API to their real journal. Existing schema-version
  and weak-history policies remain: transactional candidate version 1, future
  version 2 refusal, and no unproved legacy move or copy deletion.
- A recovery-required cause retains history even if concurrent fixture activity
  restores the owned source before best-effort rollback finishes. Commands map
  the new error to `error.plan.recoveryRequired`; the service maps it to
  `recovery_required`. UI catalogue translation is deferred; `ErrorDto.text`
  supplies the existing backend English fallback. No UI source change is part
  of this workstream.

`plan_move_staging_tests.rs` supplies a **cfg(test)-only native hook after the
last source stat and immediately before rename**. The production build has no
hook or environment-controlled sabotage. Probes include source swap, publish
source swap, late destination creation, blocked restoration of both owned and
foreign material, reservation replacement across reopen, retry, safe refusal by
an unsupported journal, and an actual SQLite pager `SQLITE_FULL` during intent
commit. The full-disk probe uses a capped real pager plus a valid long source
path requiring an overflow page; the database itself stays at a short fixture
path because SQLite's Unix path limit is narrower than the filesystem's.
Fourteen real child exits exercise seven Apply and Undo phases (intent, checked,
staged, verified, publish-checked, published, cleaned). Two more exits stop with
an unverified foreign entry already captured: sixteen exits in total. All child
roots are disposable.

Integration adds mixed copy/move batches with a relative sidecar link that is
intentionally dangling at its moved location, cancellation after its transfer,
restart Undo, exact original-byte restoration, linked-target preservation and
stage-row cleanup. Existing independent authority/adversarial/durable canaries
are unchanged. Source proofs also reject a byte-identical replacement installed
by a later row's earlier preflight callback.

The 5,000-folder/5,000-file real filesystem probe passed with **four additional
callback-observed descriptors** (asserted below 64), original bytes restored and
created output folders cleaned. Its staging run took **25.41 s Apply / 30.29 s
Undo**, versus the earlier identity-only run's **3.80 s / 3.40 s**. These are
single synthetic observations across revisions with uncontrolled filesystem
cache state and concurrent build activity; they do not isolate SQL commit,
fsync or mkdir costs and are not production SLOs. The cost is material. Intent
batching and shared private reservations may reduce it, but must preserve
intent-before-mutation, native ownership verification and restart semantics.
The throughput goal is **not** marked achieved; that optimization is a separate
scope after fresh correctness review.

Power-loss durability, kill-inside-third-party metadata writes, arbitrary
concurrent in-place writers, cross-device/network filesystem behaviour and
native Windows/macOS execution remain unverified. In particular this does not
assert that directory identity/birth semantics are equally strong on every
filesystem, or that a hostile same-user namespace actor cannot cause transient
moves or interfere with empty-directory cleanup. The tested preservation goal
is no payload deletion, no destination overwrite, no false ownership/success,
and retained discoverable recovery evidence under the injected races.

Fresh scoped candidate gate:

```sh
cargo test -p tagrex-core --lib --test executor --test sqlite_journal --test recovery_stress --test recovery_authority_final_review --test recovery_durable_recheck --test recovery_adversarial --quiet
cargo clippy -p tagrex-core --tests -- -D warnings
cargo test -p tagrex-commands --lib each_files_sidecars_come_from_its_own_folder -- --nocapture
cargo check -p tagrex-commands -p tagrex-service
cargo fmt --check -p tagrex-core
git diff --check
```

**378 passed, zero failed, three ignored** across the core unit suite and six
focused integration suites (258 + 48 + 6 + 6 + 6 + 25 + 29). The two recovery
manual probes were explicitly run and passed as reported above; the remaining
core-unit ignored probe is outside this scoped gate. Eleven move-staging unit
probes are included in the unit total. The unchanged command sidecar workflow
passed 1/1. Core all-test Clippy denied warnings, commands/service compilation,
core formatting and whitespace checks passed. No full-workspace release gate or
fresh independent review approval is claimed.

Stable review snapshot SHA-256 (source, not a commit or publication):

| File | SHA-256 |
| --- | --- |
| `crates/core/src/plan.rs` | `cade8dcf7dfa8e81146373ea19ec84901f618c37abe7a4ebcb224a7f25087ec0` |
| `crates/core/src/journal.rs` | `350d6c23ee1c06afe150f96e7b5f25f538589bcc010ec26b4ac7f91e98b28a7d` |
| `crates/core/src/plan_paths.rs` | `153c839b665791c5a80bb6d85aee7e9754f06e1f78a3596c6c8a585980f08027` |
| `crates/core/src/plan_move_staging_tests.rs` | `92d025450008244ab0e060ab8ec674951a6e7d7543d5947f3490c800657f6d04` |
| `crates/core/tests/recovery_stress.rs` | `f83e573bb9b17b3267ff396b51e1523b94c6e6d7ab0b2b7276a9e962c74ab4a8` |

The codebase-memory generation remains `tagrex-backend-readiness`
`2026-11-18T12:48:55Z`; these edited paths are stale relative to it, so this
checkpoint used current source reads rather than graph snippets for its claims.
Future metadata staging that publishes a new inode must persist a proved native
identity handoff before subsequent moves; it cannot silently replace the current
in-place writer and retain the old ownership identity. That integration and
batched intent throughput work remain separate scopes. Shared backend fixes are
eligible for curated `tagrex/tagrex:main`; the fork's image storage compatibility
difference remains deliberate and upstream inline images must be preserved.


## Whole-collection move context correction (2026-11-18)

The fresh [independent review](backend-move-staging-independent-review.md)
reproduced a P2 consistency defect in recovery metadata handling. Changing a
stored stage's source endpoint moved owned or foreign material to an unplanned
name before reporting an error. A second invalid stage could follow a valid
stage, cause partial recovery, and even produce successful Undo with history
removed. A malformed native identity also allowed restoration before refusal.
The unchanged review suite was reproduced at **two passed / four failed** before
this correction and now passes **all six**.

The correction compiles an operation context from the original recorded plan,
its typed persisted move proofs and approved roots. Its transfer endpoints and
proofs stay fixed throughout callbacks and native operations. SQL outcome
updates already preserve the recorded plan; `VecJournal` now preserves that
same immutable plan contract in `finish` and `retain_interrupted`.

Every stored stage must match one non-copy audio move or sidecar move in that
plan, with an exact original→target or target→original endpoint pair and the
exact persisted native identity. Duplicate move proofs, proof targets absent
from the plan, conflicting original sources and audio/link role mismatches are
refused. Private entry paths must be normalized absolute paths, end in `entry`,
use the exact batch-specific reservation-name grammar, and belong directly under
their planned source parent. Parent traversal, a different entry name, an
unplanned source/destination or a reservation associated with another batch is
not accepted merely because it falls inside an approved root.

Validation covers the **entire collection before the first restore or cleanup**:
metadata binding, strict identity encoding, root containment, duplicate private
paths or directory identities, retry/resource bounds, every present directory
seal, pending entry transport class, and conflicting pending captures. Only a
validated collection can enter the mutation helper. This applies to foreign
journal adapters as well as SQLite. Invalid context also stops best-effort Apply
rollback before any later transfer or metadata restoration; it does not merely
record an error and continue mutating other rows. Context validity does not
promise that later independent namespace/IO faults cannot interrupt an otherwise
valid Undo.

Native identities now have a strict decoder: known version, exact length,
nonzero native object ID, valid entry-role encoding and representable birth time.
Unix U1 remains supported. Windows writes W2 with explicit regular-file, link or
directory role. W1 remains decodable for inspection but lacks an entry role and
cannot authorize a typed move or private-directory recovery. This is an explicit
safe refusal of weak historical authority; ordinary metadata history remains
readable. The Windows codec's role contract is exercised on Linux as wire data;
that is not native Windows FileIdInfo/filesystem verification. Native Windows,
macOS and cross-filesystem gates remain pending.

Recovery accounting is bounded at **262,144 stage rows**, **128 MiB of accounted
path/identity bytes**, and **64 persisted intents per transfer**. SQLite rejects
path cells above 128 KiB and identity cells above 64 bytes before loading their
payloads into Rust, and reads rows incrementally with a limit. Foreign adapters
receive the same collection validation. Active operations check the same limits
before appending a new intent, preventing an admitted 65th retry from creating
history the replay validator rejects. These are metadata accounting bounds, not
an RSS limit for all plans, images or hash-table overhead. Very large or repeatedly
failed histories can require smaller batches or manual resolution; no 100,000
file throughput or universal recovery claim is made. Completed-stage compaction,
whole-operation admission and intent batching remain future bounded-design work.

Additional fixture probes cover later malformed rows without even cleaning an
earlier empty reservation; duplicate private paths/directory identities; bad
versions, lengths, object IDs, birth times and role bytes, including matching
corruption of both the saved proof and stage; wrong private entry names/parents;
an unexpected directory payload; foreign journal duplicates; rollback refusal;
stale empty-intent limits; oversized SQLite cells; and new-intent/replay budget
consistency. Two additional real process exits leave one completed and one
pending move stage. Valid context replays safely; corruption of even the completed
row refuses before the pending row or ordinary Undo moves anything. The original
six review assertions were not changed.

This validates consistency against the recorded plan and proofs; it does not
cryptographically authenticate arbitrary edits to every authoritative journal
fact. Existing limits concerning hostile same-user namespace actors, concurrent
in-place writers, power loss, directory cleanup races and metadata-writer inode
handoff remain explicit. Throughput optimization has not been implemented or
claimed in this correction.

Fresh scoped gate:

```sh
cargo test -p tagrex-core --lib --test executor --test sqlite_journal --test recovery_stress --test recovery_authority_final_review --test recovery_durable_recheck --test recovery_adversarial --test move_staging_adversarial --quiet
cargo clippy -p tagrex-core --tests -- -D warnings
cargo test -p tagrex-core --test recovery_stress -- --ignored --nocapture
cargo test -p tagrex-commands --lib each_files_sidecars_come_from_its_own_folder --quiet
cargo check -p tagrex-service
cargo fmt --check -p tagrex-core
git diff --check
```

The unit suite and seven focused integration suites passed **393 tests, zero
failed, three ignored** (267 + 48 + 6 + 6 + 6 + 6 + 25 + 29). Twenty staging unit
functions, including the child driver, are included. Both recovery manual probes
were then explicitly exercised: FFprobe metadata/encoded-packet preservation and
5,000-folder/5,000-file rename plus Undo. The latter restored original bytes and
cleaned created folders with **four extra callback-observed descriptors** below
the existing 64 assertion; its single local run measured **25.85 s Apply / 31.18 s
Undo**. The extra pending-entry-class prevalidation is not on this completed-row
fan-out path. These are uncontrolled local observations, not production SLOs or
an isolated comparison of SQL versus filesystem cost. The unchanged command
sidecar workflow passed 1/1. Core all-test Clippy denied warnings, core formatting,
whitespace and service compilation passed. Fresh independent review and full
native/workspace gates are still required.

Stable correction snapshot SHA-256:

| File | SHA-256 |
| --- | --- |
| `crates/core/src/plan.rs` | `9d9678451dd6303ec16185ccc57819b990b9de9c49c2a4456ee7ce5b4a9e737c` |
| `crates/core/src/journal.rs` | `7e02c37532994d11cea46a25186a681e77544f5167a33cebd138bbaba96d2fad` |
| `crates/core/src/plan_paths.rs` | `153c839b665791c5a80bb6d85aee7e9754f06e1f78a3596c6c8a585980f08027` |
| `crates/core/src/plan_move_staging_tests.rs` | `51ad4d11a2d8154c048f064f881ac372949f178ebde2f6732da32018c0adfc05` |
| unchanged reviewer `crates/core/tests/move_staging_adversarial.rs` | `d731cbe1c3a7284cfe5b3fbd5eb476da0ec1f30e5146d03ad5783a3ecbb10965` |

Discovery coverage remains the actual-checkout graph generation
`tagrex-backend-readiness` `2026-11-18T12:48:55Z`, with these edited paths stale
and the fresh reviewer test untracked. Relied-on code was read from current
source. This reusable backend correction follows the curated shared
`tagrex/tagrex:main` route; incompatible fork storage and upstream inline journal
images remain separate. No UI source, live app, real media, profile, commit or
PR was changed by this correction.


## Explicit Unix U2 role correction (2026-11-18)

The [fresh context recheck](backend-move-context-recheck.md) found that Unix U1
still encoded only a link boolean. A genuine directory identity could therefore
be placed in both a moved-file proof and its stage identity, satisfy the initial
context checks, and cause a regular pending payload to move before refusal.
The unchanged public-Executor/SQLite canary was reproduced with **seven passed /
one failed**. It now passes with all **eight recheck canaries unchanged**.

New Unix identities use `TRX-U2` with explicit regular-file (0), link (1) or
directory (2) role. The native object locator and birth-time fields are unchanged.
File/link authority and directory authority are disjoint, so copying an actual
directory proof into file slots fails context validation before any recovery
mutation. The reverse file→directory substitution is also refused. Whole-set
validation, immutable endpoints, root authority, admission bounds and no-clobber
restore/publication semantics are preserved. Windows W2 is unchanged.

Compatibility is deliberately limited to provable roles:

- Unix U1 false meant either a file or a directory. It remains decodable/readable
  as an ambiguous legacy value, but authorizes neither role. Old move proofs and
  reservation seals with that value refuse automatic recovery before restoring
  or cleaning anything. Older copied-file stamps carrying ambiguous U1 native
  evidence also refuse automatic deletion; source, copy and history remain.
- Unix U1 true explicitly meant a link. It retains link authority only for an
  immutable sidecar move intent, never audio-file or directory authority. Decoded
  comparison can match its unchanged device, inode, birth time and link role to
  the current U2 link object. This does not infer a missing role from the current
  pathname or rewrite an old proof as U2.
- Every required reservation directory still needs typed directory authority.
  A legacy U1 link does not make an old ambiguous U1 directory seal admissible,
  including retained completed-stage context. Unprovable histories stay intact
  for inspection. Ordinary metadata history remains readable.

Legitimate late foreign capture remains a separate preservation case: it is
handled only under an otherwise valid typed original proof and immutable stage
context, restored without clobber to the **recorded approved source**, or retained
privately if that path is occupied. Invalid role evidence cannot enter that
restoration path. The previous native post-stat foreign-capture tests still pass.

Additional fixtures verify U2 role/unknown-version encoding, ambiguous U1 file
and directory-seal refusal across reopen, reverse role substitution, old-copy
preservation, and the known-link compatibility subset. One actual child exit
leaves a completed regular move and pending link capture with U2 directory seals;
its explicitly typed U1 link proof safely recovers after reopen. A completed
move/copy/link batch also reopens and undoes with that link proof, keeping the
relative link entry, its referent, both original WAV payloads and regular sidecar
bytes unchanged. The link is intentionally dangling at its moved position. These
are disposable fixture tests; the retained reviewer canaries were not edited.

The SQL journal remains candidate schema version 1; the native identity version
is a separate self-describing blob format. No historical proof is silently
migrated, and no future SQL schema allowance or image storage policy changed.
Upstream inline images remain a distinct compatibility requirement. Native
Windows/macOS execution and filesystem-specific birth/identity guarantees remain
unverified; the U2 and W2 wire-role checks do not certify those platforms.

Fresh scoped gate:

```sh
cargo test -p tagrex-core --lib --test executor --test sqlite_journal --test recovery_stress --test recovery_authority_final_review --test recovery_durable_recheck --test recovery_adversarial --test move_staging_adversarial --test move_context_recheck --quiet
cargo clippy -p tagrex-core --tests -- -D warnings
cargo test -p tagrex-core --test recovery_stress -- --ignored --nocapture
cargo test -p tagrex-commands --lib each_files_sidecars_come_from_its_own_folder --quiet
cargo check -p tagrex-service
cargo fmt --check -p tagrex-core
git diff --check
```

The core unit suite and eight integration suites passed **407 tests, zero failed,
three ignored** (273 + 48 + 8 + 6 + 6 + 6 + 6 + 25 + 29). Twenty-six staging unit
functions, including child drivers, are included. Both recovery manual probes
explicitly passed: independent FFprobe metadata/encoded-packet preservation, and
the 5,000-folder/5,000-file rename/Undo probe. That single local U2 run restored
original bytes, cleaned created output folders, and observed **four extra
callback descriptors** below the existing 64 assertion. It measured **26.00 s
Apply / 29.95 s Undo**. Throughput is not marked solved: cache/load conditions are
uncontrolled, timing attribution is not isolated, and optimization remains a
separate scope after independent correctness review. The unchanged command
sidecar workflow passed 1/1. Core all-test Clippy denied warnings, formatting,
whitespace and service compilation passed. No native/workspace release gate or
new independent review approval is claimed.

Stable U2 review snapshot SHA-256:

| File | SHA-256 |
| --- | --- |
| `crates/core/src/plan.rs` | `2d2e9c0cd864fba115bb24a5ab79155cfa8b629f43136caa33e1f168b4201e2a` |
| `crates/core/src/journal.rs` | `e1326010c208ef8ebb9b3d255644412b112e87771c725716d4222cb42bd75331` |
| `crates/core/src/plan_paths.rs` | `153c839b665791c5a80bb6d85aee7e9754f06e1f78a3596c6c8a585980f08027` |
| `crates/core/src/plan_move_staging_tests.rs` | `978def07993373b5c89514095ce68b19a3b37da970fae076088f212eb846f456` |
| unchanged reviewer `crates/core/tests/move_context_recheck.rs` | `f5a36941467196b41156f7866d10a07370907e29f3e96c63c4a87a3c2d091217` |

The actual-checkout graph remains generation `2026-11-18T12:48:55Z`, stale for
these edits; current source was read for the role codec and recovery boundary.
Reusable correction route remains curated `tagrex/tagrex:main` with deliberate
fork storage differences preserved. This correction did not change UI source,
other authors' output/root/media modules, live app state, profiles or real media,
and made no commit or PR.

## Indexed staging cleanup performance checkpoint (2026-11-18)

Native profiling found a quadratic full-ledger scan in each successful Unix
private-directory retirement. Ordered directory-path and reverse canonical-alias
indexes now retire precisely the same component subtree without scanning
unrelated folders. The authoritative maps, directory identities, fallback-pin
rules, per-step intent transactions, fsyncs, cancellation, native role checks and
persisted recovery protocol are preserved. No batching API is included.

The corrected 5,000-folder/file probe passed at **16.97 s Apply / 17.13 s Undo**
against the earlier U2 run's **26.00 s / 29.95 s**, with four callback-observed
extra descriptors and byte/path/history assertions intact. At 1,000-file fan-out,
exclusive cleanup time fell from about **0.66–0.81 s to 0.33 s**; all 1,000 intent
SQL transactions still execute. An intermediate index-coherence failure was
corrected and its measurements discarded before checkpoint.

The scoped gate passed **416 tests, zero failed, four ignored**, plus explicitly
run public/native profiles, 5,000-folder and FFprobe manual probes. Clippy with
denied warnings, formatting, whitespace, service compilation and the unchanged
command sidecar workflow passed. All independent recovery canaries remain
unchanged, including safe refusal for the unfinished-copy proof gap.

[Full conditions, cold/warm limitations, component timings, invariant canaries,
commands and source hashes](backend-staging-cleanup-profile.md) bind this
checkpoint. The throughput goal remains incomplete. A new independent index and
authority review is required; SQL batching and shared reservations remain future
scopes. No commit, PR, UI, runtime/profile or real-media change was made here.
