# Selective native candidate writer checkpoint

Status: independently reviewable module, 2026-11-18. This is a pathless disposable
candidate primitive, **not** production Apply, persistent Undo, stage/journal
migration, or complete format support. `PreparedTextPatch::executable()` remains
false. The original unchanged `metadata_preservation_recheck` Executor regression
still needs protected selective writer integration. No production writer was
switched by this checkpoint.

The implementation is in `crates/core/src/model/native_patch.rs`, registered by
one model module declaration. Tests are `crates/core/tests/native_candidate_patch.rs`.
The final factoring in `model/exact.rs` moves the existing semantic capability
checks into `validate_text_intent`; `prepare_patch` calls that same helper. Core
remains serialization agnostic. The shared backend route is a curated
`tagrex/tagrex:main` contribution; fork journal/storage compatibility stays a
separate integration concern. No commit, publication, live profile or collection
writes occurred.

## API and admitted profiles

`apply_prepared[_with_budget](&mut File, &PreparedTextPatch, ..., &mut dyn Control)`
returns an owned `Receipt` only after semantic/native/raw readback and preservation
checks. `restore[_with_budget](&mut File, &Receipt, ..., &mut dyn Control)` restores
only the selected raw records, returning the reverse receipt. This supports
selective exact restoration and Redo without restoring an entire old file over
unrelated current changes. Both APIs require an exclusively controlled, trusted
read/write **disposable candidate**. The caller must supply original authority,
private storage, durable intent and protected publication. Failure after candidate
mutation means discard that candidate; the primitive does not roll back a torn
candidate or claim the live original was protected by itself.

| Profile | Admitted edits and preservation | Explicit exclusions |
| --- | --- | --- |
| Ordinary FLAC | One existing bounded Vorbis block; literal ordered values including empty members; all other block/record bytes and encoded suffix retained | Multiple Vorbis blocks, name case collisions, malformed block chains, absent comment block, oversized metadata |
| Leading ID3 MP3 / ADTS AAC | Unflagged v2.3 Latin1 scalar or v2.4 Latin1/UTF8 lists; TXXX, COMM/USLT qualifications; untouched UTF16/private frames and padding retained verbatim | v2.2, tag/frame flags, unsynchronization, extended headers/footer, compressed/encrypted frames, UTF16 **targets**, legacy-codepage Latin1 targets, v2.3 lists, qualified comment/lyrics lists, junk before audio |
| Classic local MP4/M4A | Bounded existing ilst; full freeform mean+name, UTF8/UTF16 data and locale; literal lists/empty members; opaque unselected atoms preserved | Fragmented/encrypted/external-reference layouts, mixed targeted data qualifiers, unknown targeted children, duplicate targeted items, missing ilst; moov before later bytes may change only at identical byte length |

MP4 variable-length edits require moov at EOF: no chunk offsets move in that
profile. A front moov growth/shrink refuses before any write instead of rewriting
chunk tables. Bounded classic local sample/chunk/reference tables are checked;
this is not a general ISO-BMFF layout validator. Unrecognized bounded boxes remain
opaque. IFF, Ogg, APE, ASF and other formats are pending. Ordinary UTF16 ID3 target
editing is pending even though untouched UTF16 frames survive. Existing production
scalar workflows remain in place; these exclusions must be resolved or surfaced
intentionally before changing their writer.

## Preservation, bounds and refusal

Each intent binds actual container, selected tag block, ID3 revision, native target,
exact old state, raw selected-block SHA256 and current read policy. No display
separator is parsed to infer values. Missing, present-empty-member, repeated and
removed states stay distinct. Apply refuses stale whole-block evidence before
writing. Restore instead requires the current target's exact after-record bytes,
semantic state, policy and native context; unrelated current metadata may differ.

Receipts retain exact old raw target runs plus stable neighboring native identity
anchors. Restore preserves all current unselected records and finds the original
insertion gaps. Group-boundary anchors allow an unrelated contiguous native list
to change length. Missing, changed-count interior or ambiguous anchors refuse
before mutation. Opaque ID3 records without proved qualifiers use a conservative
body-bound identity; changes beside such anchors can refuse restoration.

The final private pure helper
`parse_target_record(&NativeTextContext, &NativeTextTarget, &[u8]) -> Result<Node>`
is used by **all three actual scanners**. It derives selection, key and values
from one complete raw record. A future child receipt codec must call this parser,
validate selected records and aggregate exact states/digests, and enforce context,
length/count/anchor bounds. It must never trust separately decoded `Node` fields.
The semantic capability helper is
`exact::validate_text_intent(context, target, old, new)`; target syntax validation
alone does not authorize typed or unsupported mutations. Restore's old/new direction
and absent historical targets need intentional codec validation rather than
blindly treating raw receipts as fresh user intent.

A 128MiB transient reservation and one job are admitted before parsing, with a
separate retained receipt lease before mutation. Metadata is capped at 16MiB,
records at 4096, native file size at 64GiB, container visits at 65536 and MP4 tree
depth at eight. Selected exact state uses the existing 1024-member/1MiB field
bounds. String ownership, raw records and receipt anchors are charged
conservatively; these reservations do not constitute a process RSS ceiling.
Metadata allocations are bounded; encoded media is never collected in a Vec.
Grow/shrink moves suffix bytes in 64KiB pieces with overlap-safe direction.
Prefix/suffix SHA256 comparisons establish unchanged bytes outside the edited
metadata, and readback compares every unselected native raw record.

