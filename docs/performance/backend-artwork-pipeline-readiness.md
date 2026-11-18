# Strict artwork pixel pipeline checkpoint

18 November 2026. Implementation base `9da71249` plus the shared backend programme
working changes. Owned files: `core/src/cover/pipeline.rs`, its public module
registration in `cover.rs`, this report and `core/tests/artwork_pipeline.rs`.
Existing resize semantics, artwork resource/cache logic, model, plans, journal,
commands and service were not changed by this component. No commits/publication,
UI exposure or full artwork/backend completion are claimed. Reusable core route:
curated `tagrex/tagrex:main` after parent review and integration gates.

## Public behaviour

`cover::pipeline::Processor::new(Budget, Limits)` creates a clonable native engine.
`process`, `process_with_cancel` and `process_with_progress` return a strict
`ProcessedArtwork` or distinct `Unsupported`, `Invalid`, `OverBudget`, `Busy` or
`Cancelled` errors. There is no best-effort fallback to an unprocessed cover when
processing fails. Public stage boundaries are Inspect, Decode, Transform(index),
Encode and Publish; cancellation before publication never returns or caches a
partial result.

Ordered `Step` operations support proportional Fit, exact Resize, centred
SquareCrop, stretched SquareScale and EXIF NormalizeOrientation. Fit floors its
scaled dimensions, with minimum one pixel, and never exceeds its box. Denied
upscaling clamps Fit to its existing size but refuses an exact resize/square
request needing enlargement. Centre crop floors its top/left offsets; an odd
extra removed row/column is discarded on the bottom/right. Orientation may occur
once in the supplied order. Nearest, Triangle and Lanczos3 filters are explicit.

Output is JPEG, PNG or the detected original format. An original-format no-op
validates the static pixel stream, retains the exact original encoded bytes and
shares their immutable backing storage; JPEG quality is not an implicit
recompression request in this mode. Explicit JPEG/PNG output re-encodes even if
the original codec matches. Both encoded and byte-retaining results obey output
size limits. JPEG quality must be 1–100. Alpha is composited onto an explicit RGB
background with rounded integer sample-space arithmetic before JPEG encoding.

The result preserves the input `CoverKind` and description independently of the
image cache. Its immutable `Arc<EncodedImage>` owns its retained lease; result
clones share ownership. No bare byte Arc extraction/conversion to `CoverArt` is
provided, because doing so would let output memory escape the accounting owner.
The existing model's short picture-kind enum is still a limitation: this engine
does not recover raw picture types, qualifiers, stable picture identity, ordering
or container-specific description/icon rules that the model cannot represent.
Native batch/staging integration must solve those seams before using it for
arbitrary picture edits. This engine writes no image files or audio tags itself
and introduces no second Apply path.

## Metadata, colour, animation and precision policies

- Only static PNG and 8-bit JPEG coding profiles handled by the locked decoder
  are supported. APNG is explicitly refused, including original-mode processing;
  unsupported artwork can continue through the existing opaque artwork paths,
  but is not claimed transformable by this engine. Unsupported JPEG coding
  processes and multi-frame JPEGs are refused.
- Re-encoding ancillary/profile metadata is refused by default. Explicit
  `Metadata::Discard` permits dropping it; `Notes.metadata_discarded` reports
  that choice. JPEG APP metadata before SOF and between scans is inspected too.
  Unknown PNG ancillary chunks are treated as metadata, not silently certified.
- This is **raw channel-space filtering**, without ICC/gamma-aware colour
  management or linear-light/premultiplied-alpha resampling claims. PNG text and
  ICC decompression are disabled in the pixel decoder; original-mode bytes retain
  such metadata opaquely, without certifying the profile contents. Re-encoding
  does not claim EXIF/ICC/XMP/DPI retention.
- A nonidentity EXIF orientation requires an ordered normalization step, or an
  explicit Ignore orientation choice, when re-encoding. Normalization changes
  the pixels and removes the orientation tag under the declared metadata-loss
  permission. Conflicting known EXIF orientations are refused. Metadata that
  contains no recognized orientation does not imply a rotation was performed.
- PNG grayscale, grayscale-alpha, RGB and RGBA at 8/16-bit decoder output are
  supported. PNG 16-bit samples are retained for PNG output, including transforms.
  JPEG cannot retain that precision and requires `BitDepth::Allow8Bit`; the
  result reports precision reduction. JPEG is lossy even at quality 100.

## Resources, cache and cancellation

The existing shared `core::resources::Budget` admits work before processing
allocations. Hash/scanning input is charged; decoder work additionally acquires
an expensive job slot. Header dimensions/pixels and every ordered intermediate
are preflighted before decode. Working estimates include input/decoder copies,
16-bit-capable decoded buffers, old/new image ownership, floating resizer scratch,
row weights, colour conversion and both encoded-vector/immutable-output copies.
The encoded writer stops before exceeding its byte ceiling. Decoder constructors
receive bounded input and decoder limits; PNG uses `png::Decoder` directly so
text/ICC decompression cannot silently expand retained metadata.

