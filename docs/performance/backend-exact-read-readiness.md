# Exact metadata representation and native reads

Implementation checkpoint: `feat/native-field-foundations`, following baseline
`9da71249` and the uncommitted shared-readiness work, 18 November 2026. This is
Wave 2's first dependency component: exact read/state/intent preparation, not
exact writes, stage migration, persistent Undo migration or UI feature completion.
No production writer, Executor, journal or command/service DTO was changed.

## API and ownership

- `TagEngine::read_exact(path)` and `read_exact_with_cancel(path, callback)` return
  `ExactTrackRead`. Borrowed `legacy()` and `text()` accessors retain ownership
  with their resource lease; no public detach/clone operation silently separates
  those owners from accounting. Caller-created clones are separate ownership.
- `read_with_props` and exact reads use the same private `read_selected` helper:
  actual container probing, properties, selected-block priority/fallback and
  legacy scalar projection remain on the existing path. Exact native metadata
  is captured before that projection. It is never reconstructed from joined
  scalar strings or split on punctuation.
- `ExactTextState::absent()` differs from present `[""]` and `["", ""]`.
  Present empty lists, NUL inside a member, excessive values and excessive text
  are rejected by validated constructors/TryFrom. Ordered punctuation, whitespace,
  Unicode, duplicates and empty members remain literal. `FieldValues::append`
  validates limits before mutation and avoids repeatedly cloning earlier values.
- Native identities retain original Vorbis spelling; ID3 frame, description,
  language and supported text encoding; MP4 fourcc or complete `(mean,name)`,
  UTF-8/UTF-16 data kind and locale. Display-field resolution refuses competing
  identities. Exact native target resolution refuses duplicate frames/case
  collisions and mismatched representations instead of selecting a winner.
- Snapshots capture actual container, selected block/revision, adapter version,
  raw selected-block digest, policy generation and actual interpretation settings
  (priority, separator, codepage, ID3 write preference). Policy changes during a
  read refuse the result. File replacement/change observations also refuse it.
- `prepare_patch(target, new_state)` validates an explicit semantic intent and
  captures exact old/new state/context without writing. Typed/numeric fields,
  unsupported representation changes and nontext targets are refused. Prepared
  patches hold their own retained reservation and report `executable() == false`.
  Their integration with the existing mutation/journal path is still required.

Core remains serialization-agnostic. Command-layer strict versioned DTO decoding
must call these constructors in the next integration component; no permissive
serde envelope or persisted-state migration is introduced here.

## Supported bounded read subsets

| Native representation | Implemented exact read subset | Explicit refusal/unavailable boundary |
| --- | --- | --- |
| FLAC Vorbis Comments | Raw UTF-8 records, duplicate/order/empty preservation, literal punctuation and original name spelling | Invalid UTF-8/key syntax/NUL text, competing case variants and quotas; binary picture fields are unavailable as text |
| ID3v2.4 | Unflagged frames in leading tags or ordinary RIFF/AIFF ID3 chunks; UTF-8 and Latin-1 ordinary text; structural NUL-separated text values including trailing empties; TXXX names; COMM/USLT language+description | UTF-16 text currently unavailable; v2.2, unsynchronisation, compression, extended headers/footer and nonzero frame flags refused; private/binary/locator items are unavailable as text |
| ID3v2.3 | Same raw envelopes, Latin-1 scalar text with actual configured codepage interpretation | No arbitrary structural list claim; NUL-bearing scalar values are unavailable instead of silently trimming terminators; unsupported encodings/layouts refused |
| MP4 ilst | Ordinary nonfragmented bounded metadata trees; exact UTF-8 and strict UTF-16BE text, repeated values, empty values, full freeform namespaces, data kinds and locale | Malformed UTF-16, ambiguous freeform names, unknown data/layout, encrypted sample descriptions, compressed/fragmented movie profiles and quotas; binary/numeric/picture atoms are not editable text |
| Other selected blocks/layouts | Legacy read/selected-block context remains visible when the input profile can be established | RIFF INFO/AIFF text blocks, Ogg packet assembly and APE tail exact adapters are not implemented in this component. Unsupported native absence is never reported as known absence |

