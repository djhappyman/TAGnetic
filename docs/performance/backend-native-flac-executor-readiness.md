> Historical author checkpoint. The final clean publication, independent review
> corrections and current limits are recorded in [native-flac-publication.md](native-flac-publication.md)
> and PR #54. Counts below describe the earlier parked-source gate.

# Protected FLAC Executor checkpoint

2026-11-18. Implementation checkpoint for a **new independent review**, not a
whole-backend completion claim. Linux verification only. No commits, PRs, runtime
changes, live profiles or collection writes were performed by this worker.

## Behavior and scope

Ordinary `Executor::apply` plans consisting entirely of in-place FLAC plain-text
tag edits now use a held disposable candidate and protected publication when the
journal explicitly supports the native capability. `SqliteJournal` opts in when
its held profile parent supports the required Linux source identity. The public
scalar `FileChange` interface, shared root/pinned-path/preflight checks and
progress workflow remain the entry point. `PreparedTextPatch::executable()` stays
false: this is not the general exact staging/commands migration.

Supported fields: title, artist, album, album artist, genre, composer, comment,
publisher, ISRC, initial key, catalogue number, URL, media type and custom text.
Existing scalar input separator rules determine *new* multi-values. Old native
lists come from the same held file as the compatibility read, retaining literal
separators, repeated and empty members and original selected-record spelling.
Untargeted metadata, opaque FLAC blocks and encoded audio survive the selective
edit. Undo restores the selected historical records at their current field
position while preserving unrelated current metadata; it does not promise old
cross-field placement after neighbours have changed.

The branch is deliberately whole-plan and narrow. Typed numeric/date fields,
other formats, mixed plans, transfers, sidecars, artwork and whole-block edits
remain on the existing legacy route, selected before native journaling. Vec and
foreign journal adapters have no native capability by default. Those workflows
retain their existing normalization limits. The unchanged preservation canary
passing here proves the admitted route only, not every legacy operation.

## Coordinator and durable protocol

- Capture trusted native/source baselines before *any* public preflight callback.
  Revalidate the whole captured collection after shared preflight, before record.
  Duplicate file/field aliases and unsupported native inventory refuse before
  initial effects. Growth and candidate capacity are also checked before record.
- Record interrupted intent, immutable exact old/new targets, source identity,
  plan/root digests and opaque operation identity atomically with the batch.
  Native paths are strict versioned BLOBs. An unchanged historical String reader
  refuses the whole batch before it can undo anything. Legacy-only journals keep
  version 1; recognized native history uses version 2 with a schema signature.
  Unknown future/unrecognized versions refuse before migration.
- Build on the existing `Store::prepare_file` implementation with an explicit
  pre-recorded operation ID. Every composed field refreshes candidate context;
  the final whole candidate is read and checked before publication. Candidate
  read/write policy must still match at the publication barrier.
- Bind every checkpoint transaction to immutable intent bytes, batch/ordinal,
  original marked path, source identity, roots and operation ID. Persist native
  receipts/outcome before retiring full preimage/candidate files. Confirm the
  actual indexed durable row before retirement; the batch stays interrupted.
  Receipt checksum integrity is not treated as source or root authority.
- Pin the journal profile parent before lazy store creation. Revalidate the
  original logical parent and use held-parent `open_in`; persisted paths cannot
  grant new approved roots or adopt a substituted directory.
- Load bounded collections and validate all intents/receipts/current source
  bindings before opening recovery storage or taking any recovery action.
  Validate all attached store records before the first cleanup or restoration.
- A proved full preimage may be safely discarded; a local Published record and
  matching candidate can reconcile a lost SQL acknowledgement. Retired evidence
  uses the existing terminal-cleanup protocol. Unbound preparation, unknown
  partial publication or corrupt later evidence remains explicit recovery with
  images/history retained. No scalar fallback follows native admission.
- Strict refuses changed targets, KeepChanged reports and preserves them, and
  Overwrite captures the actual current native baseline in inverse receipts.
  Failed Apply/Undo, panic, completion and history-removal failures use exact
  rollback material. Attempted Undo files are tracked *before* publication, so
  an acknowledgement/retirement failure cannot omit the reached file.

## Resource behavior

