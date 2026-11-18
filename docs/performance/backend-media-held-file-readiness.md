# Protected held-file editor and typed sink checkpoint

Status: implemented module seam, ready for a new independent review; 18 November
2026. This is **not** Executor, SQL journal/schema, shared-stage or UI integration.
The ordinary Executor preservation regression remains the parent's integration
gate. Scope is `core/src/media_mutation.rs`, new `core/tests/media_held_file.rs`
and this report. No model/native writer/receipt codec/commands/service changes,
commits, publication, live profile or user-media writes were made.

This shared core seam is eligible for curated `tagrex/tagrex:main`. Fork-specific
storage compatibility remains separate; upstream inline journal images and
waveform compatibility are unchanged.

## Interface and invariants

Linux `Store` adds:

```rust,ignore
pub fn prepare_file<S: StateSink>(
    &self,
    original: BoundMedia,
    extension: &str,
    growth: u64,
    sink: &mut S,
    observer: &mut dyn Observer,
    edit: impl FnOnce(&mut File, &mut S) -> io::Result<()>,
) -> Result<Prepared>;
```

The editor receives the already-created, held read/write candidate File and the
same concrete sink subsequently called for `Phase::Prepared`. A caller can place
a native writer's raw receipt in its typed journal driver during editing, then
encode/persist that receipt and the prepared media proof together in
`StateSink::persist`. This avoids simultaneous mutable borrows of one journal,
interior mutability or global driver state. The callback's cursor is unspecified;
editors must seek and separately obey their allocation/growth/cancellation limits.
The format hint remains explicit. No caller-provided media path is reopened by
the held editor adapter.

There is **one** private `prepare_internal<S: StateSink>` routine for admission,
full-image copying, editing, proofs and Prepared acknowledgement. Existing public
`prepare(..., &mut dyn StateSink, ..., FnOnce(&Path))` retains its signature and
private path behavior through a small sized `DynSink` forwarding adapter. The
internal closure receives held File, private path and typed sink; the new facade
exposes only File/sink, and the compatibility facade exposes only the path.
Neither wrapper duplicates preparation logic.

Disk/resource reservations, source/native identity and recovery-artifact
exclusion, full preimage/candidate streams and sync order, private named-object
checks, manifest phase order, durable sink acknowledgements, in-place publication,
uncertain-evidence retention and terminal-proof retirement are retained. No
publication or retirement implementation was changed. A private candidate name
replacement cannot redirect the new editor: it continues editing its original
held object, then named-artifact validation refuses preparation. Conservative
cleanup retains mismatched foreign/evidence entries rather than deleting them.

The seam does not make a sink durable by itself. `StateSink` still requires its
caller implementation to durably acknowledge before returning. An acknowledgement
failure is an error, never permission to publish. An editor or later prepare
callback failure can leave a caller-owned pending/Prepared journal record even
when prepublication cleanup removes the store entry; the coordinator must record
that failed attempt appropriately and must not infer publication from Prepared.
The module does not integrate or repair the caller's history protocol.

## Confirmed correction

**P2: post-editor candidate mutation could become the prepared output baseline.**
The unchanged old path facade was sufficient to reproduce this: edit eight bytes,
change them again from `Event::AfterEdit`, then observe old `prepare` return Ok.
Its later candidate SHA described callback-authored bytes rather than the output
that a captured selective receipt described. The original had not yet changed,
but publication could otherwise accept that new output baseline.

The new `after_edit_mutation_cannot_be_adopted_as_the_prepared_output` canary failed
against source SHA256
`19bcf3d824f08c87c950ef27ace84370780fa7df0458834645afa32a2d03d820`
with its intended assertion, then passed after correction. Preparation now fixes
an edited-candidate native witness immediately after the editor returns, before
`AfterEdit`, and revalidates it afterward. After sync/Prepared acknowledgements,
it also revalidates the witness, held entry authority and the candidate's named
identity before returning. It never recaptures an accepted output witness after
these callbacks. This adds constant-size metadata/identity checks, without an
extra full-media hash per chunk or callback. Original publication still applies
its existing immutable content seals.

A second new canary changes the candidate inside the typed Prepared
acknowledgement and verifies refusal before a Prepared handle can return or any
original publication can occur. These guards share the existing Linux native
witness limitations; they do not isolate files from every uncooperative writer
between a check and syscall, or recover evidence deliberately destroyed outside
the module.

## Actual typed driver and fault evidence

The new fixture `Driver` implements `StateSink` with an owned real File and a
pending reviewed `EncodedReceipt`. During `prepare_file`, the real native writer
changes a FLAC Title to ordered Unicode/empty members; its real raw selective
receipt is encoded into that same driver. At Prepared, the driver writes and
`sync_all`s a fixture acknowledgement containing phase, store ID, before/after
proofs and receipt bytes. Before Publishing, the test independently reads that
file and decodes the receipt, proving the receipt was present before the original
write phase. Published acknowledgement is read back again by retirement's finish
confirmation. After full-image retirement, the receipt is decoded from the saved
file and used for a **second protected operation** that selectively restores the
original exact native records and full original bytes.

