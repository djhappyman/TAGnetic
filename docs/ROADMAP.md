# Preliminary roadmap

This is a working direction, not a promise of dates or platform performance. Current development targets `main` in this fork. Upstream contribution branches are curated separately.

## Finish FLAC integration, then pause — 18 November 2026

Service authority/lifecycle PR #50 is merged at `67625add` after all four hosted
checks passed. Finish the independent review and merge of the narrow FLAC slice
first, as the owner requested; remaining backend work is preserved and paused. It is
not a new release or full UI readiness claim. See the [pause handoff](design/backend-pause-2026-11-18.md),
[backend programme](design/backend-parity-programme.md) and
[resume issue #51](https://github.com/djhappyman/TAGnetic/issues/51).

The ordinary FLAC Apply/persistent-Undo integration is under fresh independent
review now. Resolve findings, run combined frozen-source checks and publish its
focused scope before the final pause. Review the stress-runner contract in the
next requested session. Keep exact staging, mixed/typed and
other-format operations, picture/export integration and native platform/device
proof open. Do not re-tick product features from backend-only evidence.

## Current milestone — a coherent, testable workbench

- Bring the foundation, compact-player work, progressive filters, interaction fixes and measured optimizations into one maintained branch.
- Keep preview, Apply and Undo boundaries clear and preserve selections, drafts and focus through navigation.
- Establish checked Windows, macOS and Linux artifacts before publishing a personal pre-release.
- Review the interface together and capture fresh light/dark, wide/narrow screenshots. No new screenshots are planned before that session.

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
