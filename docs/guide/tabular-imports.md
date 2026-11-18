# Native CSV and TSV imports

`App::preview_tabular_import(paths, bytes, options, working)` maps tabular data
into the existing field-plan engine. It uses current staged values, field locks,
container validation, normal Apply and journaled Undo. Preview does not write
audio or publish changes to the shared stage. The cancellation variant returns
no partial result. Desktop controls are still pending.

Options specify the ASCII delimiter and quote (comma and double quote by
default), whether a header is present, an explicit text encoding, matching mode
and column mappings. Column indices are zero-based. Header references are exact
and case-sensitive; headers must be nonempty and unique. Each destination is a
standard alias or explicit `custom:NAME`. Two mappings cannot assign the same
logical destination, even through different aliases.

```json
{
  "delimiter": ",",
  "quote": "\"",
  "headers": true,
  "encoding": "utf-8",
  "mode": "filename",
  "match_column": {"kind": "header", "name": "File"},
  "mappings": [
    {"column": {"kind": "header", "name": "Title"}, "field": "title", "empty": "skip"},
    {"column": {"kind": "index", "index": 2}, "field": "comment", "empty": "clear"}
  ]
}
```

Quoted cells support delimiter characters, line breaks and doubled quotes.
Whitespace is significant. CRLF, LF and CR record boundaries are accepted and
embedded newline bytes are retained. Empty physical lines outside quoted records
are ignored; a quoted empty cell is a data record. All data records must have the
header's width, or the first record's width when there is no header. Malformed
quotes, missing mapped columns and inconsistent widths fail with one-based
logical-record, physical-line and column coordinates. No formulas are executed.

Empty cells default to `skip`, preserving the current staged value. `clear`
explicitly removes that destination; `reject` refuses empty cells. A missing
column is always an error. A supplied value equal to the saved value can revert
that field's earlier staged proposal while retaining unrelated pending edits.

Sequential matching follows the explicit unique selection order. Filename and
full-path matching require a match column and use exact strings, preserving the
existing text-import matching rules. Ambiguous selected basenames and repeated
records assigning one file fail. Imported paths never grant filesystem access.
Unselected rows and selected files without rows are reported. Each row reports
its physical starting line and explicitly assigned fields.

The MCP tool `draft.from_table` uses the same implementation and normal shared
draft permission/composition rules. Supply `table` options and exactly one of
UTF-8 `text` or encoded `data_base64`. Other encodings require base64 plus the
explicit encoding in `table`. The service retains its smaller argument limit;
it cannot accept arbitrarily large native batches through JSON. Busy resource
admission is distinguished from malformed table input.

Native limits: 8 MiB raw input and decoded text, 1 MiB per cell, 256 columns and
mappings, 100,000 selected entries/data records, and a 64 MiB intermediate
accounting budget. Mapped-value expansion is charged before cloning. Shared
transient/job leases cover decode/current-record buffers, retained intermediates
and an additional field-plan construction allowance. Field-plan text/Undo limits
still apply. Accounting is conservative policy, not an allocator/RSS guarantee;
final plan retention belongs to its owning session. Cancellation is checked
between records and at bounded parsing intervals, not inside filesystem calls.

Arbitrary repeated-value/index mappings await the exact-value storage migration.
The parser does not split cells into lists. Assignments to the four existing
list fields still use the application's configured legacy separator semantics;
literal-separator distinctions require the exact-value migration. No per-record
text trimming, automatic file discovery or whole spreadsheet application
automation is provided.
