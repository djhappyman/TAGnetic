# Independent artwork pixel-engine review

18 November 2026. Reviewed the new `cover::pipeline` public engine in the existing
`native-field-foundations` checkout. This is a scoped defensive review, not full
artwork, staging, backend or release approval. No production code was changed.
Owned outputs are this report and `core/tests/artwork_pipeline_adversarial.rs`.

## Disposition: changes required

**P2 — contradictory Orientation entries inside one EXIF IFD are silently accepted.**

`png_header` (`core/src/cover/pipeline.rs:879`) delegates each eXIf payload to
`image::metadata::Orientation::from_exif_chunk`. That helper returns one
orientation. The pipeline's conflict test compares different chunks, so it never
sees two contradictory entries inside that same payload. The analogous JPEG
APP1 path delegates the same way; the retained reproduction uses PNG.

A synthetic, CRC-correct PNG contains a big-endian TIFF IFD with two Orientation
SHORT entries, values 6 and 3. Calling the public processor with
`NormalizeOrientation`, explicit PNG output and explicit `Metadata::Discard`
returns success, rotates the 3×2 pixels into 2×3 using the first entry and removes
the EXIF metadata. The strict checkpoint promises conflicting known orientations
are refused; accepting this ambiguous payload chooses a destructive interpretation
without giving the caller a refusal.

Regression: `conflicting_orientation_entries_inside_one_exif_ifd_are_refused`,
`core/tests/artwork_pipeline_adversarial.rs:300`. It is deliberately a normal,
unignored test and remains failing until the implementation is corrected. Do not
change the assertion to bless first-entry selection. The engine should inspect
all relevant IFD0 Orientation entries under bounded parsing, reject contradictory
or malformed declarations, and preserve the distinct semantics of thumbnail IFDs.
This review does not require implementing general EXIF preservation.

## Checks actually run

Initial independent suite before the conflict reproduction: **3 passed**, zero
failed, including the normally ignored external-tool oracle explicitly enabled.
The later contradictory-entry canary was run independently: **1 failed** with an
actual successful `ProcessedArtwork` result rather than the required refusal.
Scoped all-feature Clippy with denied warnings, owned rustfmt and whitespace
checks passed. The full workspace was not run: other independent writer/read
canaries are known to be under correction and belong to other scopes.

```sh
cargo test -p tagrex-core --test artwork_pipeline_adversarial -- --include-ignored --nocapture
cargo test -p tagrex-core --test artwork_pipeline_adversarial conflicting_orientation_entries -- --nocapture
cargo clippy -p tagrex-core --lib --test artwork_pipeline_adversarial --all-features --no-deps -- -D warnings
rustfmt --edition 2021 --check crates/core/tests/artwork_pipeline_adversarial.rs
git diff --check -- crates/core/tests/artwork_pipeline_adversarial.rs
```

The passing independent checks establish the following bounded evidence:

- All eight EXIF orientation mappings, each with distinct RGBA samples at both
  8-bit and 16-bit precision. Expected coordinates were specified directly from
  the orientation table. FFmpeg independently decoded every output into raw RGBA
  or big-endian RGBA64, and all sample bytes matched. EXIF appeared both before
  and after IDAT. MIME was deliberately inaccurate to verify byte-based detection.
- Late tEXt metadata is refused under the default re-encoding loss policy,
  retained byte-for-byte by an original no-op and explicitly reported discarded
  when authorized. An unknown critical PNG chunk is refused even with metadata
  discard permission.
- A public progress callback reenters the same processor and clears its cache;
  nested processing succeeds without a lock deadlock. Nested cancellation does
  not publish a result. An escaped immutable image owner remains charged after
  result/processor/cache destruction; its final drop releases all charged bytes,
  and no transient or job reservation survives.

The FFmpeg oracle uses only tiny synthetic in-memory fixtures and pipes; no
external or user images, audio, profiles, library state or Undo history are read
or written. No task-owned temporary directories were created.

## Review bounds and routing

Tier 2 graph discovery used `tagrex-backend-readiness`, generation
`2026-11-18T12:48:55Z`, with outbound processor tracing. Coverage explicitly
reported pipeline source `metadata_changed`, authored integration tests
`not_tracked`, and shared resources `metadata_match`; current source was read
for the complete pipeline and tests rather than trusting stale graph snippets.
No graph completeness claim follows.

The author suite already covers broad mutation/concurrency/budget cases; this
review adds independent geometry and reentrancy evidence and does not repeat its
counts as new review work. It does not establish whole-process RSS limits,
colour-managed filtering, ICC retention, live device behaviour or Windows/macOS
runtime correctness. Picture native identity/type/qualifiers/order, stage/Apply
integration and persistent Undo are outside this pixel-only engine.

The confirmed reusable core fix is eligible for a curated standalone backend PR
to `tagrex/tagrex:main` after correction and integration gates. No commits, PRs,
publication or merge readiness are claimed here.
