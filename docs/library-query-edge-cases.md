# Query command bar: edge-case checklist

This is a validation and design backlog, not a claim that every case below is
implemented. Keep the typed text available, make proposed meaning visible before
selection, and never infer permission to modify audio files from a search.

## Artists and credits

- Hixxy alone, Hixxy & Sharkey, Hixxy / Dougal, Hixxy, Seduction.
- Reversed credit order; three or more collaborators; repeated names in one tag.
- Semicolon/newline multi-value tags and native tag formats flattened into a DTO.
- feat., feat, ft., ft, featuring, with, w/, vs., versus, x, pres. and presents;
  mixed capitalization, parentheses, brackets and missing/repeated spaces.
- DJ/MC guests, producers and remix credits; a title-only guest must not silently
  become the track artist, and a remixer is not automatically a primary artist.
- Real names containing separators: AC/DC, Above & Beyond, Chase & Status,
  Earth, Wind & Fire. A separator is evidence, not a canonical artist identity.
- Hixxy versus Hixxy2: contains deliberately includes both; exact member matching
  must remain a separate, clearly named operation.
- Artist versus Album artist; Various Artists; blank Artist with Album artist set.
- Aliases, stage names, changed names and DJ/The prefixes must not merge silently.
- Case, repeated whitespace, apostrophes, smart quotes, hyphens, emoji and scripts
  other than Latin. Canonically equivalent Unicode and optional accent-insensitive
  matching need an explicit normalization policy.

## Genres and categories

- Hardcore versus Happy Hardcore; House versus Deep House.
- Drum & Bass, R&B and Rock/Pop may be genre names, not separable collaborators.
- Multiple genres with real list separators; duplicate genre values per file.
- DnB, D&B and Drum and Bass are possible aliases, not automatic equivalents.
- Exact category versus partial text: show the operator that the chip will use.
- Mixed capitalization produces one category choice and distinct-file counts.

## Text, titles and query syntax

- Titles such as Missing You, Live or Die, 1999, Unknown and Various Artists are
  ordinary text unless the user chooses a proposed rule.
- Literal colons, quotation marks, slashes, commas, ampersands and parentheses.
- Quoted words such as "or", "and" and "missing artwork" must not accidentally
  become Boolean operations or missing-tag rules.
- Unclosed quotes, escaped quotes, incomplete field prefixes and pasted queries.
- A colon in a filename/URL; an unknown field-looking prefix; custom field keys
  that themselves contain colons. Prefer the longest known prefix.
- Old sessions/presets containing regex or case flags must not reactivate hidden
  modes. Search is now consistently literal and case-insensitive.
- Oversized or malformed query rules must keep the last complete results and
  block applying a scope that has not been validated.

## Numeric metadata and units

- Blank is different from zero; unknown is different from a measured zero.
- 07, 7/12, A1 and disc totals are not interchangeable numeric strings.
- 1996 versus 1996-05-12; incomplete/invalid dates; original-release year.
- 170 versus 170.0 BPM; half/double-time 85/170 are not automatically the same.
- 3:30 versus 210 seconds; milliseconds, hours, decimal durations and units.
- 44.1 kHz versus 44100 Hz; kbps versus bits/s; VBR versus measured bitrate.
- Decimal comma, thousands separators, negative numbers, NaN/infinity, overflow.
- Equal range endpoints, reversed endpoints, inclusive boundaries and no matches.

## Suggestions and counts

- No popup on focus/empty input; delay after typing; composition/IME input.
- One contains rule per field, including all matching credit variants.
- Count files once, even when a name appears several times in that file.
- Apply other chips to counts; retain explicit nested and excluded conditions.
- Expanding a same-field OR rule must offer useful alternatives outside its current
  value without dropping unrelated constraints.
- Counts in duplicate mode must agree with its reviewed scan scope.
- No matches, one match, all matches; empty/missing values; unreadable files.
- Latest typed query wins; stale responses after clear, Enter, Escape, mode changes,
  preset recall or library replacement must not reopen a popup.
- Stable ranking and bounded rows; long values wrap; counts never imply confidence.

## Chips, groups and presets

- OR values in one chip versus AND between chips; nested all/any groups.
- Excluding A or B versus excluding A and B; missing values under negation.
- Duplicate chips/values, empty groups, duplicate IDs, maximum depth/node limits.
- Removing the final chip preserves remaining text and selected files.
- Editing/cancelling a chip preserves the original rule and returns focus.
- A preset with no chips, no text, legacy flags, custom columns or duplicates mode.
- Missing/deleted custom fields, renamed columns, changed mask definitions.
- Old preset/session versions, corrupt storage, quota failure and deleted presets
  reappearing from an old migration source.
- Presets store query intent, never selection, pending edits or scanned confidence.

## Duplicate evidence

- Identical bytes under different names/paths; equal sizes with different bytes.
- Same audio with different tags/artwork; a tag-only change must not imply a new
  recording. Full-byte equality and decoded-audio equality stay distinct.
- Lossless versus lossy copies; encoder padding, sample rates/channel layouts,
  clipping, gain changes and re-encodes.
- Radio edit, original mix, extended mix, remix, live version and remaster.
- Same first two minutes with a different ending; long mixes, tape sides and sets.
- Silence, very short recordings, low-information fingerprints and unknown duration.
- Similarity A↔B and B↔C does not prove A↔C; avoid transitive false groups.
- Corrupt/unsupported codecs, truncated files, files disappearing or changing during
  scanning, network/removable drives, symlinks and permissions.
- Cancel during startup/stamps/decoding/comparison; start another scan; change
  library or Apply/Undo mid-scan; cached evidence must expire appropriately.
- Filtering one member keeps companions visible and makes count semantics clear.
- “Likely” remains evidence for review; it never authorizes deletion or overwriting.

## Runtime, safety and scale

- Searches use saved tags; pending edits and transformed previews stay separate.
- Hidden selected files remain selected and are clearly reported.
- Empty/invalid/pending queries cannot silently broaden Apply's target set.
- Apply, Undo, refresh, rename, removal and library changes invalidate stale indices.
- Manual sorting/reordering cannot change what an index number identifies.
- Rapid selection, typing, preset recall and scan cancellation under load.
- 10k/50k/100k libraries; high-cardinality tags; broad queries returning every index;
  long metadata and constrained memory. Measure cold setup separately from warm work.
- Virtualization, keyboard focus, drafts and scroll position survive table updates.
- Light/dark palettes, muted chip contrast, narrow/short windows, large type,
  keyboard-only operation, screen readers and reduced motion.

Highest-priority gates are ambiguous names, distinct counts, stale responses,
saved-versus-pending metadata, and duplicate evidence that could otherwise mislead
someone into treating two different recordings as interchangeable.
