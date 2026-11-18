# DeepSeek media integrity and outputs review

Date: 18 November 2026. Requested model: OpenCode Go / DeepSeek V4.1 Flash 1017.
Checkout: `/mnt/Games/Development/tagrex/native-field-foundations`, branch
`feat/native-field-foundations`, baseline `0554022c` including code checkpoint
`b657f65d`. Only the media-integrity worker's files were edited.

Scope: artwork header/decode/resize admission and cache, bounded export and text
encoding, plan accounting/digest inputs, and Apply/Undo integrity indexing,
interrupted recovery, copies/moves/sidecars and pruning. Stage -> review ->
Apply -> Undo, persisted history/settings, source bytes, recursive selected-root
semantics and authoritative full-file integrity proof are preserved. No UI,
player, staging, command or app source was modified. No user media, runtime
state, dev server or permissions prompt was touched. Authoritative checks were not
relaxed: whole-file SHA-256 fingerprints before/after block writes, stale-value
refusal, copy-ownership stamps and guarded output publication still gate Apply
and Undo.

Method: graph-first discovery with the parent's Tier 2 evidence, then actual
source reads for every owned file because the index generation (2026-11-13) is
stale. Review -> reproduce/measure -> bounded implementation -> regression tests
-> rereview. All writes used disposable fixtures under
`/mnt/Games/Development/tagrex/tmp`; the probe directory was removed after its
numbers were captured below.

## Parent review corrections (18 November)

The worker's original severity/evidence statements below are preserved as a
review record, not all accepted conclusions. Parent reproduction of the original
symlink scan shape on Linux returned false after 41 directory visits because
ELOOP stops link resolution. The claimed stack-abort from that self-cycle is
therefore overstated on this platform; link refusal/depth limits are defensive
hardening, not a reproduced Linux crash fix.

Parent found and fixed a separate higher-impact deletion race: files created
after only_junk returned could be deleted by remove_dir_all (or unconditional
remove_file). Cleanup now deletes only recognized ordinary junk files, recurses
conservatively, and uses remove_dir for empty children. A deterministic after-scan
insertion regression preserves both a direct new file and a nested new audio
file. This does not claim hostile path-swap resistance from descriptor-pinned I/O.

Parent also restored unreadable-row refusal for playlist aliases (otherwise the
optimization bypassed the previous refusal), extending the fixture across all
seven export format aliases. Analysis path admission now stops within the quota
while building storage and counts owned String and Vec capacities both on input
and in the retained registry. Relevant pruning/output tests and core/service-lib
Clippy passed; combined validation is still pending other workers.

## Confirmed and fixed

### 1. High - pruning could follow a directory symlink into unbounded recursion

- Trigger: a move/reorganize that prunes an emptied source folder which still
  contains a directory symlink, including a self-referential link
  (`loop -> .`) or a link to an ancestor.
- Location: `crates/core/src/plan.rs`, `clear_os_junk` / `only_junk`.
- Evidence: the old code classified each entry with `path.is_dir()`, which
  follows links, then recursed with no base case. A link to the directory being
  scanned makes the walk infinite; the process aborts on stack exhaustion
  instead of completing or refusing the prune. This is source-confirmed, not a
  measured production crash.
- User benefit: reorganizing a library that contains convenience/backup symlinks
  no longer risks terminating the app mid-Apply with an interrupted journal.
- Implementation: use `DirEntry::file_type()` (does not follow links), treat any
  symlink as a real entry and leave the folder intact, and cap recursion at 64
  levels. Junk-only folders without links are still cleaned and removed.
- Edge cases: the folder and its `.DS_Store`/`Thumbs.db` remain when a link is
  present; deleting the link itself is not attempted; dangling links were
  already treated as content. Directory pruning and its recorded undo intent
  are otherwise unchanged.
- Tests: `pruning_does_not_follow_a_directory_symlink_cycle`,
  `os_bookkeeping_still_clears_an_otherwise_empty_folder`, plus the existing
  pruning/root-alias and interrupted-pruning suites.
- Limits: the regression is Linux/Unix-only; Windows directory junctions/reparse
  points were not exercised. This is a bounded cleanup path, not a general
  symlink policy change.
- Route: reusable core fix for curated upstream `tagrex/tagrex:main`.

### 2. Medium - Undo and rollback performed quadratic integrity lookups

- Trigger: a large Rename/Copy/Undo batch or a rollback containing many ID3v2
  revisions or copy stamps.