The installed Lofty 0.25.5 decoder trims terminal NULs in generic UTF-8/UTF-16
helpers used by more than ID3. Therefore this component uses bounded raw proof
for the admitted subsets instead of treating reconstructed concrete strings as
universally exact. The raw readers are read-only. Unknown/private metadata and
encoded audio remain byte-identical after these API calls; this does not prove
preservation by the existing scalar writer.

Read selection follows current Tagnetic behaviour. For example, a textless ID3
block with a meaningful RIFF title can select RIFF INFO; exact context then marks
that block unsupported, rather than pretending the ID3 text inventory was chosen.

## Bounds and consistency

Raw metadata admission is limited to 16 MiB, at most 4,096 native items/atoms per
inventory, 1,024 values and 1 MiB text per field, 1,024-byte native names, 8 MiB of
charged exact state and 24 MiB estimated combined legacy/exact result ownership.
Box/chunk visits stop at 65,536 and nesting at six levels. Known MP4 sample-table
counts are checked against their actual parent extents before compatibility
parsing; malformed counts cannot request giant metadata tables through this path.
Audio extents are sought over, so file length is not capped at metadata size.
Metadata-heavy long recordings outside the admitted profile fail explicitly.

Each active read reserves one shared job and 64 MiB of transient processing
headroom before native capture. Exact snapshot/result and prepared-intent owners
hold retained reservations. These are participating estimates, not an allocator
or RSS guarantee; a fixed conservative transient reservation limits concurrency
before all eight job slots can necessarily fit. Admission is nonblocking and Busy
is an explicit retryable condition. No partial result is returned on refusal,
policy/file change or cancellation. Filesystem calls and an already running
compatibility parser are not claimed interruptible.

Unix opens use no-follow/nonblocking descriptors and reject nonregular files;
identity includes device/inode/change time. Windows source identity uses handle
volume/file ID and native change time; missing API identity is refused, not
accepted by size/mtime fallback. Windows runtime/filesystem and macOS checks
remain unverified here. These read witnesses are not write authorization: Apply
must re-read exact baselines and use its authoritative scoped mutation boundary.

## Independent source-consistency correction

The independent reviewer demonstrated P1 mixed-source publication: raw capture
read `First`, a fixture-owned parent link selected `Other` during legacy parsing,
and restoring the link defeated the final identity witness. The unchanged
`exact_read_adversarial` canary failed before the correction.

`raw::Input` now owns one regular-file handle from initial capture. A scoped,
thread-local authority routes all synchronous compatibility-reader passes for
the original logical path through clones of that held handle. It preserves the
original filename/extension for the parser ladder, rather than substituting a
descriptor pathname. Native scans complete before the serial compatibility
passes; every compatibility open resets the shared seek cursor. No parser
handle escapes this read. Nested scopes restore their predecessor on success,
error and panic. This is a private read authority, not a new write capability.

Both the held source and the current path are checked for identity/change before
publication. An observed change refuses the result; unchanged witnesses alone
are not used as proof that independently reopened parse phases agreed. Repair
fallbacks that copy a whole file now use the held source too. During an exact
read those fallbacks reject inputs over 64 MiB and bound the read itself if the
file grows after the length check. Ordinary legacy reads keep their fallback.
This does not instrument every allocation inside the dependency parser.

Additional public-API stress varies the selected parent at every observed
checkpoint across independently built FLAC, ID3-in-WAV and MP4 fixtures. Every
accepted result must have matching native and compatibility titles; refusals
must release admission. Other probes exercise nested successful/cancelled/
panicking exact reads and confirm the outer authority and charges recover, and
an MP3 with 1,035 bytes of junk plus a misleading AAC sync must still take its
original extension-directed MP3 fallback. Files remain byte-identical.

## Verification

Tests use task-owned synthetic PCM/FLAC/ID3 envelopes and the repository's tiny
AAC/M4A fixture. Raw fixtures are deliberately built without a tag writer that
could normalize values before reading. Paths live under the reused dev-env
TMPDIR, and owners remove generated fixtures on scope exit. No live profile,
existing Undo, running app or user media was touched.

```sh
source /mnt/Games/Development/tagrex/dev-env.sh
cargo test -p tagrex-core --test exact_read -- --nocapture
cargo test -p tagrex-core --test exact_read independent_native -- --ignored --nocapture
cargo test -p tagrex-core --test exact_read native_exact_read_scale_probe -- --ignored --nocapture
cargo test -p tagrex-core --lib
cargo test -p tagrex-core --test tag_engine
cargo test -p tagrex-core --test lofty_canaries
cargo clippy -p tagrex-core --all-targets --all-features -- -D warnings
```

