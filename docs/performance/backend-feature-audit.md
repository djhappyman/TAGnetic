# TAGnetic backend feature audit

28 November 2026. Baseline `1d8ac26e`. This audit follows the shipped desktop handlers, MCP tool catalogue, product feature checklist, and Rust entry points that operate without IPC. The ledger assigns every registered command and tool to a feature family. Each family records the behavior, source route, relevant tests, and remaining evidence.

## Feature evidence

- [Feature and endpoint inventory](backend-feature-audit/inventory.md) maps 150 release desktop commands, three debug-only desktop commands, 69 MCP tools, and 184 checked entries from the product checklist.
- [Desktop shell and library](backend-feature-audit/shell-and-library.md) covers Linux/Windows relaunch, macOS Opened, settings, previews, native plugins, update policy, file discovery, search, caches, and duplicates.
- [Library verification](backend-feature-audit/library-verification.md) covers the library's data and resource contracts in more detail.
- [Editing, imports and reports](backend-feature-audit/editing.md) covers staging, masks, recipes, transforms, numbering, matching, import formats, playlists and reports.
- [Writes, history and publication](backend-feature-audit/writes.md) covers media formats, source integrity, Apply/Undo, SQLite, moves and copies, interrupted operations, and guarded outputs.
- [Audio and playback](backend-feature-audit/audio.md) covers tool windows, Split Studio, EQ, Dynamics, Spectral Repair, export, codecs, player, BPM and native media controls.
- [Providers and artwork](backend-feature-audit/providers-and-artwork.md) covers provider lookups and accounts, artwork sources and pipelines, and the fuzz harness.
- [Connected agents](backend-feature-audit/service.md) covers the 69 tools, 21 trusted desktop control commands, private transports, permissions and shared lifecycle.

A source trace establishes implementation and intended checks. A passed test establishes the behavior exercised by its fixtures. Platform rows record the difference. The inventory includes frontend-only product entries so they are accounted for, and identifies the Rust persistence or native boundary where applicable.

## Verified improvements

| Area | Before | After |
|---|---|---|
| Desktop relaunch | The running app interpreted relative arguments using its own working directory. | Uses the sender's working directory. A second process opened a relative music folder in the native Linux smoke test. |
| Media session | A slow cover decode could restore a superseded track or playback state. | Transport state publishes promptly. There is one active decode and one newest queued path. A cover can only update its matching playback generation. |
| Native FLAC history | Related quota, row and digest reads could observe different SQLite snapshots. | The standalone history reader uses a single read transaction. The writer continues using its existing transaction. |
| Legacy Ogg writes | A writer panic could bypass the guard and leave partial bytes. | The guard catches the failure before unwinding past recovery and restores the original bytes. |
| Agent pairing | Before-publication persistence failure could leave a client occupying a slot. | Failed publication releases the provisional slot. Errors after rename keep the published registry and in-memory state aligned. |
| Agent analysis | An unexpected worker panic could leave a job marked running permanently. | The job reports failure and can be released. |
| Likely-duplicate scans | A dense candidate bucket could overshoot its comparison cap before the next check and delay Stop. | The existing million-pair cap is enforced before retaining another unique pair. Long bucket walks poll cancellation. Extremely large duration metadata no longer overflows the bucket range. |
| M3U and CUE | CR/LF inside artist, title or album display text could break records. | Metadata stays on the record line. Quoted CSV, XML and template exports use their own encoding rules. |
| Bandcamp | Autocomplete copied its parsed result array. ISO release dates could be missed. | The parser borrows the result array and extracts years from ISO dates. A 250-row fixture produced identical autocomplete candidates, with 8,106 → 4,355 allocations. |

## Verification

- Linux Rust 1.98.1. `cargo fmt --all --check` and `cargo clippy --locked --workspace --all-targets -- -D warnings` passed on the combined source.
- `cargo test --locked --workspace --no-fail-fast -- --test-threads=4` passed **2,168 tests**, with 49 reported as ignored. A shorter isolated temporary path let the Unix-socket fixture bind. The clean baseline's one failure with a longer path passed after that fixture correction.
- Two additional MPRIS tests passed under a private D-Bus session. The rebuilt native Tauri application opened a relative folder sent by a second process, produced distinct 64-column stereo spectrogram lanes from a disposable WAV, and moved that fixture to OS Trash under its isolated profile.
- All five provider fixture suites and all 16 duplicate cases, including the opt-in FFmpeg case, passed. The opt-in 2,500-file metadata-cache pressure case and the independently decoded audio and report cases are recorded in the relevant feature ledgers.
- All four fuzz targets and their examples compiled with `cargo check --offline --manifest-path fuzz/Cargo.toml --all-targets`. The fuzz harness uses its separate workspace. No fuzz campaign ran in this pass.
- Independent source review checked the combined changes and found no merge blocker. Runtime checks used disposable inputs and isolated profiles.

## Remaining evidence

- Windows and macOS native playback devices, filesystem behavior, named pipes and packaging for this local patch. Linux exercised the app and its D-Bus media session in isolated profiles.
- Large real libraries and long compressed recordings. Existing synthetic pressure tests cover resource contracts. Repeated large duplicate scans, decoder reuse for long-track spectrograms, and legacy whole-file metadata writes need separate measurements.
- Durable recovery under physical power loss, large receipt histories, and agent shutdown when an accepted peer stops reading. Native journal fault and restart tests verify the specific interruption points they exercise.
- Live provider account sign-in, network and API response variation. Local provider tests use recorded/synthetic data. The fuzz harness builds separately from the shipped workspace.