Raw checks preserve ARTIST's literal separator, case-distinct GENRE records,
CUSTOM empty/Unicode members, encoded suffix and inode identity. This is a synced
fixture journal plus disk receipt roundtrip, not the application's SQL schema,
a process-restart test, independent Mutagen/ffprobe verification, or complete
FileChange durability integration. The tiny fixture envelope is not proposed as
the production journal format.

The seven new tests also cover:

- Private candidate path replacement during the held callback: the old held
  inode changes, the foreign named file remains untouched, preparation refuses,
  original bytes stay unchanged and mismatched recovery evidence is retained.
- Editor error/unwind and Prepared acknowledgement failure: original unchanged,
  owned preintent artifacts cleaned, operation resource charges released.
- Actual native editor cancellation and native admission refusal after Store
  preparation has already been admitted, with no original mutation or leaked
  operation charges.
- Shared resource saturation before the editor, nested separate operations in
  the same Store, and positive three-byte short allowances through real copy
  loops. Nested sinks/entries preserve their own operation context.
- Both post-editor and Prepared-ack output mutations, including the red/green
  legacy-facade regression above.

Store baseline retained bookkeeping remains charged while Store is open; tests
compare against that baseline rather than incorrectly demanding zero process
usage. Existing admission limits are unchanged: two 64KiB buffers plus bounded
manifest work per Store operation, explicit full disk images/growth reservation,
128 default concurrent entries, 8GiB default media and 64GiB default store budget.
Native editing/codec charges remain separate coordinated leases. No whole-media
Vec, dependency or platform feature was added.

## Executed checks and hashes

All commands reused `source /mnt/Games/Development/tagrex/dev-env.sh`, existing
toolchain/dependencies/build cache and task-owned temporary fixture directories:

```sh
cargo test -p tagrex-core --test media_held_file --locked -- --nocapture
cargo test -p tagrex-core --test media_held_file --test media_mutation --test media_mutation_adversarial --test media_witness_alias_recheck --test media_terminal_recheck --locked --quiet
cargo test -p tagrex-core --test native_candidate_patch --test native_candidate_writer_adversarial_review --test native_receipt_codec --test native_receipt_codec_adversarial --locked --quiet
cargo clippy -p tagrex-core --all-targets --all-features --locked -- -D warnings
rustfmt --edition 2021 --check crates/core/src/media_mutation.rs crates/core/tests/media_held_file.rs
git diff --check -- crates/core/src/media_mutation.rs crates/core/tests/media_held_file.rs docs/performance/backend-media-held-file-readiness.md
```

Results: **7 new seam tests passed**. Combined media suites passed **49 tests**
(7 new, 24 author, 5 artifact/image adversarial, 7 witness/alias and 6 terminal),
with one preexisting author manual performance probe ignored. The unchanged
terminal suite includes its 175 terminal-mutation cases, 39 callback-error cases,
7 panic cases and admission/reopen checks. Candidate/codec suites passed **42
ordinary tests** (17 + 8 + 7 + 10), with four existing manual probes ignored.
Ignored probes were not rerun or counted as verified by this seam checkpoint.
The new caught editor panic prints an expected panic message in `--nocapture`.
All-target/all-feature core Clippy and scoped rustfmt passed.

Final production source SHA256:
`f00f43a943c74d82b711f168168f8eca4c7171b17f1ca6094d07dfb0ad559814`.
New test SHA256:
`8cb137e4e4ddeabc42f737a9f4d10f0cd891265ee3f8a6fecc50ebdde591d1e9`.
No broader diff/source ownership is implied: this checkout contains other active
uncommitted programme components. Fixture guards remove only their own temporary
files; recovery fault fixtures remain only for the duration of their test.

Graph coverage was checked for Tier2 `tagrex-backend-readiness`, generation
2026-11-18T12:48:55Z: media module, author test and receipt codec were untracked;
actual current checkout source was read. This is bounded source verification,
not an exhaustive graph claim.

## Remaining gates

A **new independent review** is required before production integration. The parent
owns protected Executor/FileChange coordination, exact stage and SQL/history
migration, raw receipt authority binding and per-file durable outcome/rollback
confirmation before full-image retirement. A receipt checksum alone is not
historical write authority. Per-file retirement must keep large plans bounded
rather than raising Store entry limits.

The prototype still supports Linux/protection-xattr-capable filesystems only;
Windows/macOS do not gain a success stub or portability claim. No new real ENOSPC,
physical power-loss or process-kill test was added; existing author process-kill
coverage ran as part of its unchanged suite. Existing storage fault limitations
and broader format exclusions remain. No whole-backend, process RSS, end-to-end
performance, restart Undo or UI-exposure readiness follows from this checkpoint.
