# Format fields and reusable tag chips

Scope: extend the existing Format fields ribbon dialog with Build/Extract and a
reusable pattern editor. Other application inputs and search behavior are unchanged.

Build chooses **Put into** and composes **Values** from tags and literal text.
Extract chooses **Read from** and a **Pattern** whose chips name target fields.
Both preview using the existing native field operation command and use the same
shared stage, field locks, Apply and persistent Undo. Write now retains the
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
