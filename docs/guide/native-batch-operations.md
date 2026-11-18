# Native batch operations

This checkpoint adds Rust operations for extracting fields from other values,
matching text rows to files, importing accompanying text, numbering multiple
discs and operating on the four existing list fields. It builds on
[native field recipes](field-recipes.md).

These operations produce the same change plans as ordinary editing. They use
current staged values, field locks, validation, Apply and journaled Undo. They
preserve unrelated staged fields, files, artwork and rename settings. No audio
is written during preview. This is backend work; new desktop controls and the
replacement of the desktop's numbering calculations remain pending.

## Extract combined information

For example, a title containing `Björk - Jóga` can fill Artist with `Björk` and
Title with `Jóga`:

```json
{"kind":"extract","source":"%title%","pattern":"%artist% - %title%"}
```

The source can be any renderable Tagnetic expression, including custom fields
and functions. Extraction uses our existing filename-to-tags grammar. `%skip%`
discards an unwanted portion. Captures are read before assignments, so a source
can also be a destination. A pattern mismatch leaves that file's fields in place
and reports it in `plan.unrendered`. Subsequent steps still run against its
unchanged fields. Empty captures do not erase existing fields, consistent with
our text imports. Use an explicit removal step to clear a field.

Patterns with ambiguous boundaries, render-only functions in the extraction
pattern, repeated destination fields or no destination fields fail preflight,
including when the selection is empty. Locked extracted values cannot feed later
steps. Native recipes use `kind: extract`, `from` for the source expression and
`to` for the extraction pattern in the existing saved-rule shape.

