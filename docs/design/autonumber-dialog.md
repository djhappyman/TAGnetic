# Quiet Autonumber dialog

Scope: bring the existing Autonumber interface into the Format fields visual
language, directly in the main checkout per the owner's iteration workflow.
`AutonumberDialog.svelte` owns the dialog presentation and its temporary inputs.
The existing generator still computes/stages numbering in displayed file order;
Write retains the existing quick-write and Undo route. No Rust/wire change.

The recessed body contains Start at, Track display, Disc number and Disc display,
followed by three compact switches: include track total, restart for each group,
and keep A1/B2 positions. Both display selectors stay visible. No extra tabs or
new numbering modes are introduced. Leading-zero choices affect table display;
stored tags remain numeric. That explanation is available to assistive technology.

Cancel, Write and Stage Edits share the borderless footer convention. Write's
1200ms hover hint explains bypassing staging; Stage Edits uses 1600ms and explains
Review/Apply. Actions disable for invalid or missing starts and invalid disc
numbers. Invalid inputs do not expand an error panel. Actual operation failures
use the existing nonblocking notification, loaded after the shell mounts.

Cancel/reopen resets temporary inputs and reads saved display widths. Only a
successful Stage/Write saves those widths. Failed writes retain the inputs when
the dialog reopens. Existing selection, field locks, grouping, vinyl retention,
staged edits and history retain their existing engine contracts.

Verification: hosted Svelte checks/lint/unit and command-ribbon integration tests
cover numbering order, both formatting choices, cancellation, preserved staging,
quick writes, switches, quiet invalid states and narrow footer reachability.
No local builds or test suites run under the current machine-use constraint.
Visual/component review is not independent verification of native media writes.

Upstream route: shared interface -> curated tagrex/tagrex:next after compatible
workbench/native-command foundations (#729). No fork identity/storage changes.