The native read integration run passed 14 tests, with two manual probes ignored
by the default run. The independent Mutagen 1.48.1/ffprobe/raw-envelope probe was
run explicitly and passed for FLAC, MP3 and M4A. It checks arbitrary native text,
namespace values and ID3 raw trailing empties independently; encoded packet
hash output and full file bytes remain unchanged. A fixture must produce actual
ffprobe packets, so comparing two empty outputs cannot satisfy the check.

Stress conditions include twelve simultaneous admitted/refused reads with forced
overlap, bounded shared usage, exact native value agreement and clean retry;
policy changes during capture and before intent preparation; file replacement;
nonregular/final-link refusal; malformed Unicode, sizes/counts, sparse oversized
ID3, excessive lists/snapshot size, deeply nested MP4 and 65,537-atom input.
Single-operation success cases reserve test capacity separately from the explicit
concurrency stress; they do not weaken or bypass production admission limits.
An initial unrestricted parallel success-case run correctly encountered Busy,
which motivated that separation, not a higher production resource ceiling.

The manual scale probe generated 1,000 disposable files per format and measured
three native passes for 100 and 1,000 files. At 1,000 files, median legacy/exact
milliseconds were approximately FLAC 35.824/51.751, WAV 32.150/45.680 and M4A
53.293/80.134. Every exact native result was checked and reservations returned to
baseline. File creation and preceding passes warm the filesystem; these are not
cold-disk, UI, IPC, Apply or journal timings. Exact reads intentionally do more
work; no speedup is claimed. Cache exact+legacy together in the next integration
rather than adding repeated per-tool reads or ignoring the measured extra cost.

A production-library Clippy gate passed during initial read integration. Later
all-target attempts were temporarily blocked by other workers' in-progress
artwork/recovery lints; the final scoped results are recorded after those edits
settle. Parent combined and independent recheck gates remain required.

## Required next integration and known write blocker

The parent's `metadata_preservation_recheck` demonstrated an existing scalar
writer defect: a Title-only Executor update changes untouched ARTIST `["A; B"]`
to `["A","B"]`, drops an empty custom member and collapses case-distinct genres.
This component neither fixes nor weakens that canary. New exact reads expose
those distinctions but do not make current Apply safe for arbitrary repeated
fields. Do not mark general multiple-value editing complete or expose its new
controls on the strength of this checkpoint.

The next dependency component must:

1. Patch only explicitly touched native identities; preserve every untouched
   native value, namespace, qualifier, private/binary record and encoded payload.
   Rebuilding a whole tag from a legacy scalar map is insufficient.
2. Add strict exact envelopes to selected command/stage atoms, digests, plan
   conversion, rebasing and journaled outcomes. Preserve old scalar history's
   meaning; never infer lists from punctuation or backfill old journal values.
3. Route exact mutation through the existing Executor and recovery seam. Re-read
   old exact state at Apply, record observed exact outcomes from disk, and verify
   restart/Undo/Redo and interrupted failure behaviour before enabling execution.
4. Complete raw/profile adapters, immutable policy integration across Apply and
   format-specific representability/secondary-block rules. Read availability is
   separate from write support, and unknown inputs must retain explicit refusals.
5. Cache the immutable exact+legacy read together with correct file/policy identity
   and complete retained accounting, then expose capabilities/diagnostics in UI.

### Native writer reuse and durable mutation boundary

The selected scalar writer remains a separate component; no new writer or plan
call was introduced while closing the read finding. Preserve ordinary supported
FLAC/MP3/IFF/MP4/Ogg/APE scalar workflows when developing it. A blanket refusal
for all not-yet-migrated formats is not an acceptable completion condition.

Actual Lofty 0.25.5 source inspection found the following boundaries:

