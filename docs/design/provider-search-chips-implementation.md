# Provider search chips implementation

Scope: one field/value filter per source, accepted by suggestion then a compact
value entry. Enter or OK commits a chip displaying the value; the shared field
hue remains on the search box border even while blurred. A second confirmed
field replaces the old one. Clicking the chip edits/removes it. Label identity
preview and freedb/GnuDB are deferred by Mike. All edits stay in personal-app on
main, with no new branch/worktree, per his explicit instruction.

The original multi-chip plan is superseded by the single-filter interaction.
Current sources have independent in-session text/filter drafts; switching back
restores them. Plain text remains separate from the filter and pagination keeps
the submitted query. Changing source/filter invalidates in-flight results.

Native capabilities and parameter names are defined once in
`crates/core/src/search-fields.tsv`, consumed by Rust and Vite. The native command
and adapters reject unsupported fields, empty/invalid values and a second filter.
Legacy text/query DTO callers remain compatible. Existing Medium selection becomes
an editable chip and cannot act as a hidden second filter.

- Discogs: Artist, Release title, Label, Catalogue number, Medium, Classification.
- MusicBrainz: those shared fields (classification uses Primary/Secondary type),
  Barcode, Country, Release date, Packaging, Language and Track count.
- Beatport release search: Artist, Label, UPC, Publish date, Pre-order date.
  These map to documented release-search parameters; no title-only/BPM/Key filter
  is fabricated from broader free text or track-only facets.
- Bandcamp/Soundeo: general text and existing direct release URLs; independent
  native field constraints remain unverified and are not suggested.

Ownership: query model/catalogue, command DTO validation, five provider search
entry points, Online search controller/component, shared chip presentation in
form-controls.css, Workspace integration, targeted unit/native/browser tests,
focused browser workflow and this note. No tag-writing workflow change.

Verification gate: Svelte diagnostics/lint/source checks; native development
build and native WebKit smoke; focused query, single-filter, IME/keyboard,
source-switch/paging and shared Format fields browser regressions in hosted CI;
full applicable platform checks before publication. Runtime switch is explicitly
requested: checkpoint existing settings, Undo journal and WebKit preferences,
reuse existing dependencies/cache/profile, launch the main source via Vite,
verify the window and only then remove the installed AppImage and stale launcher.
Keep the checkpoint as recovery evidence. No music-file test writes.

Upstream classification: shared interface behavior, eligible for curated
`tagrex/tagrex:next` under WORKFLOW.md when publication is requested. This local
main iteration is not an upstream submission or release milestone.

## Local verification result

- `npm run check`: 0 errors, 0 warnings; frontend lint: 99 files, 0 problems.
- Official Svelte analyser: no issues in the new input or affected components.
- JavaScript syntax, native formatting checks and `git diff --check` passed.
- Incremental `cargo build --locked -p tagrex --features dev-mcp` succeeded in the
  existing main build cache with devUrl and the established local bridge overlay.
- Real native WebKit checks passed: Enter/OK insertion, one-chip replacement,
  Escape cancellation, independent provider drafts, unsupported suggestions absent,
  clear, local Delete/Undo, cancelling value entry from the text box, matching
  persistent border in light and dark, and restoration of 371 loaded files.
- Native IPC rejected unsupported source/field combinations and two filters.
  A live Discogs React + Bonkers query returned 15 visible results at the saved
  page size. Live Beatport filtering and MusicBrainz result semantics were not
  independently exercised in this session.
- Tiled window management ignored the requested narrow size: actual viewport
  stayed 1910px. Narrow/Chromium/WebKit regression assertions were added to the
  focused hosted workflow but have not run on this uncommitted working tree.
- Unit/native regression suites and full hosted platform CI have not run; no
  passing CI or completed release checkpoint is claimed. No commit/push/PR.
- Development main window uses `http://127.0.0.1:1420/`, native binary
  `/mnt/Games/Development/tagrex/build-cache/debug/tagrex`, source `personal-app`.
  The installed 1.4.10 AppImage and its release launcher/icon were removed only
  after the development window was verified. Settings/Undo/preferences recovery
  checkpoint: `/mnt/Games/Development/tagrex/checkpoints/provider-search-chips-2026-11-20`.
  Undo table contents matched the checkpoint; no music-file writes were made.
