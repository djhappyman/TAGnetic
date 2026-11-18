# Native field recipe validation

Measured on 17 November 2026 on Linux x86_64, Intel Core i7-12700K (20 logical
CPUs). Baseline: fork `main` at `23286e0e`; implementation branch:
`feat/native-field-foundations`. Cargo's test profile uses the repository's
existing per-package optimisation settings, not a release build.

## Scope and upstream route

This checkpoint implements explicit custom-field references, native field
formatting/copying and text remove/keep recipes. The existing Format values
command uses the native operation. Recipe-editor controls and complete
Mp3tag script compatibility are not part of this checkpoint.

The reusable core and native field operations are candidates for a curated
upstream `main` contribution. The shared-stage/MCP and desktop adapters require
separate compatibility review; shared UI integration follows upstream `next`.
This local checkpoint does not publish or merge a PR, change the running app,
or modify the separate README/roadmap draft in the main checkout.

## Performance probe

Run from the implementation checkout using the existing development environment:

```sh
source /mnt/Games/Development/tagrex/dev-env.sh
cargo test -p tagrex-commands --lib field_recipe_benchmark -- --ignored --nocapture
```

The probe creates 10,000 disposable copies of a tiny tagged FLAC. It formats a
custom Venue field, composes Comment from that value and Artist, then keeps a
selected set of text fields. It asserts the number of changed files, with no
writes during the measured preview. Each size has one first result and five
warm samples. Smaller sizes warm overlapping files before the next size, so
“first result” is not a completely cold-disk measurement.

| Selected files | First result | Warm median | Warm maximum |
| --- | --- | --- | --- |
| 100 | 4.786 ms | 0.437 ms | 0.510 ms |
| 1,000 | 38.671 ms | 3.242 ms | 3.715 ms |
| 10,000 | 394.661 ms | 35.440 ms | 37.285 ms |

These timings cover native preview construction, recipe compilation, metadata
cache lookup and field diffs. They exclude IPC, rendering, real-library scanning,
Apply, image decoding and journal writes. Cache-warm results are not disk-read
throughput claims. The benchmark runs alongside the independent readback probe
in the recorded validation command; scheduling noise is included.

## Correctness and preservation

The core, commands and service test suites cover custom rendering and extraction,
unchanged legacy masks, raw punctuation/Unicode, empty-value clearing, field
aliases, ambiguous custom names, preflight errors, cancellation, no-op handling,
locked intermediate values, old-value baselines, staged composition and retained
structural changes. Service tests cover current working values across different
scopes, protected human inputs and saving native recipes with revision checks.

The independent probe generates real synthetic MP3, FLAC and M4A files with
FFmpeg. FFprobe reads their metadata separately from Tagnetic. The probe checks
that preview leaves the file bytes unchanged, Apply writes the requested values
and removals, and Undo restores independently read metadata. It also hashes
copied encoded audio packets before/after Apply and Undo. All three formats
pass. Vorbis field names are compared case-insensitively because the existing
writer may normalise their casing; every metadata value remains checked.
Artwork byte/type/description preservation and explicit removal/Undo are covered
by the disposable native artwork fixture.

```sh
cargo test -p tagrex-core -p tagrex-commands -p tagrex-service
cargo test -p tagrex-commands --lib field_recipe_independent_audio_roundtrip -- --ignored --nocapture
cargo clippy -p tagrex-core -p tagrex-commands -p tagrex-service --all-targets -- -D warnings
cargo check -p tagrex
npm run lint
npm run check
npm run test:unit
npm run build
npm exec vite build -- --config tests/ui/vite.config.js
npm exec playwright test tests/ui/format-values-batch.spec.js tests/ui/command-ribbon.spec.js --workers=2
```

The desktop check requires the normal frontend build to exist. Node dependencies,
Rust dependencies and the build cache were reused. The focused browser suite
passes 27 tests; the frontend unit suite passes 205 tests. Browser mocks exercise
UI integration, not the Rust engine or its throughput.

Validation is local Linux evidence. Native window interactions and Windows/macOS
execution have not been exercised for this checkpoint. No real collection,
active application profile or existing Undo history was used for test writes.