| Representation | Existing code that can be reused | Exact-preservation boundary |
| --- | --- | --- |
| FLAC/Vorbis comments | Current field/key mapping, validation, generation of touched values, container/properties parser | `ogg/tag/write.rs::create_comments` skips every empty value even from a concrete tag. Generic conversion also changes aliases/vendor and drops custom keys. Raw untouched comment records must survive; a whole concrete rewrite is insufficient. |
| Leading ID3 and IFF ID3 chunks | Parser ladder, field/frame mapping, typed validation, generated touched-frame payloads, explicit version conversion, current IFF chunk handling | Text decoders trim terminators; concrete serialization can normalize encodings/version and reconstruct lists. Preserve raw untouched frames, qualifiers, ordering, private frames and container chunks. Extended/flagged/unsynchronized profiles need explicit proof or refusal; the read adapter does not certify their writes. |
| MP4 ilst | Touched-field mapping, typed atom generation, established structural relocation/chunk-offset handling where independently verified | `ilst/write.rs::write_data` emits locale zero; UTF-16 serialization passes `text.as_bytes()` to the writer, so parsed UTF-16 is not a proven exact round trip. Preserve original namespace, raw data kind/locale/payload/order of untouched atoms. Concrete `Ilst` alone is insufficient. |
| Ogg/APE and secondary blocks | Existing guarded format-specific routes and compatibility behavior | Exact packet/tail adapters are still missing here. Add raw/profile proof and format fixtures before claiming untouched-value preservation; do not extrapolate the FLAC adapter to these containers. |

An ordinary scalar plan must pass its explicit applied/non-kept field list into
the selected native mutation path. It must not infer touched fields by comparing
a normalized whole `TagMap`. Keep broad explicit `TagEngine::write` semantics
separate. Selective Undo must restore only those fields so unrelated edits made
after Apply survive; exact stage/journal state is still needed for lossless
Undo of touched repeated values.

Raw splicing or calling the current backend writer can shift audio while growing
metadata. Metadata-only rollback bytes cannot recover a torn shift. Before
introducing that path, establish durable bounded-RAM prewrite evidence (for
example a validated disk-backed full-source backup with explicit states) through
Executor's current mutation authority. Preserve mode/owner, hardlinks and xattrs:
replacing the original with a renamed temporary candidate changes that contract.
On ambiguous recovery, retain both evidence and current file and refuse automatic
overwrite of possible foreign edits. These are implementation requirements for
the next component, not guarantees supplied by this read-only checkpoint.

Reusable core changes target a curated upstream `tagrex/tagrex:main` contribution
after parent verification. Exact fork storage/service compatibility remains a
separate migration. No commit, PR, merge, installer or full-roadmap completion is
claimed by this worker checkpoint.

Graph discovery initially used the stale other-checkout project; relied-on source
was read directly. The parent then refreshed `tagrex-backend-readiness` rooted in
this actual checkout (generation 2026-11-18T12:48:55Z). Exact model/values/raw/test
and relevant existing fixture-source paths were checked with metadata_match at
that snapshot. Subsequent owned edits still require source/freshness checks in
the final review; no exhaustive codebase coverage claim is inferred.


Final source-consistency correction checks (18 November 2026), using the reused
dev environment:

```sh
cargo test -p tagrex-core --test exact_read -- --nocapture
# 17 passed; 2 manual probes ignored
cargo test -p tagrex-core --test exact_read_adversarial -- --nocapture
# 4 passed, including the unchanged independently authored P1 canary
cargo test -p tagrex-core --test exact_read independent_native -- --ignored --nocapture
# 1 passed: Mutagen 1.48.1, ffprobe packet checks and independent raw checks
cargo test -p tagrex-core --lib --quiet
# 247 passed; 1 ignored
cargo test -p tagrex-core --test tag_engine --quiet
# 110 passed
cargo test -p tagrex-core --test lofty_canaries --quiet
# 2 passed
cargo clippy -p tagrex-core --all-targets --all-features -- -D warnings
# passed
rustfmt --edition 2021 --check --config skip_children=true crates/core/src/model.rs
rustfmt --edition 2021 --check crates/core/src/model/exact.rs crates/core/src/model/exact/raw.rs crates/core/src/model/exact/source.rs crates/core/src/values.rs crates/core/tests/exact_read.rs
git diff --check -- crates/core/src/model.rs crates/core/src/model/exact.rs crates/core/src/model/exact/raw.rs crates/core/src/model/exact/source.rs crates/core/src/values.rs crates/core/tests/exact_read.rs docs/performance/backend-exact-read-readiness.md
# formatting and whitespace passed
cargo test -p tagrex-core --test metadata_preservation_recheck -- --nocapture
# remains 0 passed / 1 failed with the original untouched native values assertion
```

