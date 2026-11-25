# Engineering roadmap

Current development targets main in this fork. Upstream contributions are curated
separately. Capabilities and outstanding product epics are listed in the README
and [roadmap issue #49](https://github.com/djhappyman/TAGnetic/issues/49).

## Current checkpoint — native tools exposed in the workbench

Released [1.4.10](https://github.com/djhappyman/TAGnetic/releases/tag/personal-v1.4.10)
connects the accepted Autonumber, Disc and Vinyl interface to
Rust, retaining silent validation, displayed order, field locks and independent
Stage/Write scope. Format fields exposes Build a field and Extract fields with
reusable coloured tag chips. Shared toggles now visibly reflect checked state.
See [numbering design](design/autonumber-dialog.md) for exact boundaries.

Release source is `e7727116c665c7179661335aad7ed9a50ef1a82f`.
[Platform validation and all five packaging jobs](https://github.com/djhappyman/TAGnetic/actions/runs/44612675866)
passed; all eight published installer digests match the checksums and build
provenance. The exact-head browser gate passed 63 Chromium and 45 WebKit cases;
the focused native preflight passed all 75 concurrency stress trials across
normal, single-CPU and disk-pressure profiles, including controlled recovery.

Apply the pre-push and preflight-first gates in [WORKFLOW.md](WORKFLOW.md).
Packaged-app launch/install smoke checks remain a tracked follow-up in
[issue #49](https://github.com/djhappyman/TAGnetic/issues/49); installer creation
and provenance verification do not establish installed-app launch behavior.
Local rebuild authorization does not authorize stress against the user library.

## Earlier backend foundation and remaining scope

Frontend #52, service #50, backend #54, documentation #53 and reviewed consolidation
#55 are merged. #55's seven applicable checks passed on ce8fc24d; this is historical
scope evidence, not validation of later source. Preserve the checkpoint/ref and
runtime recovery evidence in [the consolidation record](design/backend-consolidation.md).

General exact/mixed/typed staging, repeated-value expansion, additional format
integrations and the frozen stress programme remain separate unfinished work.
Windows process supervision and real device/NAS/power-loss testing remain open.
Use [tracker #51](https://github.com/djhappyman/TAGnetic/issues/51) and the
[backend handoff](design/backend-resumption.md); never restore an old whole-tree
patch over current main. UI exposure does not complete the saved-recipe editor.

## Product priorities — artwork and library cleanup

The order below reflects the current priorities for desktop features.

1. **Optimize embedded artwork.** Start with front covers across the selection,
   with a maximum dimension and a choice of quality or target size. Skip covers
   that already meet the settings and avoid upscaling. Encode each identical
   source image once per output configuration and reuse it across files.
   Preserve distinct sleeves and other embedded pictures. Start with JPEG for
   opaque covers and PNG where transparency matters. Consider opt-in WebP after
   verifying support in the relevant audio containers and players. Preview old
   and new dimensions and sizes, estimated savings and skipped covers before
   staging changes for Apply and Undo. [Artwork epic #40](https://github.com/djhappyman/TAGnetic/issues/40)
2. **Album and pack checkup.** Show missing or inconsistent covers, duplicate
   track positions, numbering gaps and conflicting label or catalogue values by
   album or folder. When a source edition is matched, show missing or unmatched
   tracks against that edition. Selecting a finding should open the affected
   rows and the relevant repair action.
3. **Finish the duplicate workflow.** Audition two copies from the same playback
   position, pick a keeper and stage selected tags or artwork from the other
   copy onto it. Keep the choice of which files to retain with the user.
4. **Finish the saved recipe editor.** Expose field copying, combining,
   extraction, selective removal and keep-only steps in a single reusable
   cleanup recipe. Add artwork optimization as a recipe step once the standalone
   action is ready. [Field recipe epic #33](https://github.com/djhappyman/TAGnetic/issues/33) · [Workflow recipe epic #35](https://github.com/djhappyman/TAGnetic/issues/35)
5. **Open playlists and import spreadsheet corrections.** Open local M3U/M3U8
   files in playlist order and report missing entries. Map CSV/TSV columns to
   fields, match by filename or path independently of the current sort, and
   preview ambiguous or unmatched rows. [Playlist epic #41](https://github.com/djhappyman/TAGnetic/issues/41) · [Import epic #37](https://github.com/djhappyman/TAGnetic/issues/37)

**Batch scope.** Carry the visible and filtered-out selection counts into
batch previews so the user can confirm which files the action will affect.
Start with artwork and recipes. Preserve the existing selection and unrelated
staged edits through review and Apply.

## Audio conversion and recording splits

The audio studio shares a Rust workflow for conversion, splitting, tagging and
output verification. See [audio studio design](design/audio-split-studio.md).

- [x] **Convert selected recordings.** Open the converter from the command
  ribbon or file context menu. Review MP3 or FLAC outputs, choose quality and a
  destination, and see conflicts or metadata warnings before saving.
- [x] **Keep originals or replace them with recovery.** Save alongside the
  originals by default. Replacement moves originals to a recovery folder after
  verifying the new files, updates the open library and offers Undo.
- [x] **Split from a CUE sheet, Discogs timings or by ear.** Add and move cuts in
  a zoomable waveform with scrubbing, audition and loops. Fast splitting keeps
  the original audio when splitting MP3 to MP3 or FLAC to FLAC. Exact cuts are
  available when re-encoding.
- [x] **Tag and save split tracks together.** Set shared recording tags and
  artwork and edit each track's artist and title. Use tag chips and presets for
  filenames and optional folders. Review and audition the outputs, save a
  draft or export the cut list as a CUE sheet.

## Next — connect remaining built capabilities

- Expose supported split/merge/deduplicate value tools without promising general
  repeated-field storage beyond the admitted native model.
- Connect the remaining report and artwork foundations in focused slices.
- Verify each complete workflow with disposable fixtures and independent readback,
  retaining Preview/Review/Apply/Undo, selection and unrelated staged edits.

## Next — performance and confidence at scale

- Measure native input-to-preview-to-render latency, scrolling and selection with 10k/20k libraries, then larger 50k/100k stress cases. Separate backend timing, IPC and rendering costs.
- Investigate the large-library metadata/cache cliff with file-read counts, cache hit/eviction data, queue waits, allocations and peak memory. Preserve external-change detection and bounded memory.
- Evaluate a tags-only read path using cross-format fixtures and independent Apply/Undo readback before adopting it.
- Share in-flight provider lookups, budget total artwork memory and improve cancellation/Stop responsiveness during provider waits.
- Exercise real WKWebView, WebView2 and WebKitGTK behavior; automate what can be reproduced and document runtime checks that still need people/devices.

## Interface refinement

- Make A/B transformation comparisons and automatic previews more intuitive without adding a dense permanent control surface.
- Refine filtering incrementally: saved conditions, clearer active scope and hidden-selection counts, keyboard flow and predictable invalid-query behavior.
- Improve bulk editing feedback for very large selections while retaining the familiar table and explicit write boundaries.
- Complete keyboard, focus, contrast, reduced-motion and real IME checks; improve localization of newer controls.
- Refine artwork navigation and visual consistency, then document the resulting look and feel with actual screenshots.

## Later — a sustainable personal project

- Establish the Tagnetic identity throughout the app and documentation, retaining clear attribution and the history of the TagRex foundation.
- Add fork-specific signed/notarized releases and, if worthwhile, a separate trusted updater feed. Never install original-project releases over this interface.
- Keep contributions small and testable, with a visible upstream status ledger and release notes grounded in shipped behavior.
- Consider larger architectural changes only when profiling or maintenance evidence justifies them.

## Release gates

A local passing suite is preliminary. The applicable Linux, Windows and macOS checks and triggered audits must pass on the exact published head. Cancelled or approval-required jobs are not passing evidence. Build artifacts prove packaging; they do not certify native interaction behavior on every platform.