- Location: `crates/core/src/plan.rs`: `restore_revision`, callers inside
  `Executor::undo_with_mode` and `Written::roll_back`; `ensure_untransfer_safe`
  (linear `copy_stamps.iter().find` per transfer); `pruning_candidates`
  (`Vec::contains` per parent).
- Evidence: the baseline had already indexed whole-block fingerprint lookups
  (`5f6cd3ab`) but left these three scans linear. A synthetic probe of the same
  lookup shapes measured worst-case `n` scans of `n` `PathBuf`s:

  ```text
  n=1000   linear 18.97 ms   indexed 0.035 ms
  n=10000  linear 1.62 s     indexed 0.334 ms
  n=50000  linear 39.72 s    indexed 2.18 ms
  ```

  The probe is a lookup microbenchmark, not an end-to-end Undo profile; on real
  files Undo is dominated by tag I/O. It establishes the removed quadratic
  comparison count only.
- User benefit: Undo and rollback latency stops growing quadratically with batch
  size; a 10k-file batch no longer spends seconds comparing paths for revision
  and copy-ownership checks.
- Implementation: build one borrowed `HashMap<&Path, Id3v2Revision>` and one
  `HashMap<&Path, &CopyStamp>` per Undo/rollback; report `batch.interrupted`
  separately so the legacy/no-stamp behavior is unchanged. Deduplicate pruning
  candidates with a `HashSet` while preserving first-seen order.
- Edge cases: first matching revision/stamp still wins for malformed duplicate
  paths; a missing revision remains a no-op; copies with no stamp follow the
  same legacy-vs-interrupted rule; pruning candidates are still sorted deepest
  first and filtered by `created`.
- Tests: whole `tagrex-core` suite, including executor, SQLite journal reopen,
  copy/sidecar ownership, interrupted pruning and block-fingerprint suites.
- Limits: no real-library end-to-end timing; no Windows/macOS filesystem
  measurement.
- Route: reusable core fix for curated upstream `main`.

### 3. Medium - every export read each selected file twice

- Trigger: any `exports.create` over a large selection, most visibly an
  ordinary M3U/playlist export.
- Location: `crates/service/src/outputs.rs`, `Service::export_files`.
- Evidence: `app.read_track_rows(&paths)` already reads every file once for
  library membership and playlist rows, then the exporter called
  `TagEngine::read` for every path again. Playlist formats never use the
  resulting `TrackFile` at all. A probe on 2,000 disposable tagged WAVs measured:

  ```text
  n=2000 first (kept) read = 82.9 ms
  removed second read      = 76.3 ms
  row -> TrackFile rebuild = 0.48 ms
  tags compared             = 14,000 vs 14,000 (identical)
  ```

  The benchmark measures the removed metadata pass and its replacement, not
  whole-export wall time.
- User benefit: large CSV/HTML/XML/report/CUE exports do roughly half the tag
  reads they used to; playlist exports avoid the extra pass entirely. The
  exported values also come from the same snapshot used for membership.
