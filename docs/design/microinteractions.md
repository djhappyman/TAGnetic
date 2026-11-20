# Workbench microinteractions

Tagnetic adapts selected [Interior.dev](https://www.interior.dev/docs) behaviours
using existing Svelte/Bits UI controls, CSS and Web Animations. It adds no React
runtime or animation dependency and does not alter media-write authorization.

## Shared feedback

- Workspace and manual-source selection use a travelling themed fill. Reversals
  start at the current visible position; resizing and reduced motion settle it.
- Main workspace tabs have no hover or keyboard tooltips. Toolbar hints wait
  450ms initially, then neighbouring hints appear immediately within a 400ms
  grace period. Escape, activation, scrolling, resize and blur clear the grace.
- Buttons use quiet press and colour feedback. Tabs, popup anchors, unavailable
  actions and separators stay anchored. Focus and persistent states remain clear.
- The multiple-selection pill only appears above one selected file. Its count
  acknowledges changes without replaying entry; dismissal retains the last
  multiple count so zero and one do not flash during exit.
- Staged counters acknowledge changes without adding routine success toasts or
  changing persistent staged/error colours. Virtual rows receive no observers.
- Apply exposes busy state immediately. Apply/Undo meters expose labelled measured
  progress when backend totals exist; unknown totals remain indeterminate.
  The visual delay still avoids a flash for quick operations, and cancellation
  and rollback boundaries remain unchanged.

## Sidebar and table

The sidebar, file-pane boundary, ribbon and footer/player share a 180ms transition.
The table scrollport stays full width and data columns retain their geometry
under the overlay. Desktop toggles avoid unnecessary dock publication and delayed
preparation; compact layouts retain their required module routing. Nodes, drafts,
selection, keyboard reachability and scroll positions remain intact.

## Artwork preview

Clicking Inspector artwork opens a full-window lightbox over a dark veil. The
image expands from the actual thumbnail and returns to it on close. A sampled
150/27 home spring follows Interior's [lightbox](https://www.interior.dev/docs/lightbox)
pattern, with a reversible 520ms travel. Existing read-only zoom and pan remain;
reduced motion skips travel. The pencil keeps artwork-editing commands.

Artwork dialogs own Escape before the global staged-plan discard handler. Closing
preview, including during entry, must preserve a pending draft and return focus
to the thumbnail.

## Verification

Regression coverage exercises rapid reversal, reduced motion, both themes,
compact layouts, fixed table/column geometry, native-control event semantics,
Apply/cancel recovery, Undo and read-only artwork inspection. Browser fixtures
prove frontend behaviour; native playback/file writes and release packages have
separate gates. Tests never write to the user's music library.

## Popup resize delivery

`npm ci` applies `tools/patch-floating-resize.mjs` to Floating UI 1.8.0 and Bits UI 2.21.0. Their ResizeObserver callbacks otherwise perform layout writes during delivery, which WebKit reports as undelivered notifications while animated selects settle. The idempotent, version-checked fixes coalesce only resize-driven layout into a cancellable animation frame. Initial placement and scroll-driven placement retain their existing timing. Keep both dependencies pinned until this patch is reviewed or replaced by an upstream fix; do not suppress browser errors in the regression tests.
