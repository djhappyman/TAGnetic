# Independent numbering actions

Scope: expose native numbering through the accepted three-tab dialog, preserving
its appearance and the existing Stage/Review/Apply/Undo workflow. Iterate on main.

## Current behavior

- **Autonumber** changes track numbers only, with a chosen start, optional track
  totals and a reset at each consecutive displayed-group change.
- **Disc** changes disc numbers only, optionally advancing at displayed-group
  changes, resetting at album changes and adding disc totals.
- **Vinyl** reads positions from Track or Disc. B3 becomes Disc 2, Track 3; a bare
  B becomes Disc 2, Track 1. Reverse/lowercase notation is accepted. Plain numbers
  are skipped. Optional Position preservation, side track totals and album disc
  totals are independent. Zero and overflowing track numbers refuse the action.

Only active-tab controls are sent. No inactive option can affect another action.
Album changes take precedence when disc/group boundaries coincide. Boundaries
follow consecutive files in displayed selection order, without hidden sorting or
merging scattered occurrences. Totals use the highest assigned ordinal in each
segment, matching the native numbering foundation; they cover selected files.

## Rule and publication ownership

`preview_numbering_dialog` runs on the cancellable native session queue. Rust
reuses the existing numbering engine for tracks and discs; vinyl shares the
native side parser. The interface supplies intent, current group identities and
a small snapshot of current source values, including raw staged inputs awaiting
acknowledgement. It performs no production counter/total/conversion calculation.
The browser-only mock is an interface fixture, not a second production path.

Rust returns an operation-only validated plan and all requested assignments,
including values unchanged on disk. Stage needs those assignments to replace an
older pending value. Write cannot pick up unrelated staged changes. Both actions
use the existing field locks and write/Undo machinery. Conversion requires both
track and disc fields together. Native validation, cancellation, file limits,
checked integer arithmetic and group-payload limits apply before publication.

The interface seals selection, display order, grouping, staged revision and locks
around asynchronous validation and rechecks them when publishing. Cancel and newer
requests invalidate old results. Stage records one complete batch. Silent preview
updates do not fade the footer; a click awaits its exact latest request.

## Accepted presentation

The fixed viewport-bounded shell retains rounded connected tabs, borderless
surfaces, a two-column toggle grid and subtitles of at most four words. Vinyl explanation is available from a keyboard-accessible info icon beside the
source-field label. There is no result preview or loading/error panel. Actual failures use the existing toast.
Footer buttons remain Cancel, Write and Stage Edits, with delayed tooltips.
Display width is a preference; stored numbers remain unpadded. Only the active
tab's relevant width preference is saved after success.

Shared Switch/Checkbox/Radio styles now use Bits UI's actual `data-state` values.
The compatibility switch uses individual `translate` so movement is not doubled.

## Verification and contribution route

Rust fixture tests cover scope isolation, staged albums and vinyl sources,
per-side/release totals, unchanged-value assignments, locks, cancellation,
duplicates, group alignment and overflow. JavaScript contracts cover native intent
serialization, asynchronous invalidation and Stage/Write isolation. Browser tests
cover layout, independent tabs, stable footer validation and toggle state.
Authored tests are not execution evidence; current-head GitHub results and native
fixture readback determine release readiness. Build/test workloads remain separate
from the running user library; test writes use disposable audio only.

Shared interface behavior is eligible for curated `tagrex/tagrex:next` contribution
after compatible foundations (#729); standalone shared backend changes route to
upstream main. No upstream publication is implied by the fork release.
