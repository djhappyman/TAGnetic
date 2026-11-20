# Pattern input chips

Scope: replace the plain Rename, From Name, From Text, report/export naming,
artwork import, custom-column and mask-builder inputs with the existing shared
field chip editor. Preserve raw pattern strings, presets, caret insertion,
detection, previews, staged edits and explicit write/Undo boundaries. Work stays
in the existing main checkout at Mike's request; preserve unrelated edits.

Files owned by this task: FieldPatternInput.svelte, new PatternInput.svelte,
pattern-input.js, Workspace.svelte, UtilityOverlays.svelte, RibbonTools.svelte,
maskbuilder.js, exporters.js, field-pattern.js, focused tests and this note.
Completion: every listed entry point renders known placeholders as chips, offers
operation-appropriate fields and passes focused editing/controller checks.
Verification: Svelte check/analyser, lint, focused unit/UI tests, live native
read-only checks. No real audio writes. Shared interface behavior; eligible route
is a curated upstream UI contribution to tagrex/tagrex:next after a verified
fork checkpoint. No commit or publication is requested in this iteration.

## Verification

- All eight entry points converted. Export filenames enable tag suggestions only
  for grouped playlists; ordinary export filenames remain literal.
- Shared native catalogue filters render vs extraction suggestions. Width tokens
  render as chips and retain exact raw syntax. Functions and unknown tokens stay
  editable text; saved presets and native parsing retain their existing format.
- Existing controllers keep value, selection, focus and input-event access. The
  builder's error/empty states still target the outer compound field. Suggestions
  consume Enter before dialog/controller handlers. Paste strips line breaks.
- `npm run check`: zero errors/warnings. `npm run lint`: 100 files, zero problems.
  Official Svelte analyser: no issues in all five affected components; retained
  effect suggestions concern imperative DOM synchronisation/existing workspace
  effects. Build, JavaScript syntax and `git diff --check` passed.
- 18 focused unit tests passed. 24 Chromium UI tests passed across the new inputs,
  Format Fields, staging and online chips. After the final builder-state fix,
  all six new UI tests passed, including preset save/apply and Clear/Undo.
- Native WebKit: chips visible in Rename, suggestions commit with Enter, atomic
  Backspace/local Undo, and builder suggestion Enter leaves its dialog open.
  Cancel leaves the source pattern unchanged. Restored original rename pattern.
  371 files and the original selected path remain loaded; preview plan is null.
  No real tag/artwork/rename writes or file Undo operations were performed.
- Automated Playwright WebKit could not launch: its webkit-2388 executable is not
  installed. Hosted Chromium/WebKit workflow now includes this regression file;
  no hosted CI/platform-matrix result is claimed.
- Task also changes the browser mock catalogue (Album/Album Artist), focused
  workflow and tests. Existing uncommitted provider-search work is preserved.

Checkpoint ready locally. Suggested subject: `Use field chips across pattern inputs`.
No commit, branch, worktree, push or PR created.

## Compound surface correction

The nested chip editor was painting `--panel` over the lighter compound field.
The shared component now uses a transparent background when directly inside a
`.field`, leaving the outer field as the single surface owner. Standalone editors
retain their own fill. Verified live computed backgrounds across six compound
editors in both themes and visually in the native app. Svelte check, lint and
diff whitespace checks pass. This is part of the same shared UI upstream route.
