# Rename sidebar

Operate surface, refining the existing workbench on main after c88ec162.
Rename filenames leads with the shared mask editor and builder, followed by a
compact preview of the resulting filename. Destination has one keep/move/copy choice with a
conditional folder picker and move-only empty-folder cleanup. Related files
retains same-named companions and folder extras with their existing settings.
Keep the explicit Preview → Apply → Undo path and raw mask/storage compatibility.

Own Workspace.svelte rename markup, renamer.js, inspector.css, English copy,
focused sidebar tests and the small footer button-selector/disabled-state seam.
Preserve unrelated player edits in workbench-system.css and player.js. No new
branches/worktrees, dependency installation, native restart or real audio writes.
Completion: preview generation from the fixed footer; destination semantics,
settings restoration, latest-selection examples, keyboard focus, both themes,
short/narrow layouts, builder/preset controls and focused regressions. Use native
read-only inspection for the live app. Shared interface route: upstream next after
a verified local checkpoint; no publication requested for this iteration.

Delivered: three sections with palette-filled fields and filename comparison;
one fixed-footer rename Preview, with the existing controller ID retained.
Other modes keep their staged-preview action. Preview is disabled for no
selection and during busy work, and existing function-composer guards still
block partial plans. Current/proposed examples reject outdated native replies.
No new frontend evaluator or Rust commands were needed: existing native rename,
move/copy and transformation plans remain authoritative.

Verification: 4 sidebar, 10 builder, 3 preset, 9 function-expression, 6 shared
pattern-editor, 9 Apply-progress, 6 selection-summary, 6 BPM-staleness and 1
inspector typography tests passed. Sidebar tests cover native plan flags through
the browser fixture, saved destination/mode restoration, selection clearing,
error recovery, mode switching, keyboard focus and short-window access in both
themes. Svelte check found 0 errors/warnings; lint 103 files/0 problems; build,
JS syntax and whitespace checks passed. Official Svelte analyser found 0 issues;
its suggestions concern existing Workspace effects outside these markup changes.

Light/dark rendered fixtures and native WebKit layout inspected. Live app retains
371 files, the same selected file, raw mask, saved move mode/destination and
cleanup choice, with no staged plan. No native file writes, Undo commands,
settings edits or process restarts were performed for verification. Automated
WebKit and hosted CI were not run. Temporary builds/renders removed after checks.

Verified local checkpoint on main. Commit subject:
`Redesign rename sidebar and unify placeholder pickers`.

Follow-up scope: “Rename filenames” is the single header, with builder access in
that heading row. Remove the duplicate Filename heading and current/proposed
labels; show only the resulting filename on a compact filled surface. The
placeholder picker shares the builder's reference renderer, chip palette,
headings, dark search and fading overlay scrollbars. Supported function chips
open the existing composer only where the target editor advertises support;
extraction and unsupported grammar retain native text insertion. No evaluation
moves into the frontend. Verify caret insertion, function composition, extraction
restrictions, filtering, keyboard dismissal and palette surfaces.

Follow-up verification: all six sidebar/picker tests pass, covering the simplified
preview, supported function insertion with its saved tag/caret, raw tag insertion,
extraction restrictions, filtering and Escape/focus return. Native WebKit shows
44 chips/five supported functions and exactly the builder's dark search fill;
new-name-only preview and single header confirmed with 371 files still loaded.
Svelte check has 0 errors/warnings; lint 103 files/0 problems, build, JS syntax,
Svelte analyser (0 issues; existing imperative-effect suggestions reviewed) and
whitespace checks pass. No native writes or settings changes were made.

Commit gate: fresh Svelte check, lint and fixture build pass; 12 sidebar/shared
pattern tests and 10 builder tests pass. Shared UI upstream route remains next;
this checkpoint is local only. Player work is excluded, including player-only
changes in the shared footer stylesheet.
