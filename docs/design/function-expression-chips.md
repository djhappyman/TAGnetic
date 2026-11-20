# Inline text-function expressions

Scope: extend the shared field editor on main with typing-led Tag/Function
suggestions, short function descriptions and a Value / operation / argument row.
No input-type selector. Uppercase, Lowercase, Trim, Find (native strpos) and Replace
use the existing native language; logic remains raw. Support nested expressions,
whole-expression chips, click/F2 editing, operation switching, two-word ghost
prompts, atomic delete/local Undo and exact raw clipboard/preset compatibility.

Ownership: function-pattern.js, FunctionComposer.svelte, FieldPatternInput.svelte,
PatternInput.svelte, FormatFieldsDialog.svelte, pattern-input.js, focused mocks,
shared form styles, tests/workflow and this note. Preserve existing uncommitted
builder work and runtime. Completion: lossless native syntax, composer cancellation,
no partial staging, IME/keyboard/paste, nesting, themes/narrow reachability and live
read-only native verification. No real audio writes. Shared UI route: curated
upstream contribution to tagrex/tagrex:next after a verified fork checkpoint.

## Native ownership and delivered behavior

Rust now owns the text-function catalogue, exact expression/argument spans,
UTF-16 offsets and argument construction/quoting, reusing the native mask parser.
No new dependency was added. Stateless Tauri commands expose that presentation
seam; native evaluation and file operations keep their existing path. The UI
caches spans (bounded to 100 expressions), debounces requests and keeps at most
one active layout request per editor, coalescing later input. Confirmation loads
its native spans before committing, so Backspace is atomic immediately.

The JS scanner lives only in the existing browser-only mock transport. Product
interaction code does no function parsing or evaluation. Recent table status
formatting and resizing stay in the UI; native plans and transactional validation
already own actual rename/write decisions, so an extra native round-trip for DOM
geometry or presentation would add work rather than remove it.

Delivered: tag/function suggestions and brief function descriptions; Value /
operation / argument columns; nested argument chips; operation switching with
retained values; exact no-op editing; click/F2 editing; whole-chip deletion and
local Undo. Supported functions are Uppercase, Lowercase, Trim, Find (strpos) and
Replace. Logic/maths and other advanced functions remain in their existing text
form. Extraction offers no new function operations. Unfinished composers block
associated actions, cancel when their parent closes/hides, and ignore cancelled
asynchronous construction replies.

The sidebar uses shared chips for supported values and text functions (44 chips
in the live render reference, including five function controls and width/side
skeletons). Function controls open the same composer instead of inserting an
empty call. Name-only suffixes are removed there. Group headings use sentence
case, 600 weight, existing label colour and section spacing. Other grammar,
logic/maths and unsupported functions retain their current text representation.

## Verification

- Native core: three focused tests pass, covering canonical grammar, Unicode
  offsets, quoted separators/apostrophes, empty replacements, nested functions,
  untouched arguments and exact no-op spelling. Native debug build succeeded in
  the established cache/profile with the existing dev-server/bridge overlay.
- Frontend check: zero errors/warnings; lint: 103 files, zero problems. Build,
  syntax, Rust formatting and diff checks pass. Svelte analyser reports no issues;
  remaining suggestions are reviewed imperative DOM/async bridges and event-only
  caches. Design detector reports only the four previously recorded main-table
  markers outside this change.
- Final focused Chromium run: 18 UI tests pass (nine expression/sidebar cases and
  nine builder/scrollbar/sizing cases). Shared field/pattern regressions also ran;
  the later narrow-row fix was verified by the new composer suite. Browser-mock
  unit tests: nine pass. Hosted Chromium/WebKit workflow includes the new suite;
  hosted CI, full platform checks and automated Playwright WebKit have not run.
- Native WebKit: new Rust commands verified, a complete chip and inline edit row
  inspected visually, all eight reference headings checked (600, sentence case),
  five supported function controls/44 chips and zero name-only suffixes confirmed.
  Direct native construction/layout checks cover nested calls and quote escaping.
- Native process alone restarted once to load commands. Dev server stayed active;
  371 loaded files and the selected row returned. Later user filtering/selection
  remains authoritative. Settings and every Undo table match the checkpoint at
  /mnt/Games/Development/tagrex/checkpoints/function-chips-2026-11-20. No audio
  writes, file Undo operations, new worktrees/branches or publication.
- Existing builder/resizer work and unrelated player/workbench edits were retained.

Final stage checkpoint verified on main: 28 focused Chromium UI tests, three
Rust presentation tests and three browser-mock unit tests pass. Svelte check,
lint, fixture build, Rust formatting and whitespace checks pass. Automated
WebKit and hosted/platform CI remain unverified; native WebKit inspection is
recorded above. Includes the finished mask-builder resizing/table/reference and
compact saved-preset picker iterations documented in mask-builder-table.md.
Player edits are outside this checkpoint. Shared UI upstream route remains next.
Commit subject: `Improve mask builder with function chips and saved presets`.