- Implementation: build the native export view from the already-read rows,
  pairing rows with the real `PathBuf` list (so file placeholders do not inherit
  the row's lossy display path) and inverting `TagField::to_storage_key`. Only
  non-playlist formats construct it. An unreadable row keeps the previous
  `read_failed` refusal.
- Edge cases: custom fields round-trip through `custom:`; unknown storage keys
  remain custom names; `TrackDto::unreadable` is refused exactly as
  `TagEngine::read` refused it; duplicate paths stay positional; ordered
  snapshots are preserved because `read_track_rows` preserves input order.
- Tests: `every_text_export_format_uses_the_shared_row_snapshot` (CSV, HTML,
  XML, report with `%filenameext%|%_codec%`, playlist, CUE) and
  `export_refuses_a_track_the_native_reader_cannot_read`, plus the existing
  publication/scope/symlink/recovery tests.
- Limits: `TrackDto.format` is a debug string; the local mapping mirrors the
  current `AudioFormat` variants. A future variant would be refused until the
  adapter is extended (cross-owner request below). The fixture is WAV only.
- Route: fork service implementation; reusable idea requires curation against
  upstream `main`.

### 4. Medium - sealed-draft budget ignored retained scope and identity maps

- Trigger: many drafts over very large path/field scopes, especially live
  (publish) drafts whose projected plan is filtered to nothing after sealing.
- Location: `crates/service/src/plans.rs`, `Service::seal`.
- Evidence: `size_bytes` counted only the JSON plan. `SealedPlan` also retains
  an owned `BTreeMap<String, Identity>` (one canonical `PathBuf` per changed and
  scoped path) and a `BTreeSet<StageKey>` (path + field per scope key). A live
  draft's plan is replaced with `filtered(..., false)` and its size recomputed,
  so the retained maps were the dominant unaccounted memory.
- User benefit: the 128 MiB draft budget now reflects what a draft actually
  keeps, closing a way for many large-scope drafts to bypass the intended bound.
- Implementation: `retained_overhead` adds a conservative saturating estimate
  (path/field lengths, `size_of::<Identity>()`, fixed node overhead) to the JSON
  size at both sealing points, including after live filtering.
- Edge cases: estimates are conservative, not an RSS cap; zero-scope/identity
  cases add nothing; plan JSON bytes and the existing 256-handle quota are
  unchanged; digest and authorization inputs are not touched.
- Tests: `draft_retention_includes_scope_and_identity_overhead`, the full
  service suite, and existing draft/composition/commit suites.
- Limits: exact allocator overhead is not modelled; no process-RSS measurement.
- Route: fork service accounting; compatible idea could be curated separately.

### 5. Low - analysis admission counted source paths, not retained strings

- Trigger: `analysis.duplicates`/`analysis.bpm` over paths containing invalid
  UTF-8 bytes.
- Location: `crates/service/src/outputs.rs`, `Service::duplicate_analysis`.
- Evidence: admission summed `OsStr::len()`, but the job retains
  `to_string_lossy` strings; each invalid byte becomes a three-byte replacement,
  so retained bytes could exceed the advertised 64 MiB path budget.
- User benefit: the analysis-registry bound covers the memory actually retained.
- Implementation: `retained_paths` converts once and sums the retained string
  lengths; admission and the stored job share the same owned strings.
- Tests: `analysis_admission_counts_the_retained_lossy_path_bytes` (Unix
  invalid-byte path) plus the existing duplicate-job admission/release test.
- Limits: Unix-only regression for the expansion case; the budget remains an
  accounting estimate.
- Route: fork service bound; reusable idea for curated upstream.

## Confirmed, deliberately not changed (deferred / cross-owner)

### Interrupted-batch recovery is blocked when no per-field outcome was persisted

A process that stops after some writes but before `UndoJournal::finish` leaves
the initial durable record: `interrupted = true`, `applied_values` empty. On
restart, `changed_since_applied` refuses any field that no longer reads as its
`old` value, and covers are likewise accepted only when already restored. This
is deliberate and covered by
`crates/core/tests/sqlite_journal.rs::an_interrupted_batch_can_be_undone_after_a_restart`
and `interrupted_field_recovery_at_each_tag_write_boundary`: the tests restore a
field by hand and then Undo succeeds. Service recovery guidance
(`crates/service/src/recovery.rs`) offers `undo_correlated` for such a batch, but
that strict Undo cannot complete until the values are restored externally.

I reproduced the gap with a temporary test (apply, clear `applied_values`, mark
interrupted, Undo): it failed with `ChangedSinceApplied`. Accepting the plan's
recorded `new` value as the second safe state made it pass, but that contradicts
the deliberate fail-closed tests above, so I reverted the change and did not
edit recovery/service policy outside this worker's ownership.

Proposed exact change for the parent/owner: add an explicit, user-confirmed
recovery path that permits Undo of an interrupted batch when each tracked field,
cover and block reads as either the plan's `old` or `new` state. That needs (a)
a core `UndoMode`/flag used only by that path so ordinary Undo keeps refusing
ambiguous values, (b) `crates/service/src/recovery.rs` to require the existing
proof plus a human confirmation before passing it, and (c) updating the two
sqlite-journal tests to assert the new confirmed behavior while keeping the
refusal for a value that is neither `old` nor `new`. Copies without an observed
stamp and blocks without a post-write fingerprint remain fail-closed unless the
same explicit policy is defined for them.

## Rejected or no-defect hypotheses

- **Artwork cache correctness/resource admission.** Reviewed
  `cover.rs`/`cover/resources.rs`: content+max+quality keys, clamped quality,
  LRU byte/entry bounds, caller-owned kind/description, header-only oversize
  detection, malformed/zero dimensions, encoded sink refusal before growth,
  row-wise PNG fallback and 512 MP work cap. Existing tests cover these; no
  defect found. Kept the global processing gate because it is the aggregate
  working-memory bound, not a UI lock.
- **Re-encode property loss.** JPEG re-encoding intentionally changes MIME and
  drops EXIF orientation/ICC profiles; viewers expecting rotation or wide-gamut
  colour may see a change. Fixing needs a metadata-preserving decode/encode
  pipeline and format-specific tests, so it is deferred rather than guessed.
- **Bounded export escaping.** Reviewed the writer sink, HTML/XML/CSV/CUE/M3U
  escaping, formula neutralisation and per-file/aggregate budget arithmetic; no
  under-allocation or partial-publication path found. Existing limit tests
  passed.
- **M3U/CUE line breaks in tag values.** A title containing CR/LF can still
  produce extra lines because these formats are line-oriented. Any sanitisation
  changes exported text, so it needs an explicit product choice.
- **`text_codec` direct `Codepage(UTF_16*)` construction.** The public enum can
  bypass `from_label` for decode BOM handling; all in-tree callers use
  `from_label`/known variants, so this is an API-hardening idea, not a
  reachable defect here.
- **Export mutation/app lock span.** Rendering and publication still hold the
  mutation and app guards, which keeps an export consistent with a concurrent
  Apply. Shortening it would trade that consistency for latency; no measured
  stall justified the change.

## Cross-owner requests

- `crates/commands`: `TrackDto.format` should expose a typed `AudioFormat` (or a
  stable parse helper) instead of the debug string the service currently maps
  back. Also `render_columns` and `playlist_name` call
  `export::lenient_tags` and render masks against the full cloned map; using
  `lenient_mask_tags`/a bounded sink would avoid cloning large values per column
  per file. Not edited here.
- `crates/service/src/recovery.rs` + core Undo policy: see the explicit
  interrupted-recovery path above.
- No change was made to `crates/commands`, `app`, `player`, `staging`,
  shared tests, README or UI source.

## Exact changed files

- `crates/core/src/plan.rs`
- `crates/service/src/outputs.rs`
- `crates/service/src/plans.rs`
- `docs/performance/deepseek-media-integrity-review.md` (this report)

## Verification evidence

Run from the checkout, with the existing toolchain loaded from
`/mnt/Games/Development/tagrex/dev-env.sh`, serialized on
`/mnt/Games/Development/tagrex/tmp/deepseek-rust-review-build.lock`:

- `cargo test -p tagrex-core`: **443 passed, 0 failed, 8 ignored** across the
  core binaries; includes the new pruning tests and all existing
  executor/journal/Apply/Undo/recovery suites.
- `cargo test -p tagrex-service`: **148 passed, 0 failed, 0 ignored** in the lib
  target (the count includes concurrent staging-worker tests that landed during
  this run); this worker's new tests are the export-row, unreadable-track,
  retained-path and draft-overhead cases, and all existing service
  plan/output/recovery tests passed.
- `cargo clippy -p tagrex-core --all-targets -- -D warnings` and
  `cargo clippy -p tagrex-service --lib -- -D warnings`: passed. A later
  `--all-targets` service run was blocked by three
  `field_reassign_with_default` diagnostics in another worker's in-progress
  `crates/service/src/staging.rs` tests (e.g. line 2613), not by this change;
  the parent should resolve that before the combined all-target gate.
- `rustfmt --edition 2021 --check` on the three changed Rust files: passed.
- Independent audio check: `cargo test -p tagrex-commands --lib
  redo_preserves_flac_audio_with_independent_readback -- --ignored --nocapture`
  passed; Apply/Undo/Redo of a disposable FLAC kept its audio as verified by
  ffprobe. The new code paths delete only OS bookkeeping after a move or rebuild
  an in-memory export view; none rewrites audio bytes.
- Probe run (disposable, removed afterwards): 2,000 tagged WAV export read
  component and the lookup-shape timings quoted above.

A transient `cargo test -p tagrex-service` attempt failed to compile because
another worker was mid-edit in `crates/commands/src/query.rs`; the run above was
repeated after that file compiled. No combined workspace test, whole-workspace
Clippy/formatting, hosted CI, Windows/macOS or package check was claimed here;
the parent must run the combined gate.

## Limits

No user library or running app was profiled. The export and lookup numbers are
component microbenchmarks on disposable fixtures, not app-level speedup
percentages; no process-RSS bound is claimed. Windows/macOS filesystem and
symlink semantics are unverified. Artwork orientation/ICC preservation and
interrupted-recovery policy remain open as stated. Reusable core fixes are
upstream `main` candidates after curation; fork service/storage accounting needs
separate compatibility review. No PR, merge, release or enterprise certification
is claimed.