Mp3tag calls this [Guess Values](https://docs.mp3tag.de/actions/guess/): it splits
a source expression using a stated pattern. It does not infer missing metadata.
Our operation stages its result for review instead of saving it immediately.

## Match text rows to files

`App::preview_text_import(paths, text, options, working)` returns a plan,
physical line numbers, row statuses, explicitly supplied field names and the
selected files without matching rows. Its cancellation variant publishes no
partial result.

```json
{"pattern":"%filenameext%|%artist%|%title%","mode":"filename"}
```

Modes are `sequential`, `filename` and `path`:

- Sequential mode pairs matching nonblank rows with unique selected files in
  explicit order. Nonmatching headings do not consume a file.
- Filename mode uses `%filenameext%`, including the extension. Two selected files
  with the same filename are an error; use full paths to distinguish them.
- Path mode uses `%filepath%` and the exact selected path. Rows cannot target a
  file outside the selection.

The matching modes also accept Mp3tag's `%_filename_ext%` and `%_path%` aliases.
Exactly one matching placeholder is required. Matching is exact and
case-sensitive, avoiding assumptions about the source filesystem. Duplicate
rows targeting one file fail the whole preview. Blank rows are ignored;
nonmatching rows, unselected targets, surplus sequential rows and unmatched
selected files are reported. Empty captured fields do not clear saved values.
This is a delimiter-pattern importer, not a quoted CSV parser.

The selection index and extraction mask are built once. The MCP `draft.from_text`
tool uses this engine and accepts `mode`. Its diagnostics survive live draft
projection and remain available through `draft.get`.

The behavior follows Mp3tag's documented
[text import modes](https://docs.mp3tag.de/converters/import-tags-from-text-files/),
using Tagnetic's pattern language and current stage.

## Import accompanying text

`App::preview_text_sidecars(paths, options, working)` imports the complete text
of a file into a chosen standard or custom text field:

```json
{"field":"custom:LYRICS","filename":"%filename%.txt","skip_missing":true}
```

The filename is rendered from current staged tags and the current audio path.
It is relative to that audio file's folder, including any contained subfolder.
Whole-file line breaks, Unicode and trailing newlines are retained. An empty
text file explicitly clears the destination. UTF-8 and BOM-marked UTF-16 in
either byte order are supported. Invalid encodings, odd UTF-16 byte counts,
unpaired surrogates, NULs, oversized files and nonregular files fail.

Absolute paths, parent traversal and Windows drive/stream syntax are rejected.
On Unix, opening uses pinned directories and a nonblocking, no-follow file
handle. On Windows, ancestors are pinned and final reparse points are refused.
An escaping parent link or final file link cannot redirect the read. The native
boundary intentionally confines derived paths rather than accepting arbitrary
absolute paths from tag contents.

Only genuinely missing files may be skipped with `skip_missing`; they are
reported. Permission errors, unsafe links and invalid files always fail. A
cancellation callback is available. MCP exposes `draft.from_sidecars` and also
requires a read grant covering each audio folder. It does not grant filesystem
access merely because text mentions a path.

This covers the purpose of Mp3tag's
[Import Text File action](https://docs.mp3tag.de/actions/import-text/) while
respecting our existing review and filesystem boundaries.

## Number tracks and discs

`App::preview_numbering(paths, options, working)` is the shared native engine.
MCP `draft.number` now uses it. The current desktop numbering implementation
continues to work; connecting its controls to this engine is UI work.

```json
{
  "start":1,"step":1,"width":2,"write_total":true,
  "disc":1,"disc_width":2,"write_disc_total":true,
  "next_disc_on":{"kind":"folder"},
  "reset_disc_on":{"kind":"field","field":"album"},
  "reset_track_on_disc":true,"keep_vinyl":true
}
```

Track/disc starts, step size, zero padding and explicit totals are supported.
`next_disc_on` advances the disc when a folder or chosen field changes.
`reset_disc_on` starts a new release, restoring the starting disc and track.
`reset_track_on` independently restarts tracks on a folder or field change.
`reset_track_on_disc` controls whether advancing the disc restarts tracks.

Boundaries compare consecutive eligible files in the supplied order; they do
not sort files or gather separated occurrences of the same album. Missing
boundary fields compare as empty. Custom-field names are supported and ambiguous
case variants are refused. A release reset takes precedence over a simultaneous
disc advance. Vinyl positions are retained by default, reported and excluded
from counter consumption. Duplicate selected paths consume only one number.

Automatic track totals use the highest generated track number in each counter
group; disc totals use the highest disc number in each release group. Starting
at track 5 with two files therefore produces tracks 5 and 6, total 6. Explicit
totals below a generated number are rejected. Counters use checked arithmetic;
zero starts/steps, overflow and padding wider than 12 are rejected. Container
validation still applies: MP4 stores numbers as integers and cannot preserve
leading zeroes or values outside its supported range.

These controls cover the rules in Mp3tag's
[numbering documentation](https://docs.mp3tag.de/converters/auto-numbering/),
adapted to explicit selection order and staged values.

## Split, merge and deduplicate values

The native `FieldOperation` and saved recipes support:

- `split_values`: split an explicitly selected field using a literal separator,
  trim the resulting values and omit empty pieces. Native options use `field`
  and `separator`; saved rules use `scope` and `from`.
- `merge_values`: join separate values into one using a chosen separator,
  including an empty joiner. Saved rules use `scope` and `to`.
- `deduplicate_values`: remove exact duplicates, retaining the first occurrence
  and order. Comparison is case-sensitive; different spellings remain.

These currently operate on Artist, Album Artist, Genre and Composer, the four
fields our storage already treats as lists. They never split ordinary titles,
comments or arbitrary custom fields merely because those contain punctuation.
Operations are ordered and can be combined with extraction, formatting,
removal and existing text transformations in one saved recipe.

The new core `FieldValues` type holds an ordered list independently of a
container's separator convention. Its adapter to our existing joined string is
fallible: a merged single value containing the configured list separator is
refused rather than written back as several values. Exact values, meaningful
whitespace and empty entries can be represented by the typed foundation even
when the legacy adapter cannot encode them.

Mp3tag documents [splitting](https://docs.mp3tag.de/actions/split-field/),
[merging](https://docs.mp3tag.de/actions/merge-duplicate-fields/) and
[duplicate-field removal](https://docs.mp3tag.de/actions/remove-duplicate-fields/).
Its duplicate removal can discard differing repeated values. Our operation
removes exact duplicate content only. General repeated-field editing remains
unfinished: expanding it requires lossless container read/write, staged diff,
journal, recovery and Undo representations. The typed foundation does not claim
that migration is complete.

## Bounds and remaining backend work

Native selections are limited to 100,000 entries; duplicate paths are processed
once. Text input and each accompanying file are limited to 1 MiB. Sidecar import
data is limited to 64 MiB per batch. Field recipes limit effective text to 8 MiB
per file and changed text, including Undo baselines, to 64 MiB per batch. Lists
hold at most 1,024 values and 1 MiB. Patterns retain the existing compilation,
nesting and output limits. Limits fail the preview before publication.

The wider roadmap still has substantial Rust work. In particular, this
checkpoint does not implement generalized list storage, quoted CSV column
mapping, artwork crop/format/property pipelines, playlist loading, complete
export templates/encodings/append behavior, chapter writing, additional audio
containers, configuration portability or Mp3tag scripting/configuration
compatibility. Those capabilities require their own storage, writer or output
work and verification; they are not merely waiting for buttons.

The current checkpoint's [verification record](../performance/native-batch-operations.md)
states the tested scope and platform limits.
