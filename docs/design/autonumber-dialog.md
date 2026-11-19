# Independent Autonumber and Disc actions

Owner scope: two connected tabs in the numbering dialog, with independent actions,
matching Format fields styling. Iterate directly in the running main checkout.

**Autonumber** owns track start, track display, include-total, group restart and
vinyl-position retention. Its Stage/Write routes explicitly pass `includeDisc:false`
to the existing numbering engine. They do not read, validate or change the inactive
Disc input. Only the track display preference is saved after success.

**Disc** owns disc number and disc display. Its separate `number-disc` action
validates only disc input, previews disc-only scalar edits through the existing
native command, and refuses stale selection, changed staged inputs, new field
locks and invalid native values. Stage records only disc edits in one batch,
preserving pending track/other fields. Write saves the disc-only plan through
the existing quick-write/Undo route. Only the disc display preference is saved.
Setting a disc applies to the selection independently of track/vinyl/group rules.

The shared lower-level numbering helper retains its optional composed behavior
for existing callers; the desktop Autonumber action always explicitly excludes
disc assignment. No Rust command, storage or journal format is changed.

The active tab determines validity and action scope. An invalid value left in
the other tab cannot block or affect it. Both tabs preserve their temporary
inputs while switching. Cancel/reopen resets them and reads saved display widths;
failed operations keep the inputs. Disc requires a positive whole number.

The fixed, viewport-bounded shell has connected borderless tabs, rounded inactive
corners and concave joins, with internally scrolling panels and no loading/error
prose. Cancel, Write and Stage Edits share the borderless footer. Existing delayed
tooltips and assistive descriptions explain immediate Write and Stage/Review/Apply.
Leading zeroes affect display; tags remain numeric. No media is written by opening,
editing controls or switching tabs.

Hosted coverage: Svelte/lint/unit plus Chromium/WebKit browser contracts for both
tabs, inactive-input isolation, field scope, staged-field preservation, independent
width persistence, numbering order, quiet validation, layout and Disc Write/Undo.
No local build/test/benchmark workloads run. Mock browser results do not establish
native media preservation outside existing backend verification scope.

Upstream route: shared interface -> curated tagrex/tagrex:next after compatible
workbench/native-command foundations (#729), preserving upstream storage/API policy.
