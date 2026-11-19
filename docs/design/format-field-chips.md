# Format fields and reusable tag chips

Scope: extend the existing Format fields ribbon dialog with Build/Extract and a
reusable pattern editor. Other application inputs and search behavior are unchanged.

Build chooses **Apply to** and composes **Values** from tags and literal text.
Extract chooses **Pull from** and a **Pattern** whose chips name target fields.
Both preview using the existing native field operation command and use the same
shared stage, field locks, Apply and persistent Undo. Write retains the
existing quick-write route and saves only this operation's target fields.

`FieldPatternInput.svelte` accepts a canonical native pattern string, a supplied
field catalogue, a label, disabled/invalid state and an on-change callback. It
owns presentation, caret movement, atomic chip deletion, plain-text clipboard,
local undo/redo, IME handling and suggestions. It does not evaluate expressions
or interpret extraction. Native Rust remains authoritative for both operations.
Unknown native syntax stays exact text. Ordinary words become chips only when a
suggestion is explicitly accepted. Labels use the existing title-case catalogue;
canonical field keys retain their spelling independently of displayed labels.

Suggestions appear after at least two letters and 200 ms without further typing.
They highlight the matching letters, offer keyboard navigation and accept with
Enter or pointer. Escape dismisses the list before closing the dialog. Tab moves
focus without implicitly accepting a tag. Moving the caret, blur and composition
hide suggestions. Shared anchored-popup placement handles viewport edges/scroll.

The dialog shows up to three actual selected-file results and whole-selection
mismatch/lock feedback. A changed selection, staged input, lock or newer request
invalidates earlier native results. Preview never writes tags. Multi-target
extraction stages one complete batch while preserving unrelated edits and paths.

Completion gate: hosted Svelte checks/lint, unit contracts, browser integration
and screenshots in both themes/narrow layouts, followed by the applicable full
platform gate. Browser mocks exercise frontend composition; they do not prove
native media writing. Native extraction and Apply/Undo keep their existing Rust
verification scope. No local build/test/benchmark/formatter is authorized.

Upstream classification: shared interface behavior, intended for curated
`tagrex/tagrex:next`. It depends on the compatible native field-operation command
and the current workbench foundation; while #729 is open, state its dependency
and merge order. Do not port fork branding or pooled journal policy.

## Direct-main visual refinement

Per the owner's current workflow, iterate directly in the running main checkout;
no additional local branch/worktree or fork PR is needed for these UI iterations.
Source delivery and hosted verification status remain distinct.

Tags keep a stable field hue across input chips and suggestion text. Preview
field names use a muted version of the same hue; result values use normal text.
Chips use a 24px inline-flex box centred in a single-line 36px editor. Suggestions
remain in the shared anchored overlay and never participate in dialog layout.
The dialog keeps a stable viewport-bounded height; results scroll internally.
Build/Extract tabs connect directly to the recessed body without a border.

No expanding validation/error panel or instruction paragraph is shown. Invalid
or pending results disable the action buttons; actual write/stage failures still
use the existing nonblocking notification. Footer labels are Cancel, Write and
Stage Edits. Opt-in existing controller tooltips explain Write after 1200ms and
Stage Edits after 1600ms hover (keyboard focus exposes help immediately).
Global footer button borders/shadows are removed, retaining explicit focus rings.

The first hosted UI run found the initial chip catalogue arriving after the
nonreactive editor reference, a keyboard test pressing a still-disabled preview
action, and a hidden editor incorrectly left contenteditable. The component
now tracks its attached editor and disables editing while closed. The keyboard
regression explicitly waits for native preview validity before pressing Enter.
The original run passed 659 browser tests; its three failures remain archived.
New hosted probes verify stable geometry, popup overlay, shared hues, centring,
footer border/focus, connected tabs, delayed help and quiet invalid states.

### Atomic caret and quiet preview refinement

WebKit needs a real text position beside noneditable inline-flex chips. Marked
zero-width caret stops are now supplied at empty boundaries; their marker is
removed during canonical pattern serialization and copy/cut. Literal pattern
text, including real invisible characters, is otherwise retained. Clicks snap
to a chip edge, arrows cross a tag atomically, and both keyboard deletion and
beforeinput deletion remove one whole tag. No caret is left inside a chip.

Preview labels are muted field-colour chips, with plain result values beside
them. The last result remains while a new preview runs silently; actions remain
disabled until the new result is accepted. Loading prose is absent. The content
corner beneath the inactive Build/Extract tab is rounded in either mode.
Hosted caret/clipboard/geometry contracts now run in WebKit as well as Chromium.