Callback, cancellation and policy checks surround parse, hashing, resizing,
chunk writes and final verification. File size/mtime/native change-time witnesses
are revalidated after callbacks and write allowances, without adopting a new
baseline after a callback. Unix requires a read/write nonappend handle. Windows
uses FILE_BASIC_INFO change-time; other platforms without a supported witness
refuse. Linux runtime evidence does not verify Windows/macOS, coarse/network
filesystem metadata races, hostile concurrent file access or physical power loss.
The primitive assumes exclusive candidate ownership; it is not a replacement for
the protected artifact store's authority/evidence protocol.

## Executed verification

All commands used the existing environment and build cache:

```sh
source /mnt/Games/Development/tagrex/dev-env.sh
cargo test -p tagrex-core --test native_candidate_patch --locked -- --nocapture
cargo test -p tagrex-core --test native_candidate_patch independent_native_readback_and_encoded_packet_hashes --locked -- --ignored --nocapture
cargo test -p tagrex-core --test native_candidate_patch native_candidate_streaming_cost_probe --locked -- --ignored --nocapture
cargo test -p tagrex-core --test exact_read --test exact_read_adversarial --locked
cargo clippy -p tagrex-core --all-targets --all-features --locked -- -D warnings
rustfmt --edition 2021 --check --config skip_children=true crates/core/src/model/native_patch.rs crates/core/src/model/exact.rs crates/core/tests/native_candidate_patch.rs
```

The candidate suite passed **17 tests**, with two opt-in probes ignored in the
ordinary run and actually run separately. The exact read suite passed 17 with
two unrelated manual probes ignored; exact read adversarial passed four. Core
all-target/all-feature Clippy passed with warnings denied. Workspace `cargo fmt
--all -- --check` reported concurrent formatting differences in another worker's
`core/tests/move_staging_profile.rs`; this checkpoint did not edit that file.
Scoped formatting passed.

Evidence includes raw FLAC application metadata and comment spelling/order,
separator literals, case-distinct genres, empty members, Unicode, ID3 private
and UTF16 unselected frames, qualified comments, MP4 binary/UTF16/nonzero locale
items and full freeform namespaces. Exact restoration and reverse-receipt Redo
are checked independently of legacy display projection. Stale/unsupported inputs
are checked for byte-for-byte unchanged preflight refusal. One thousand bounded
deterministic malformed FLAC mutations assert no panic/admission leak and raw
unselected/audio preservation for accepted shapes. MP4 malformed counts and mixed
target qualifiers refuse without mutation.

Cancellation is injected at every observed operation checkpoint on a 210KB
suffix. Positive seven-byte short write allowances complete and restore exactly;
zero allowances and injected ENOSPC return no receipt. These simulate write
boundaries; they do not fill the machine disk. Candidate-changing callbacks and
isolated-process policy changes refuse. Actual simultaneous operations saturate
the shared reservation: the second candidate stays unchanged, then succeeds after
the first releases its resources. Success/error/cancel paths assert baseline
resource usage. These new APIs did not exist at baseline; the tests establish
behavior, not a claim that the unchanged production Executor regression is fixed.

The opt-in independent probe generates owned short FLAC, MP3, ADTS AAC and M4A
fixtures with ffmpeg. Mutagen **1.48.1** reads the edited Japanese title; ffprobe
encoded packet SHA256/size/duration records agree before/after/restore. The entire
file matches its original bytes after receipt restoration. No user media is used.
Fixtures clean up through their owning test guards.

The opt-in streaming probe appends a 64MiB suffix incrementally, changes the title
and restores it. On this Linux development machine, warm debug build, one sample:

| Operation | Elapsed |
| --- | ---: |
| Apply-like candidate growth | 79.77ms |
| Raw receipt restoration/shrink | 78.79ms |

Total bytes written were **134,217,889**, largest write **65,536**; full-file SHA256
returned to its baseline and resource charges returned to zero. This measures
candidate movement plus verification on the local warm filesystem, excludes
protected full-image copies/sync/journal and is not an end-to-end performance or
cold-cache guarantee. The current algorithm performs bounded linear media passes,
not a full SHA rescan per moved chunk.

Graph evidence used Tier2 `tagrex-backend-readiness`, generation
2026-11-18T12:48:55Z. Coverage reports native_patch/test untracked and exact/model
metadata_changed; actual checkout source was read instead. No graph completeness
claim is made.

## Integration gates still open

A new independent review must exercise candidate/receipt behavior before enabling
any production write. The parent owns the bounded durable receipt codec and
protected Store/journal/Executor integration. Persistent receipts must compare
semantic policy and raw/native evidence, then explicitly rebind a valid restarted
process generation; a historical numeric policy revision alone is insufficient.
History authority is separate from codec validity. Add held-file candidate access,
full durable intent/acknowledgement and bounded artifact retirement at that seam.

Keep the original Executor preservation canary unchanged until ordinary Apply
passes. Then prove reopened-journal selective Undo/Redo, unrelated post-Apply
metadata, recovery fault/process-kill boundaries, composed artwork/block changes,
large batches and platform behavior through the actual public coordinator. Do not
promote this receipt primitive or `executable()` before those integrations and
independent checks exist. No overall backend/UI readiness claim follows from this
checkpoint.
