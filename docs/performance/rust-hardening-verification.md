# Final Rust hardening verification

Verified on Linux on 17 November 2026. Final code checkpoint: `b657f65d` on
`feat/native-field-foundations`. The programme remains backend-only; this is not
full Mp3tag parity, a package release or an enterprise certification.

- `cargo test --workspace --all-features`: 1,148 passed, zero failed, 19 ignored,
  across 46 test binaries/doc-test groups.
- `cargo clippy --workspace --all-targets --all-features -- -D warnings`: passed.
- `cargo fmt --all -- --check` and `git diff --check`: passed.
- `npm run check`: zero errors and zero warnings.
- `npm run test:unit`: all 205 passed.
- `npm run build`: passed.
- `cargo build --locked -p tagrex --features dev-mcp`, with the existing local
  development/MCP configuration overlays: passed.

The dev shell was launched from this checkout against the existing live Vite
server without replacing that server's source attachment or development profile.
A native IPC probe succeeded and the main window reported the development URL
and active Vite client. Session/history preservation evidence and the recovery
checkpoint are held outside the repository in `runtime/native-dev-handoff`.
No audio files were written during runtime handoff; test writes used disposable
fixtures. Shared dependencies, caches, other checkouts and portable builds remain.

New regressions cover cache invalidation/reuse, bounded/LRU cache admission,
shared computed metadata, streaming serialization/export limits, staged-view
scope and revision races, preview supersession and dropped callers, artwork
resource budgets and encoded caching, playback coalescing/order/admission, and
shared metadata isolation. Existing transaction, preservation, recovery and Undo
suites passed. No wall-clock speedup or process-wide RSS bound is claimed.

Cross-platform filesystem behavior, real-device playback latency, production
queue/compute profiles and hosted CI are still unverified. Eligible shared fixes
require focused upstream `main` curation; fork service integration needs separate
compatibility review. No PR, push, merge, installer or release is claimed.

The unchanged 207 top-level README feature entries retain their previous states.
Only completed backend capabilities beneath the performance roadmap were added.
The two existing format/migration research documents remain preserved as research;
they are not implemented capabilities.

Ignored cases (not counted as passed):

```text
test media_session::mpris::tests::every_method_sends_the_frontend_its_event ... ignored, needs a D-Bus session bus: run under dbus-run-session
test media_session::mpris::tests::the_player_is_on_the_bus_with_what_the_frontend_reports ... ignored, needs a D-Bus session bus: run under dbus-run-session
test update::tests::the_latest_release_can_be_read_from_github ... ignored
test duplicates::tests::local_transcoding_check_finds_same_recording_across_encodings ... ignored, requires ffmpeg only to generate a disposable MP3 fixture
test tests::field_recipe_benchmark ... ignored, manual performance measurement; creates 10,000 disposable tiny files
test tests::field_recipe_independent_audio_roundtrip ... ignored, manual independent verification; requires ffmpeg and ffprobe
test tests::native_batch_benchmark ... ignored, manual performance measurement; creates 10000 disposable tiny files
test tests::native_batch_independent_audio_roundtrip ... ignored, manual independent verification; requires ffmpeg, ffprobe and Python mutagen
test tests::redo_preserves_flac_audio_with_independent_readback ... ignored, requires ffmpeg and ffprobe for a disposable FLAC fixture
test tests::shared_artwork_thousand_file_memory_probe ... ignored, bounded standalone artwork allocation probe; run with --ignored --nocapture
test audit_encodings ... ignored, audit harness: prints findings, run with --ignored --nocapture
test audit_executor ... ignored, audit harness: prints findings, run with --ignored --nocapture
test audit_formats ... ignored, audit harness: prints findings, run with --ignored --nocapture
test fuzz_all ... ignored, audit harness: prints findings, run with --ignored --nocapture
test fuzz_real_files ... ignored, audit harness: prints findings, run with --ignored --nocapture
test audit_flac ... ignored, audit harness: prints findings, run with --ignored --nocapture
test audit_m4a ... ignored, audit harness: prints findings, run with --ignored --nocapture
test audit_mp3 ... ignored, audit harness: prints findings, run with --ignored --nocapture
test auth::tests::reads_the_client_id_off_the_live_docs_page ... ignored, hits the live documentation page
```
