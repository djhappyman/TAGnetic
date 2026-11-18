# Query snapshot reuse

Ordinary batched row reads retain the existing query snapshot, facets and
duplicate evidence when both row data and observed filesystem fingerprints are
unchanged. Changed or unavailable fingerprints rebuild the snapshot. Unix
fingerprints include device, inode and ctime; other platforms currently use
length and mtime, with the same limitations as the metadata cache. This is a
display optimization; authoritative write validation remains uncached.

Explicit single-file Refresh, full refresh, Apply, Undo and Redo still supersede
old results. Unchanged batch reads avoid rebuilding the row-vector and derived
caches, but still validate/read the requested paths; this is not zero-I/O refresh.

Verification: `cargo test -p tagrex-core -p tagrex-commands -p tagrex-service`
passed 912 tests (15 ignored). Warning-free Clippy for those crates with
`--all-targets --all-features`, and workspace formatting passed. A disposable WAV
regression proves Arc reuse, isolated returned rows, unchanged media bytes and
Unix ctime invalidation with restored mtime. Existing integration tests cover
Apply/Undo/Redo and explicit Refresh invalidation. The original no-op failure
was reproduced before the fix. No installed runtime or user audio was changed.

Route: reusable backend fix for curated upstream `main`; no publication or
cross-platform filesystem verification is claimed by this local checkpoint.
