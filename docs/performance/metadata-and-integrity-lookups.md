# Metadata sharing and integrity lookup work

Validated metadata cache hits now return shared immutable TrackRead snapshots.
Property-only reads and duplicate grouping borrow this snapshot instead of
copying all tags and block metadata. Callers that need an owned editable tag map
or IPC row still explicitly clone their view. Fingerprint checks, byte/entry
budgets, eviction and write/settings invalidation remain unchanged. A regression
proves cache hits share the exact snapshot and caller copy-on-write mutation
leaves the cached value unchanged.

Whole-block Apply/Undo builds a borrowed path index once instead of scanning
every plan change for each fingerprint. Cover Undo builds one set of paths
already covered by whole-file fingerprints. Empty block-fingerprint work skips
index construction. Lookup behavior preserves the original first matching
change, including defensive handling of malformed duplicate paths.

Whole-file integrity proof is unchanged: hashes still stream all bytes from
authoritative files before/after writes, and Undo still refuses later external
changes. No cached mtime/size shortcut replaces those checks. Existing block,
artwork, rollback, interrupted-operation and reopened-journal suites verify the
transaction behavior. This removes copies and repeated lookups; no measured
end-to-end speedup or reduced integrity I/O is claimed.

Route: reusable commands/core fixes for curated upstream `main`. The running
development shell remains on the earlier verified handoff build.
