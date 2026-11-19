# Structured report rendering core

Local renderer checkpoint; independent review and output/staging integration pending.

The new `report_template` module accepts a versioned Rust data tree. It reuses
Tagnetic's mask language and supplied immutable metadata/technical contexts;
rendering performs no filesystem reads or writes. Its first version consumes
legacy scalar fields explicitly. Exact repeated-value access remains a separate
migration and is not inferred from punctuation.

Supported nodes: literals, mask values, track iteration, nested grouping,
explicit group keys, row counts, known-duration sums and unknown-duration counts.
Groups use encounter order or exact lexical order; members retain input order.
Mask nodes use the first row of their current scope. Iteration narrows the scope
to one row. Missing fields render empty. Values retain punctuation and whitespace
without filename sanitisation or implicit track-number padding.

Plain text, HTML/XML text escaping and quoted CSV cells are explicit. XML text
rejects forbidden XML characters. Output uses the shared strict UTF-8, UTF-16 or
explicit codepage encoder, with explicit BOM policy and no replacement of
unmappable characters. The template author still controls document structure;
this is not a complete HTML/XML/CSV document validator or Mp3tag `.mte` importer.

## Bounds and ownership

Before compilation: 64 KiB aggregate literal/pattern source, 128 nodes, eight
nested scopes, 2,048 bytes per mask and a conservative retained AST allowance.
Rendering limits selection to 100,000 rows, 4,096 fields per row and one million
aggregate fields. It caps group fan-out at 4,096, aggregate active grouping
allocations at 4 MiB and execution visits at two million. Aggregate scans and
per-row grouping/iteration consume this work budget too.

The renderer acquires shared working capacity before allocation; each mask also
uses the new actual-AST work estimate and bounded-render seam. It borrows tag
maps instead of cloning selected metadata. Text is capped at 4 MiB and encoded
output at 8 MiB. Growth, group members, cancellation, callback panic and policy
changes return no partial result. The final encoded result keeps its resource
lease until its last owner drops; its bytes are exposed by borrowing.

These are participating allocations, not an RSS guarantee. Compiled-mask work
accounting is undergoing its own independent verification. The retained policy
revision is preview evidence, not permission to publish stale output.

## Local evidence

`cargo test -p tagrex-core --test report_template -- --include-ignored --nocapture`:
**13 passed**, including both explicit manual probes. Scoped core/report Clippy
with all features and denied warnings passes.

Tests cover nested grouping/order/scope, literal Unicode and punctuation,
strict encodings, all cancellation checkpoints, panic cleanup, final-callback
policy changes, resource pressure/retry, bounded Unicode diagnostics, extract-only mask preflight, invalid/future/oversized trees, group
fan-out, output limits and arithmetic overflow. Independent Python CSV and XML
consumers read the generated Unicode/multiline output across UTF-8/UTF-16.

A local native compute probe rendered 100,000 rows in 100 groups to 1,700,890
UTF-8 bytes. Four samples took approximately **150–152 ms**; compilation was
about **99 microseconds**. Participating transient high-water was **33,563,040
bytes**. All four outputs matched in length/checksum and charges returned to
baseline. This excludes metadata acquisition, IPC, publication and UI work;
it is not a release or cross-platform performance guarantee.

## Remaining delivery

No commit/PR or UI change. Reusable core route: curated upstream
`tagrex/tagrex:main`. Still required: independent adversarial review, command
DTOs and staged snapshot/seal integration, guarded grouped output publication,
append/staleness/fault handling, and native platform/current-head CI evidence.
Existing scalar report exports retain their existing interface.
