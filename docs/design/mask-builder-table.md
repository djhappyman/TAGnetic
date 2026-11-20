# Mask builder table and reference sidebar

Scope: refine the existing large batch builder on main. Retain resizable preview,
300-file inspection bound, native preview queue, extraction modes, selection,
presets, syntax, staged state and Apply/Undo boundary. Preserve the current
unfinished builder expression across development reloads.

Deliver a main-table-style rename preview (sticky headers, stripes, readable
columns and status), a continuous sidebar with its filter inside, and dark chrome
with lighter working surfaces using the existing panel/bg tokens. Only Tags
reference entries become shared field chips; other syntax stays unchanged.
Use the existing vertical/horizontal overlay scrollbar controller and fade rules.

Owned files: UtilityOverlays.svelte, maskbuilder.js, placeholders.js,
field-pattern.js, style.css, surfaces.css, English fallback strings, focused UI
tests/workflow and this note. Shared interface behavior; eligible upstream route
is a curated UI contribution to tagrex/tagrex:next after a verified fork checkpoint.
Completion: batch table, insertion/filtering, scrollbar fade/drag/keyboard, themes,
narrow reachability and unchanged cancel/write boundaries verified. No audio writes.

## Verified result

- Rename preview uses numbered Current name / Proposed name / Status columns,
  sticky sentence-case headers, established stripe/hover tokens and readable data
  fonts. Native names remain escaped, long names can scroll horizontally, and
  existing themed tooltips expose clipped content. From Name/Text keep their
  native extraction columns and diagnostics.
- Panel chrome and bg working surfaces now follow the main app. The reference is
  one surface, including the filter and fixed help footer. Only its 19 Tags
  entries become chips: 12 shared hue values in the live native catalogue. Grammar,
  file, technical and function tokens retain their existing text presentation.
- Preview and reference scrollports reuse initTableScrollbars without modifying
  its shared controller. Both preview axes hide native bars and use the same
  overlay thumbs, drag/keyboard handling and 800ms idle/200ms fade behaviour.
  Reopening does not duplicate controllers. Preview content replacement is
  observed without rebuilding the scrollbar rails.
- Acceptance is disabled during refresh or invalid syntax. The existing one-active
  preview/latest-queued-expression behaviour is retained. Cancel leaves source
  patterns and staged state alone. The 300-row inspection cap remains explicit
  (verified against a 371-file disposable browser fixture).
- Svelte check: zero errors/warnings. Frontend lint: 100 files, zero problems.
  Official Svelte analyser: no issues. Build, JS syntax and diff checks passed.
  Six focused unit tests and 13 Chromium UI tests passed (six table tests, six
  shared pattern-input regressions and one native-queue simulation).
- UI checks cover dark/light surfaces and striping, integrated search, exact
  insertion, non-tag references, collisions/missing/unchanged states, invalid
  expressions, horizontal/vertical fade, keyboard and thumb dragging, reopen,
  extraction and 520px reachability. Native WebKit visual inspection confirmed
  the layout, surfaces, 19 tag chips, shared palette and loaded 371-file library.
- The unfinished builder expression was captured before reload and restored.
  Subsequent user selection/editor interactions remain authoritative. No music
  file writes, file Undo tests or runtime/profile replacements were performed.
- Design detector reported four existing main-table state/drag-marker stripes
  outside this change, and no new builder findings. Automated Playwright WebKit
  remains unavailable locally; the focused hosted Chromium/WebKit workflow now
  includes the new suite. Hosted CI/full platform suites have not run.

Checkpoint ready locally on main, uncommitted. Suggested subject:
`Give the mask builder a batch preview table and continuous reference sidebar`.

## Resizing follow-up scope

Add explicit left/right dialog edge grips and a draggable preview/reference
separator, with keyboard controls and bounded sizing. Keep intrinsic table/column
widths while changing the scrollport; allow reference descriptions to reflow.
Remember sidebar width, reuse the dialog's existing size storage, and retain its
corner resize. Hide the vertical divider in the existing stacked narrow layout.
New owned module: mask-builder-sizing.js; extend focused table UI regressions.
Verify drag, keyboard, cancellation, persistence, table width stability, sidebar
wrapping, overlay geometry and narrow reachability. Same shared UI upstream route.

