# Native field recipes

Field recipes copy or build tag values, remove unwanted fields and keep selected
fields. Their implementation lives in Rust. They use Tagnetic's existing plans,
shared stage, field locks, Apply and journaled Undo.

See [native batch operations](native-batch-operations.md) for extracting combined
values, matched text imports, text sidecars, disc numbering and list operations.

The existing **Format values** ribbon command now uses this engine for its common
destination fields. Other native recipe operations are available to Rust callers
and through MCP transformation groups. The graphical preset editor does not yet
provide controls for creating these new rule kinds.

## Operations

- **Format:** put a pattern's result into a standard or custom text field. An
  empty result removes that value. A missing source is empty, so a conditional
  such as `$if2(%albumartist%,%artist%)` can fill a value from a fallback.
- **Remove:** remove the explicitly named text fields. `COVER` or `PICTURE`
  explicitly removes embedded artwork as well. No wildcard matching is implied.
- **Keep:** retain the named text fields and remove other exposed text fields.
  Artwork stays in place. Removing artwork always requires a separate explicit
  Remove operation. Private binary data and whole tag blocks are not text fields.

An empty Remove list changes nothing. An empty Keep list removes all exposed
text fields but still retains artwork. Keep-only rules and ordinary Clear tags
therefore share the same artwork boundary.

## Native API

`App::preview_field_operations(paths, operations, working)` accepts these typed,
serializable operations:

```json
[
  {"kind":"format","field":"custom:Venue","pattern":"The Hall"},
  {"kind":"format","field":"comment","pattern":"%custom:Venue% / %artist%"},
  {"kind":"remove","fields":["ENCODEDBY"]},
  {"kind":"keep","fields":["artist","title","album","comment","custom:Venue"]}
]
```

The optional `working` plan supplies the current desired values. Each step reads
what preceding steps produced. Files and fields outside the selected operation
remain staged; existing renames, sidecars, tag-block operations, artwork assets
and folder settings remain attached to their existing plan.

Patterns and selectors compile before reading any file. Each unique selected
file gets one cached metadata read. Artwork bytes are read separately only when
artwork removal requires its Undo baseline. Invalid recipes fail before producing
a plan. An unreadable selected file fails the operation rather than disappearing
from its result. No write happens during preview.

`preview_field_operations_with_cancel` accepts a cancellation callback, checked
between files and steps. Cancellation returns an error and publishes no partial
plan. Applying the result uses the ordinary executor, progress and cancellation
path. This callback does not add a new desktop Stop control for preview generation.

The desktop forwards the same request through `preview_field_operations` on the
existing background session queue. Format values makes one request, not one
request per field or track, and checks that selection and staged inputs still
match before publishing the reply.

## Saved recipes and MCP

Transformation groups retain their existing JSON shape. The new rule kinds use
these fields:

| Rule kind | Parameters |
| --- | --- |
| `format` | `scope` is the destination field; `to` is the value pattern. The group scope is the fallback destination. |
| `remove_fields` | `from` is a semicolon-separated list of fields to remove. |
| `keep_fields` | `from` is a semicolon-separated list of text fields to keep. |

For example, supply this group to `draft.transform` with an explicit selection:

```json
{
  "name":"Prepare credits",
  "scope":"tags",
  "rules":[
    {"kind":"format","scope":"albumartist","to":"$if2(%albumartist%,%artist%)"},
    {"kind":"format","scope":"comment","to":"%custom:Venue% / %year%"},
    {"kind":"remove_fields","from":"ENCODEDBY;ORIGALBUM"}
  ]
}
```

Existing tag text transformations may be mixed into these groups in order.
Disabled rules do nothing. The same groups can be validated and saved through
`presets.put`. A single-text sample cannot show a recipe that depends on several
fields; use a selected-file preview instead.

Field recipes operate on tags. Do not combine their steps with `filename` or
`fileext` scopes. Those combinations are rejected explicitly; the existing
rename/move operations keep ownership of paths, collisions and sidecars. A
field recipe can still extend a plan that already contains a rename.

## Field names and pattern behavior

Standard field names and aliases are case-insensitive. Supported aliases include
`TRACKNUMBER`, `DISCNUMBER`, `TOTALTRACKS`, `TOTALDISCS`, `INITIALKEY` and `LABEL`.
Native operation targets accept custom names; `custom:` explicitly distinguishes
a custom field from a standard field with the same name.

Custom pattern references use `%custom:NAME%`. Existing custom-name spelling is
retained when updating it. If several stored fields differ only by ASCII letter
case, formatting refuses the ambiguous target. Unknown ordinary placeholders
still report errors. This keeps the existing typo protection and file/technical
placeholder namespace intact.

Formatting field contents preserves slashes, punctuation, Unicode and line
breaks. It does not apply filename sanitisation or implicit track-number padding.
Naming patterns keep their established sanitisation, optional-section behavior
and padding. Explicit padding remains available in both contexts.

Locked intermediate assignments cannot feed later steps. Rejected attempts
still appear in the existing lock feedback. The final plan gate applies field
locks and format-specific value validation,
including unsupported custom fields. Invalid proposals cannot be applied.
Original saved values remain the staleness and Undo baseline. A working text
plan whose original values no longer match disk is rejected during preview.

## Work limits

Patterns support at most 8192 UTF-8 bytes, 32 nested sections/calls and 255
characters of placeholder padding. Results are bounded to 1 MiB. Function counts
and replacement expansion are bounded before large allocations. Native batches
support 256 operations and selectors support 256 field names. A preset saved
through MCP also retains its existing 64-rule limit.

These are explicit resource limits, not truncated results. Existing decimal
arithmetic and function names remain unchanged; this is not an Mp3tag script
interpreter or a claim of full function-language compatibility.

## Mp3tag reference and Tagnetic choices

The implementation was checked against the official [Format Value action](https://docs.mp3tag.de/actions/format/),
[Remove Fields action](https://docs.mp3tag.de/actions/remove-fields/),
[Remove Fields Except action](https://docs.mp3tag.de/actions/remove-fields-except/),
[format reference](https://docs.mp3tag.de/format/) and
[function reference](https://docs.mp3tag.de/scripting/).

The adopted capabilities are cross-field assignment, named-field cleanup and
keep-only lists. Tagnetic deliberately retains its explicit custom namespace,
shared staging and field validation. Unlike Mp3tag's keep-only action, Tagnetic
keeps artwork unless a removal step names it explicitly. Filename/directory
assignment remains with the existing naming tools. Pattern syntax, arithmetic
and save behavior follow Tagnetic, rather than reproducing Mp3tag internals.
