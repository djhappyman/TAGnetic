# Preliminary roadmap

This is a working direction, not a promise of dates or platform performance. Current development targets `main` in this fork. Upstream contribution branches are curated separately.

## Backend checkpoint and pause — 18 November 2026

Service authority/lifecycle PR #50 is merged at `67625add`. The finish-and-pause
boundary is protected Linux FLAC [PR #54](https://github.com/djhappyman/TAGnetic/pull/54),
reviewed head `f663bd63`; required hosted results and merge evidence live there.
Its clean publication passed 1,623 workspace tests, Clippy and formatting after
scoped independent reviews, including portable policy and true-death harnesses. It includes required shared file-recovery,
per-App policy and concurrent-admission corrections.

All remaining backend implementation is paused and preserved. Start next time
with the [handoff](design/backend-pause-2026-11-18.md),
[programme](design/backend-parity-programme.md) and
[resume issue #51](https://github.com/djhappyman/TAGnetic/issues/51).
Review the stress-runner contract independently before claiming a complete
programme stress run. Exact staging, mixed/typed/other-format operations,
picture/export integration and native platform/device/large-library proof remain
open. Backend evidence does not re-tick product features or create a release.

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