Resizing verification: Svelte check (zero errors/warnings), lint (101 files, zero
problems), official Svelte analyser, build and diff checks passed. Fifteen focused
Chromium UI tests pass, including edge drags, divider drags, strict sidebar text
reflow, unchanged table/column widths, stored sizes, keyboard limits, Escape
cancellation and the existing overlay-scrollbar/narrow-layout regressions.
Native WebKit keyboard checks changed sidebar 396→406→396px and dialog
1300→1290→1300px while the table stayed 504px wide. Draft and 371 loaded files
remained intact, with no staged plan or audio writes. Native sizing preferences
and the prior open/closed state were restored after inspection. The detector
again reported only the four previously recorded main-table marker warnings.

## Mask-builder chrome refinement

Removed the builder's aggregate count/summary row in all modes and removed its
unused tally/summary computation. Staging remains responsible for counts. The
pattern field has no resting border; keyboard focus and error tint/feedback are
retained. Error-only status space collapses when there is no error. Sidebar group
heading separation increased from 16px to 24px. Scope: existing builder markup,
controller/styles and focused test expectations; same shared UI upstream route.

Frontend check/lint and native live computed-style inspection confirm zero field
border, no summary element, 24px heading gap and the 371-file loaded library.

### Saved masks picker refinement

Scope: remove the builder's transformation shortcut; keep existing chain preview
behaviour. Improve the shared mask bookmark picker in UtilityOverlays,
maskpresets.js and surfaces.css: search, readable tag/function chip previews,
current-mask check, a secondary save form, and explicit deletion actions.
Preserve raw masks, named updates, settings conflict recovery and the Apply
boundary. Completion gate: focused picker/editor tests, Svelte check/lint and
native read-only inspection; both themes and narrow viewport. Shared interface
behaviour; eligible for curated upstream `next` when this local iteration is
checkpointed. User requested continued work on main, without branch/worktree.

Verified 2026-11-20: Svelte check 0 errors/warnings; lint 103 files with
0 problems; official Svelte analyser 0 issues; fixture build passed. Picker
3 + shared pattern editor 6 tests passed on the final picker; builder 9 tests
passed after wand removal. Coverage includes raw/function masks, keyboard
apply/Escape, save/update/cancel/delete, unmodified library/staging, both themes,
480px viewport wrapping and fading overlay scrollbars. Native WebKit inspection
confirmed the picker, 11 chips across the four existing presets and 371 loaded
tracks. Native inspection was read-only; no saved settings or audio were written.
Local checkpoint ready on main, uncommitted. Suggested subject: Improve saved
mask picker and remove builder wand.

Compact picker follow-up: removed the heading; search now says “Search presets”
and shares the dialog filter's `--panel` fill. Menu width is 360px; ordinary
rows are 46px with 18px subtitle chips. Delete uses × again. The footer's
centred “Save preset” action has transparent background, no border and no shadow.
Verified: 3 picker tests (including both-theme search fill, compact row height
and centred action), Svelte check/analyser and lint passed. Native inspection
confirmed matching search fills and transparent borderless Save preset.
Shared interface route remains upstream next; checkpoint subject unchanged.

Dropdown subtitle refinement: preset previews now show the same field/function
colours as plain text, with no chip fill, padding or rounded capsule. Scoped
to `.preset-menu .preset-mask`; editor/sidebar chips retain their usual style.
Native inspection verified all 11 preview tokens are transparent, unpadded and
coloured while the editor retains its chip background. Lint and whitespace
checks passed; no runtime state or saved masks were modified.

Selected preset refinement: removed the tick and its unused copy/style. All
preset titles retain normal text colour; the selected title colour experiment
was reverted at the user's request. Native verification confirmed identical
title colours. JavaScript syntax, lint and whitespace checks passed.

### Dialog scroll-edge fades

Scope: add the main inspector's 8px top/bottom fades to preview and reference
scrollports. Reuse the overlay scrollbar measurement frame via an opt-in flag;
existing users remain unchanged. Fades follow overflow, scroll position, focused
controls, content refresh and resize. Keep sticky headers, search/footer and
scrollbar rails unmasked. Own maskbuilder.js, overlay-scrollbars.js, the builder
styles and focused preview tests. Completion: boundary/resize/filter checks plus
native inspection, lint and syntax checks. Shared interface route: upstream next.

Verified: all 10 builder tests passed, including start/middle/end fade state,
filter removal/restoration, sticky-header offset, unmasked search/footer/rails
and existing resize/scrollbar behaviour. Native WebKit inspection confirmed
sidebar top/bottom fades when scrolled and no table fade with one non-overflowing
row; all 371 tracks remain loaded. Fixture build, JS syntax, lint and whitespace
checks passed. Checkpoint ready on main, uncommitted. Suggested subject:
Match mask builder scroll fades to the main workbench.
