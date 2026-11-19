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

## Next — connect remaining built capabilities

- Add field-copying, combining, extraction, removal and keep-only recipe controls.
- Expose supported split/merge/deduplicate value tools without promising general
  repeated-field storage beyond the admitted native model.
- Connect the import, playlist/report and artwork foundations in focused slices.
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