The existing budgets and Store quotas were not raised. Intents are bounded to
64 fields/file, 32 MiB packed/file and 8192 files. Loaded native collections have
a 64 MiB bound; retained and temporary codec data hold shared leases. Final
candidate growth has a conservative 16 MiB metadata bound and respects the
Store's existing 8 GiB media limit. Images retire per file; the public 160-file
round trip leaves only the Store lock after both Apply and reopened Undo despite
the existing 128-entry Store cap.

The private coordinator retries only typed Busy admission at read-only or
proved pre-mutation boundaries, with a 500 ms bound and 2 ms waits. Apply-side
read/write waits observe cancellation. Journal admission reserves before SQL
changes; SQL errors and uncertain commits are never automatically retried.
The low-level shared Budget stays nonblocking. A control event prevents retrying
native mutation after BeforeMutation. Explicit exhausted-job pressure still
refuses before intent/media effects, releases owners, then permits retry.
Two real concurrent sessions each perform three Apply/Undo cycles without
increasing quotas; this is an integrity/availability probe, not throughput data.

## Findings resolved while integrating

| Rank | Finding | Result |
| --- | --- | --- |
| P1 | Ordinary scalar rewriting normalized untouched native FLAC members | Selective protected production route; unchanged public canary passes |
| P1 | Undo could publish and fail its ack before entering rollback's completed list | Track attempted file first; eight before/after phase failures restore actual external baseline and reopened retry succeeds |
| P1 | Rollback-of-Undo could retain the prior operation's inverse receipts | Preserve until the new preparation, then atomically replace nonzero-operation recovery receipts; reopened retry works |
| P1 | Publication acknowledgements needed immutable initial-intent checks | Same-transaction intent/path/root/source binding; tampered durable intent refuses before media publication and retains evidence |
| P2 | Vorbis WWW was not resolved as the established facade URL field | Reuse established container mapping; public alias test observed red with old mapping and green with fix |
| P2 | Parallel valid work could hit incidental Busy admission | Bounded private pre-mutation admission retry; original parallel recovery stress retained unchanged |

## Verification

Use the existing environment, cache and disposable fixtures:

```sh
source /mnt/Games/Development/tagrex/dev-env.sh
cargo test -p tagrex-core --all-features --locked
cargo clippy -p tagrex-core --all-targets --all-features --locked -- -D warnings
cargo check -p tagrex-commands -p tagrex-service --all-features --locked
cargo fmt --all --check
```

Final full core run: **850 passed, 31 default ignored, zero failures**, across 59
result groups including doctests. This includes the unchanged preservation
canary, 22 new public integration tests, SQLite (29), recovery stress (25 plus two
existing manual ignores), and all retained copy/staging/reader/native/media tests.
The SQL storage unit test exercises genuine same-connection SQLITE_FULL, actual
second-connection exclusive locking and trigger ABORT, checking no completed
publication and retained previous history. The new public tests exercise all
three conflict modes, reopened actual reapply, raw native repeated/absent/empty
values, aliases, unrelated later metadata, cancellation, profile/source
replacement, malformed last receipt, 160 files, durable-phase faults and actual
exit(73) children at Prepared/Publishing/Published/Retired.

Both integration manual tests also passed explicitly:

```sh
TAGREX_HISTORICAL_NATIVE_READER=/path/to/historical-reader \
  cargo test -p tagrex-core --test native_flac_executor --locked -- --ignored --nocapture
```

The historical fixture compiled the **unchanged** core at
`9da71249a62c75189b2131c379fc326244aac3c4` with a tiny caller that opens its
`SqliteJournal` and invokes its public `Executor::undo`. It returned its explicit
refusal exit code 42 on the new native batch; media/history stayed intact and
current reopened Undo still worked. Its archived journal/plan/model sources were
compared byte-for-byte with `git show` for that commit. The other test generated
a real ffmpeg FLAC and checked native values with Mutagen 1.48.1 and nonempty
ffprobe encoded-packet SHA output before Apply, after Apply and after reopened
Undo. Synthetic-frame fixtures alone are not that independent audio proof.

