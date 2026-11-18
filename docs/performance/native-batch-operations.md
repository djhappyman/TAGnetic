# Native batch checkpoint: verification and review

Date: 17 November 2026. Checkout: `feat/native-field-foundations`, based on
foundation commit `2a9341e9` and fork main `23286e0e`. Development dependencies,
toolchain and build cache were reused. No running application was restarted and
no user audio, library profile or existing Undo history was used for test writes.

## Scope and completion condition

The scope is the shared native plan boundary: field extraction, file-matched
text imports, confined text sidecars, track/disc counters, ordered-value
foundations and split/merge/deduplication for the four existing list fields.
Owned files are the Rust modules and their tests, the README roadmap and the
two native-batch guide/evidence documents. No frontend source is changed.

Completion requires current core, command and service suites, Clippy with denied
warnings, formatting, a desktop Rust check, independent audio readback and a
bounded performance probe. The README distinguishes backend completion from UI
completion. General arbitrary repeated-field editing is explicitly partial.

Reusable core and native command changes are candidates for a curated upstream
`tagrex/tagrex:main` contribution. Service/MCP changes need fork compatibility
review; future shared UI integration follows upstream `next`. The checkpoint is
local and dependent on the existing foundation; no upstream PR, merge, hosted CI
or release is claimed. A curated upstream change must preserve its journal and
waveform compatibility boundaries described in `docs/WORKFLOW.md`.

## Verification

Run from the implementation checkout with the existing development environment:

```sh
source /mnt/Games/Development/tagrex/dev-env.sh
cargo fmt --all --check
cargo test -p tagrex-core -p tagrex-commands -p tagrex-service
cargo clippy -p tagrex-core -p tagrex-commands -p tagrex-service --all-targets --all-features -- -D warnings
npm run build
cargo check -p tagrex
cargo test -p tagrex-commands --lib native_batch_independent_audio_roundtrip -- --ignored --nocapture
cargo test -p tagrex-commands --lib native_batch_benchmark -- --ignored --nocapture
```

The complete run passed 894 tests across library, integration and documentation
suites; 15 manual or fixture-dependent tests were ignored by that normal run.
The library suites passed 330 command tests, 213 core tests and 133 service
tests. The two new ignored probes were also run explicitly and passed.

The normal suites cover preflight on empty selections, overlapping extraction
destinations, skipped matches, custom names, locks, staged sources, baseline
staleness, cancellation, unrelated staged/structural changes, duplicate paths,
counter overflow, release/disc resets, automatic totals, vinyl preservation,
invalid encodings, confined paths, symlink refusal, value order and ambiguous
legacy list conversions. Service checks include explicit no-op reverts, shared
stage composition, diagnostic retention, input types and sidecar read grants.

The desktop build is checked against its normal frontend output. Building that
output verifies packaging input; no new desktop controls or browser interaction
tests are claimed for this Rust-only checkpoint. Verification here is Linux
evidence. Windows and macOS runtime checks remain pending.

## Independent audio checks

The manual probe generates tiny synthetic MP3, FLAC and M4A streams with FFmpeg.
It combines extraction, list splitting/deduplication, filename-matched album
import, multiline text sidecars and track/disc numbering into one staged plan.

For all three formats, preview leaves file bytes unchanged. FFprobe independently
verifies saved title, album and multiline comments. Mutagen verifies two physical
artist values, rather than merely reading our joined table string. Copy-mode
FFmpeg packet hashes remain unchanged after Apply and Undo. FFprobe's original
metadata is restored after Undo; Vorbis field names are compared
case-insensitively, consistent with the existing writer's normalization.

These checks passed for the generated fixtures. They do not establish behavior
for every container variant, damaged file or third-party player. Existing native
artwork-preservation suites also run, but this checkpoint adds no artwork tools.

## Performance probe

Machine: Linux x86_64, Intel Core i7-12700K, 20 logical CPUs. Cargo uses the
repository's existing test/development optimization settings. The probe creates
10,000 disposable tiny tagged FLAC copies. It imports rows in reverse order by
filename, then numbers the explicitly ordered selection over the imported plan.
It asserts file count and correct row association.

Measured first preview / median of five warm previews / warm maximum:

- 100 files: 5.426 ms / 1.055 ms / 1.138 ms.
- 1,000 files: 46.851 ms / 10.737 ms / 11.184 ms.
- 10,000 files: 473.910 ms / 129.246 ms / 133.016 ms.

Smaller cases warm overlapping files before larger cases. First preview is not a
cold-disk claim. The measurement includes native import parsing, index lookup,
metadata cache access, numbering, staged-plan composition and validation. It
excludes IPC, rendering, Apply and journal writes. Work is linear in selected
files and input rows apart from each file's own fields; there is no per-row scan
of the whole selection.

## Review findings resolved

- Live draft projection discarded import and lock diagnostics. The service now
  retains bounded diagnostic metadata on the owning handle and includes it in
  live responses, without retaining complete duplicate plan payloads.
- A no-op replacement could leave an earlier pending value alive. Native imports
  and numbering now declare explicit field intent even when the new result
  equals the saved value; regression checks preserve unrelated pending edits.
- Permissive Unicode decoding could introduce replacement characters into
  imported text. UTF-8 and UTF-16 decoding now reject invalid input.
- Duplicate extraction destinations could silently replace earlier captures.
  New batch APIs reject these patterns before reading files.
- A merge result containing the configured list separator would be re-split by
  the current writer. The ordered-value adapter rejects that conversion, empty
  values and meaningful edge whitespace. Only an explicit split trims pieces.
- Lossy path-string conversion could make preview and Apply address different
  names. Native batch APIs refuse non-Unicode paths at their boundary.

The review also checked checked counter arithmetic, independent counter groups,
relative-input confinement, cancellation and metadata/output size limits. No
unresolved blocking finding is recorded within this checkpoint's stated scope.
That is a scoped review result, not a guarantee of bug-free code.

## Limits and remaining Rust work

The new `FieldValues` type preserves an ordered list in memory, but most existing
tag storage, shared-stage atoms and journal values are still scalar strings.
General repeated custom fields require a coordinated, backward-compatible
container/stage/journal migration. They are not enabled by changing the list of
four allowed fields. This remains unchecked in the roadmap.

The larger roadmap also still needs separate native implementations for advanced
artwork processing and complete picture categories, playlist loading/custom
partitioning, grouped export templates and encodings, broader field mappings,
additional supported containers and chapter editing. Complete recipes crossing
metadata, paths, artwork and output publication need their own composition and
failure rules. These are not claimed complete by this batch checkpoint.

New field batches accept at most 100,000 selected entries, 8 MiB of effective
text per processed file and 64 MiB of changed text including Undo values. Text
input files and pasted imports are capped at 1 MiB; sidecar assignments are
capped at 64 MiB per batch. Value lists are capped at 1,024 items and 1 MiB.
Existing pattern compilation/rendering limits also apply. Limit errors and
cancellation publish no partial plan. Documentation of the intended behavior
and official Mp3tag source links is in the [operation guide](../guide/native-batch-operations.md).
