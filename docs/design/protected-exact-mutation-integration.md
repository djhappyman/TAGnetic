# Protected exact writes: integration work

This is the implementation design for the next Wave 2 seam, not a completion
claim. The original public Executor Title-only preservation regression remains
red. A selective disposable-candidate writer or exact reader alone does not fix
Apply, persistent Undo or the shared stage.

Reusable backend changes are candidates for curated `tagrex/tagrex:main`.
Fork journal/storage changes need a separate compatibility review; preserve the
upstream inline journal image representation and waveform interface. Work stays
in the existing foundation checkout. No live profile or user audio is a fixture.

## Existing modules and current limits

`model::exact` supplies native identity, exact absent/present ordered states,
selected container/block/revision, raw block digest and read policy. Prepared
intents deliberately remain non-executable through the production Executor.
`model::native_patch` edits only a caller-supplied disposable read/write File.
Its receipt retains the changed raw records, before/after target digests and
neighboring record identity anchors. Selective restore preserves unrelated
current metadata and refuses ambiguous anchors. Its native profiles and refusal
cases are recorded at its own checkpoint; they must not be generalized to every
currently readable or writable format.

`media_mutation::Store` keeps full preimage/candidate evidence, publishes in
place to preserve the original inode, and detects changed recovery artifacts.
It currently supports Linux only and remains outside the Executor. Its path
callback was designed for existing writers; the exact writer takes a held File.
Its typed held-file edit interface now passes the same sink to the editor and
preparation acknowledgements. The independent held-editor review verifies that
an observer cannot substitute a later candidate image. `TagEngine::read_exact_file`
now reads exact and compatibility state from an owned held file without reopening
its logical label. These two independently reviewed seams still require an
authoritative Executor/journal coordinator.

The existing journal records interrupted intent before the write phase and
records completion separately. Its scalar old/new fields do not encode native
lists or authorize raw selective restoration. Existing historical records must
remain readable. A journal row or a filename by itself is not mutation authority.

## One production mutation seam

Keep Executor as the sole coordinator of authorization, cancellation, batch
progress, file edits, transfers, rollback and history. Implement one private
module for a complete in-place FileChange. Its interface prepares a candidate,
persists its recovery/Undo evidence, publishes it, and returns an observed
outcome. Tag, artwork and block work must eventually compose into this complete
candidate instead of publishing several independently recoverable half-edits.
The legacy and exact adapters genuinely differ; callers must not learn their
container parsing or artifact cleanup protocols.

For exact text, the ordered sequence is:

1. Verify the complete plan, trusted source snapshot, native identity, old state,
   policy interpretation, supported profile and aggregate resource bounds.
2. Record the interrupted batch before any media mutation. Reserve full recovery
   images and conservative candidate growth before invoking the native writer.
3. Edit the held disposable candidate. Collect the raw selective receipt and
   verify the candidate's encoded media and untargeted metadata invariants.
4. Durably bind the receipt and store record to batch, file-change position,
   original logical path, native source identity and the plan's exact intent
   before publication can start. Persisted metadata never broadens allowed roots.
5. Publish using the protected module. Persist its acknowledgement. A missing
   acknowledgement or ambiguous current image remains recoverable evidence and
   an explicit unresolved operation, never inferred successful rollback.
6. Durably commit each published file's observed outcome and validated selective
   rollback receipt before confirming its full-image retirement. Keep the batch
   interrupted until the entire plan completes. This permits bounded recovery
   storage across large batches without pretending the overall batch finished.
   If a complete FileChange includes work whose rollback still needs full images,
   those images cannot retire under the text-only rule. Keep selective Undo
   receipts after full preimages can safely retire.

The sink and editor need one mutable journal owner without hidden global state
or overlapping mutable borrows. A held-file preparation interface can pass the
same typed sink to the editor and subsequent state acknowledgements. Its durable
finish confirmation must prove the complete file outcome and its rollback
material; a boolean or a local success flag cannot stand in for that check. Reentrant
independent operations must preserve their own journal/source contexts. Do not
hold the service registry across file copying or callbacks.

## Durable receipt validation

Use a bounded, versioned codec with checked lengths/counts, strict enum tags,
UTF-8/native-name validation, exact end-of-input and content integrity. Reserve
owned decoding memory before allocating. Reject malformed/future data without
editing media, deleting evidence or discarding existing history. Keep core free
of transport-specific serde requirements.

Raw target records must be parsed with the same native parser as candidate
editing. Derive their selected identity, key and semantic values from their
bytes; never trust separately serialized `Node.selected`, keys or values. Bind
raw runs and after-target bytes to their semantic states, digests and context.
Anchors need bounded ordinal/count validation and ambiguous relocation refusal.
A checksum detects damage; it does not turn caller-authored bytes into trusted
historical write authority. Whole-collection context validation comes before
any earlier file recovery or artifact cleanup.

