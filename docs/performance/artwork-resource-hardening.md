# Artwork resource hardening

Core resize work now admits an image using header dimensions and conservative
estimates for decoded pixels, resize scratch, RGB conversion, row scratch,
decoder quota and encoded-output copies. The working estimate is 256 MiB;
processing is serialized across core resize callers. Full decode also receives
an explicit allocation limit. Row-wise PNG decode retains its memory-saving
path, with a 32 MiB decoder quota and a 512 megapixel work cap. Oversized,
unsupported or failed transforms preserve the original image, with the existing
preview warning when it remains over the configured dimensions.

Compressed input and encoded output are each capped at 16 MiB for transformation.
The JPEG writer refuses growth before crossing the output cap. RGB conversion
consumes the resized buffer where possible; the source image is dropped before
encoding. Successful encoded results are cached by SHA-256 content, dimension
and quality, within 32 entries / 64 MiB, with LRU eviction. No decoded images or
source bytes are retained by the cache. Image type/description always come from
the current picture, so shared bytes cannot carry another picture's properties.

These are admission estimates and library limits, not an OS-enforced RSS cap.
Header parsing, decoder internals, allocator overhead and references retained by
plans have separate lifetimes. Content hashing still reads each supplied byte.
The serialized processing gate can trade parallel latency for lower aggregate
working memory; this checkpoint does not claim a wall-clock speedup.

Verification: 929 core/commands/service tests passed (15 ignored), clean
all-target/all-feature Clippy and formatting. New tests cover extreme dimensions,
row-wise admission, source-pixel work caps, encoded overflow, cache budgets/LRU,
content/options keys and shared output with distinct picture properties. Existing
PNG quality/dimension, tag/artwork Apply, Undo and recovery suites passed on
disposable fixtures. No user media or running app was mutated for these tests.

Route: reusable core fix, eligible for curated upstream `main`.

Review follow-up: header dimension reads also use the decoder allocation quota
and shared processing gate. Oversized compressed inputs are refused before
header parsing. The final combined workspace gate passed 1,148 tests (19 ignored)
with clean workspace/all-target/all-feature Clippy and formatting.