Compact local evidence is retained under `/mnt/Games/Development/tagrex/tmp/`:
`native-flac-all-core.log`, `native-flac-independent-manual.log`,
`native-flac-final-clippy.log`, `native-flac-downstream-check.log`,
`native-flac-final-fmt.log`, `native-flac-undo-ack.log`,
`native-flac-intent-binding.log`, and `native-flac-url-mapping-{red,green}.log`.
Historical extraction/executable and disposable fixtures are task-owned cleanup;
shared toolchains/build caches, active work and runtime/Undo state are preserved.

## Remaining limits and review handoff

- No new independent review has yet accepted this integration checkpoint.
- Linux local filesystem evidence does not prove Windows/macOS behavior,
  physical power-loss durability, remote/NAS guarantees or whole-process RSS.
  A profile/source without required Linux capabilities stays outside this route.
- Genuine ambiguous partial publication and unbound preparation deliberately
  require retained-evidence recovery. This checkpoint does not implement a new
  user-facing recovery UI or a blanket automatic cleanup policy.
- Current Undo holds the bounded source capabilities for whole-collection
  validation. Descriptor exhaustion refuses before source effects; the 8192-row
  structural limit is not a claim that every host can open 8192 files.
- Identical semantic read policy can rebind after restart. A genuinely changed
  interpretation refuses with history retained until compatible policy returns.
- General exact Stage/Review DTOs, native mixed artwork/block/transfer composition,
  typed fields and other formats remain roadmap work. No UI exposure was added.

Owned integration: `plan.rs`, private `plan/native_flac.rs`, journal and private
native ledger, new integration/storage tests; narrow shared helpers in media
mutation, native patch/codec and exact URL mapping. Other touched consumers have
only mechanical `native_tags: false` literal additions. Existing path-authority
implementation and preservation canary are unchanged.

Reusable coordinator and preservation fixes are **eligible for curated upstream
`tagrex/tagrex:main`**, together with their required reviewed foundations. This
worker did not publish or commit. Preserve upstream inline journal images and
stable waveform API; do not wholesale copy the fork's pooled-image storage.
The native journal format/barrier needs explicit compatibility review on each
route; fork-only representation differences remain separate.

## Frozen source digests

The source table below is filled from the checked candidate. A fresh reviewer
must compare these hashes before reusing the verification claim.

| Source | SHA-256 |
| --- | --- |
| `crates/core/src/plan.rs` | `4cd7b00264a0773dcbee0c780af3a446de1fa5b013988d5e90b3e5e0808880b4` |
| `crates/core/src/plan/native_flac.rs` | `58c1ad92a665a8f51e87c6cb642019ab07cdb111804a3fb32a41c37444a6e93a` |
| `crates/core/src/journal.rs` | `9e2e2d0ab8c0ab5b0cdb42eca12c366b7666a4ae27e243983cf24d7c9a795a4b` |
| `crates/core/src/journal/native.rs` | `6e9b59ccfff71aef2ca505787a531c0e0aaf36360c0c1bc9244ddfed746788ea` |
| `crates/core/src/media_mutation.rs` | `b49cc1c672855bb48c04891310dbc5c42aed0eb8f9c5a58fbf5bc95610fc1cd2` |
| `crates/core/src/model/native_patch.rs` | `9b9dbe719fbed356f5369332ab9dc9ecc8d2e90a19eb5fea1b60eaf771465045` |
| `crates/core/src/model/native_patch/codec.rs` | `7d3188099fb9e77b8d95e2db7c9c573cda232c4bc06eca8e5ef23fcdd144612d` |
| `crates/core/src/model/exact.rs` | `125b13593330ddbd0b473397847f7fd2e5f81f3b8a4a05d81d83858520af54c3` |
| `crates/core/tests/native_flac_executor.rs` | `0dc04c681140106cddec5aaa6f1e78bf64b7cadf14473a22e35cc962b2cdb582` |
| `crates/core/src/native_flac_storage_tests.rs` | `a699611985899300ad5afbe30cb6f4ad70a2faef35c9ee93340f790ec18fbcc1` |
| `crates/core/tests/metadata_preservation_recheck.rs` | `760ca90de387ddcec45025febe80f28a62e658ad4a2f672409dec32da31f5fc8` |
| `crates/core/src/plan_paths.rs` | `bdd4d844b6c3f53a5638aa23c82eaa8f4e9648cd0a9f5ccc658f3956a6976544` |