A process-local policy revision is not a durable policy identifier. After a
restart, identical settings can have a different revision counter. Restore must
validate the stored semantic policy and selected native context, then explicitly
bind to the current process generation. An old numeric revision alone cannot
permit or permanently prevent legitimate Undo. A genuinely changed native
profile or incompatible interpretation must refuse with retained history.

## Commands, staging and old history

Carry exact target identity and absent/present ordered values in versioned field
intent/outcome envelopes through the existing commands DTO, staged revision,
digest, Apply and journal. Clients provide desired values and target identity;
trusted native reads supply old state and source evidence. Scalar display
projections remain useful, but equality of projections is not exact equality.
Do not split a literal separator to invent historical native values.

Preserve locks, excluded rows, selection, sidecars, renames and composed recipes.
An exact no-op decision uses exact states. Old readers must refuse new exact
mutation payloads rather than silently applying a scalar projection. Existing
scalar history keeps its recorded meaning; unavailable exact ownership is an
explicit limitation. Do not expose an exact-write capability before the whole
path is implemented and independently reviewed.

## Required public acceptance evidence

Keep `metadata_preservation_recheck` unchanged and make it pass through the
ordinary Executor. Add supported-container Apply/reopened-journal/Undo/Redo
probes with independent raw metadata, Mutagen/FFprobe and encoded packet hashes.
Include empty, repeated, case-distinct, Unicode and separator-literal values;
qualified targets; opaque frames/application data; pictures and chapters.

Inject failures before/after evidence persistence, editing, publication,
acknowledgement and retirement. Use real process exit and reopen. Compare source,
foreign objects, all relevant journal rows and artifacts. Include unrelated
post-Apply metadata changes, same-object hardlinks, replaced sources/ancestors,
short writes, space limits, cancellation, panic, pressure and ambiguous history.
Keep the original refusal canaries; a safe error must not conceal an earlier
unauthorized effect. Include batches exceeding the store's 128 concurrent-entry
limit and enforce bounded retirement rather than silently raising that limit.
Measure full-image copying, hashing, syncs and retained disk space separately
from native parsing/SQL time. Durable receipt validation is not an excuse to
read/copy a large media payload several extra times without measuring the cost.

Linux prototype evidence does not verify Windows, macOS, NAS, filesystems
without required native capabilities, physical power loss or complete UI
readiness. Those remain explicit delivery gates. Broader old-format workflows
need an intentional adapter/refusal policy before changing the default writer.


## Next journal checkpoint: compatibility and authority requirements

Before implementing the production coordinator, keep the journal work one
reviewable concern with these acceptance conditions:

- Bind each exact intent and receipt to the original batch, file-change ordinal,
  logical source path, approved roots, selected native target and old/new state,
  and native source identity. A valid codec checksum is insufficient authority.
- Persist the new interpretation marker atomically with the initial interrupted
  batch record, before any media effects. Adding only `PRAGMA user_version=2`
  cannot exclude older shipped readers that never inspected that pragma. A new
  row representation must make those readers fail during whole-batch loading,
  before their scalar Undo logic can mutate a file. One candidate is a strictly
  versioned BLOB in the existing TEXT-path column, decoded explicitly by the new
  reader; verify actual older-reader failure rather than assuming a schema bump
  supplies a barrier. New legacy-only history should retain its existing format.
- Prepared and published outcomes must validate the whole collection before
  recovery or artifact removal. Refuse unknown/future/duplicate markers, missing
  receipts, incompatible policy interpretation, native identity disagreement or
  source/store mismatch without deleting images or discarding history.
- Store creation now has a separately reviewed held-parent opener, including
  controlled initialization replacement probes. The journal coordinator must
  acquire and retain that profile-parent authority before a later lazy open.
  Persistent store paths never grant additional filesystem roots.
- Committing a file outcome and its bounded selective rollback material permits
  retirement of that file's full images while the batch remains interrupted.
  Exercise more than 128 files under the existing store capacity, with actual
  crash/reopen between outcome commit and retirement. Do not raise the store cap
  to conceal missing per-file retirement.
- Multiple exact fields share a changing block digest. Preflight all original
  states together, then refresh only internally authorized candidate context
  between edits. Reusing a stale whole-block digest or treating a projection as
  exact old state is invalid.
- Preserve the existing explicit Undo conflict modes. An overwrite must capture
  the actual current state for its own rollback and retain the historical exact
  state; silently falling back to the scalar writer would lose the new contract.

The copy-checkpoint candidate adds a separate explicit adapter opt-in and durable
per-copy outcome contract; it does not implement any of these exact-media
journal operations. Its fresh independent review is complete on the corrected source, with 437
retained tests passing; the production integration worker now owns subsequent
`plan.rs`/`journal.rs` changes and must retain those authority regressions. The SQLite busy/cancellation observation
(about 15 seconds across sequential attempts) remains a distinct latency scope.