Default per-request ceilings: input/output 16 MiB each, dimension 8192, 16,777,216
pixels, 256 MiB estimated work, 32 MiB decoder scratch, 32 steps and 1 MiB
picture description. Configuration has hard upper bounds as well. Shared
admission coordinates concurrent retained/transient bytes and expensive jobs;
no worker or retry thread is created here. These are participating-owner
reservations and conservative estimates, **not an allocator/RSS guarantee**;
locked image/png internal max-allocation support is partly best-effort.

The cache is bounded by entries and encoded bytes, contains no decoded images,
and is keyed by SHA-256 with framed input length, all ordered steps/options and
an explicit locked-codec/pixel-policy identity. Result properties do not enter
that key, so two picture descriptions share pixels without acquiring each
other's metadata. Cache metadata is reserved before storage allocation. New
result ownership is admitted before unrelated useful entries are evicted; failed
admission leaves those entries intact. Evicted results held by clients remain
charged. `clear_cache` releases cache-owned images while retaining the leases of
live client owners. There is no retained weak-pointer payload workaround.

Cancellation is checked while hashing/scanning, between decoding and steps,
at alpha-compositing rows, on encoder writes and before publication. It cannot
forcibly interrupt a codec call already executing. Public callbacks execute
outside cache locks; panic unwinding releases work admission. No external file,
active profile, media library or Undo state is mutated.

## Verification evidence

Existing toolchain/dependencies/cache were reused via `dev-env.sh`:

```sh
cargo test -p tagrex-core --test artwork_pipeline
cargo test -p tagrex-core --test artwork_pipeline independent_encoded_pixels_and_original_bytes_roundtrip -- --ignored --nocapture
cargo test -p tagrex-core --test artwork_pipeline bounded_artwork_scale_probe -- --ignored --nocapture
cargo clippy -p tagrex-core --lib --test artwork_pipeline --all-features -- -D warnings
rustfmt --edition 2021 --config skip_children=true --check crates/core/src/cover/pipeline.rs crates/core/src/cover.rs crates/core/tests/artwork_pipeline.rs
git diff --check
```

Observed caller suite: **21 passed, zero failed, two ignored**. Both ignored probes
were explicitly run and passed. Scoped core/artwork Clippy, formatting and
whitespace checks passed. Parent owns the combined gate. A broader core run was
already launched before the parent's instruction to avoid it; it stopped on the
parent's intentionally failing `metadata_preservation_recheck` untouched-value
writer reproduction. That failure is unrelated to pixels and is not reported as
a passing whole-core gate. An earlier all-target Clippy run also correctly caught
an unrelated in-progress `recovery_stress` modulo lint; no foreign code was edited
or warning suppressed.

Tests exercise manually asserted crop pixels for portrait/landscape odd margins,
7×3 Fit rounding, tiny no-upscale retention, explicit stretch/upscale, ordered
steps and cache option isolation; alpha-to-JPEG background with lossy tolerance;
no-op pointer/byte equality and separate kind/description; EXIF normalization and
profile-loss refusal; PNG16 exact sample preservation/JPEG loss permission;
invalid headers and pixel payloads, APNG refusal, dimension bombs, decoder scratch
pressure and output limits (including no-op). Ordered intermediate work is
refused before decode even when the final image would be tiny.

Stress includes 2,000 deterministic PNG/JPEG truncations and header/payload bit
mutations under reduced dimensions/work/output limits; no panic, leaked job or
transient charge is accepted. A successful tolerant decoder result is required
to satisfy bounded dimensions/output, not assumed corrupted merely because bits
changed. Twelve concurrent callers vary formats/options and pressure the shared
job/byte budget; a gated worker proves Busy is nonblocking and cancellation
returns capacity. Tests verify LRU retention, failed retained admission preserving
hot entries, client-held image ownership after eviction/cache-clear/engine-drop,
all public cancellation stages, and panic cleanup without cache poisoning.

The independent probe uses FFprobe and FFmpeg against synthetic outputs: exact
3×3 RGBA pixels after crop/nearest scaling, JPEG compositing against blue within
four sample levels, and exact 16-bit RGBA big-endian samples after resize. Original
no-op input bytes remain identical. Task-owned probe directories are removed
once assertions pass. No externally sourced or user images were used.

One bounded scale observation: synthetic 1024×768 PNG **1,484,479 bytes** → 512×512
JPEG **327,257 bytes**. First processing **124.005 ms**; seven cache hits median
**0.654 ms**, maximum **0.666 ms**, with shared immutable result identity asserted.
Estimated shared high-water: **88,605,757 transient bytes**, **331,649 retained
bytes**, one expensive job. This development-profile component probe includes
hashing/processing/admission; it is not process RSS, IPC, UI, Apply or cold-disk
performance evidence, and does not establish a universal speedup.

Tier 2 discovery initially used the stale other-checkout `tagrex-personal-app`
graph and read exact current sources for flagged files. The parent subsequently
indexed this checkout as `tagrex-backend-readiness`, generation
`2026-11-18T12:48:55Z`; fresh coverage should be checked on cited paths during
independent review. Locked image 0.25.10 and png 0.18.1 primitive sources were read
locally for allocation, precision, metadata and encoder behaviour. No graph
completeness or full artwork preservation claim follows from this checkpoint.