The scalar writer still changes ARTIST `["A; B"]` to `["A", "B"]`, CUSTOM
`["", "尾"]` to `["尾"]`, and GENRE `["House", "house"]` to `["House"]`.
This intentionally unweakened acceptance blocker prevents a full core-green or
write-preservation claim. Source-consistency checks alone cannot clear it.
Intermediate final-check attempts also encountered recovery's temporary missing
`plan::open_bound_entry`; no recovery code was changed here, and the commands
above were rerun after that component compiled. Fresh independent review of the
held-source correction and the parent's combined gate remain outstanding.


## Held-file integration seam and compatibility expansion follow-up

`TagEngine::read_exact_file(File, logical_path)` and its cancellation variant now
consume an already held regular file. The logical path supplies the returned
label and parser extension hint; it is never reopened by this request and need
not exist. The caller owns root/path authorization and must check that binding
again before a later write. Other clones must not concurrently seek the shared
file cursor. This read-only result grants no mutation capability.

Both path and file entry points use the same private policy, compatibility,
native-state and publication pipeline. Both revalidate the held object; the
path API additionally preserves its existing named-path witness. One job and
64 MiB work admission precede scanning. Cancellation, panic and nested public
requests release or restore their own source scopes and resource owners.

Seven author tests cover foreign/missing labels, exact empty and literal values,
nested same-label requests, every cancellation checkpoint, unwinding, late held
mutation, denied admission before cursor IO, nonregular/write-only handles and
oversized labels. These pass alongside the existing 17 native read tests, four
adversarial tests and eight independent source-isolation tests. Two existing
manual native-read probes remain outside the default run. The [fresh held-file review](backend-held-exact-read-independent-review.md)
passes eight new tests and a 44-test combined reader gate. It found and retained
two final-callback regressions: the last cancellation callback could change
source or policy after their validation. That callback now runs before the final
policy and held-file checks; both original probes and a named-reader canary pass.

The separate [compatibility expansion review](backend-exact-read-expansion-review.md)
found ID3 list/scalar and MP4 UTF-16 conversions allocating beyond the 64 MiB
reservation before later native rejection. Early envelope validation now checks
text member counts and raw/decoded field lengths and aggregate text before
compatibility conversion. Configured ID3 codepages are measured by an incremental
512-byte decoder; default Latin-1 counting allocates no expanded string. MP4
UTF-16 decoded length is checked without constructing the string. UTF-16 ID3 is
still diagnostic-only; conservative decoded-size limits may refuse larger
unavailable fields. No resource ceiling was increased.

Fourteen independently authored allocation/boundary probes pass, including the
original dense/scalar failures, valid 1 MiB text boundaries, near-8 MiB text,
14 MiB opaque data and codepage output-buffer boundaries. This bounds those
Linux fixtures, not every dependency allocation or whole-process RSS. Scoped
all-features Clippy passes. Current held-reader evidence is retained in
`tmp/backend-roadmap-query-art-gate/held-exact-reader-expansion.log` and
`held-exact-reader-clippy.log` beneath the shared tagrex directory.

The ordinary Executor preservation canary remains an open P1. These reader
changes do not integrate native receipts into Apply, persistent Undo or staging.


Final orchestrator reader/writer/codec compatibility gate on the corrected
callback ordering: **100 passed, 0 failed, 6 existing optional tests ignored**
across ten integration binaries. The subprocess's own one-test summary is not
counted a second time. Scoped all-features Clippy also passed. This gate covers
read-only capture and disposable native candidate/receipt work, not ordinary
Executor preservation or production journal integration.


The six optional read/writer/receipt probes were subsequently run explicitly on
the same corrected reader source and all passed. They include independent
Mutagen 1.48.1/raw metadata/ffprobe packet checks for actual FLAC/MP3/M4A,
1,000-file-per-format exact-read observations, qualified/namespaced native text
and 64 MiB suffix streaming. Evidence is
`tmp/backend-roadmap-query-art-gate/native-reader-writer-manual-recheck.log`.
This supplies those six local manual results; native platform, production Apply
and journal-integration gates remain open.
