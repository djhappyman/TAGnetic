# Mp3tag official feature catalogue — 2026-11-17

## Scope and evidence rules

Research date: **17 November 2026**. Inventory of Windows Mp3tag and the separate native macOS app, based on official product sites and documentation fetched during this research. This is a documented-capability inventory, not an installation test or a guarantee about every file/tag combination. This catalogue records the external-source research. The companion comparison audits the local implementation and proposes adoption priorities. No audio writes, implementation changes, commits or PRs were made; upstream route is research only.

Windows sources: [product site](https://www.mp3tag.de/en/) and [documentation](https://docs.mp3tag.de/). macOS sources: [product site](https://mp3tag.app/), [features](https://mp3tag.app/features/) and the developer-authored **Mac → Documentation** articles linked by the [official support page](https://mp3tag.app/support/). Ordinary forum replies and third-party articles were not used as feature evidence. Linked documentation pages can be edited over time; this is a research-date snapshot, not an archived release specification. No inferred latest-version number or promised future functionality is used.

**Interpretation:** “editable metadata” means the application documents reading/writing that field; it does not establish an audio-analysis engine, encoder, media player, chapter authoring system, or all-format support for that field. “Not established” below means the reviewed official documentation does not establish parity, not proof of absence.

## 1. File formats and tag standards

### Windows format matrix

The [official format matrix](https://docs.mp3tag.de/file-formats/) separates read, write and remove support:

| Files | Reads | Writes | Removes |
| --- | --- | --- | --- |
| AAC, MP3, TTA | ID3v1, ID3v2, APEv1/v2 | ID3v1, ID3v2, APEv2 | All listed read types |
| AIF/AIFC/AIFF, DSF | ID3v2 | ID3v2 | ID3v2 |
| APE, MPC, OFR/OFS, TAK, WV | ID3v1, APEv1/v2 | ID3v1, APEv2 | ID3v1/v2, APEv1/v2 |
| FLAC | Vorbis Comments | Vorbis Comments | Vorbis Comments, ID3v2 |
| OGG, OPUS, SPX | Vorbis Comments | Vorbis Comments | Vorbis Comments |
| MP4/M4A/M4B/M4V; ALAC container family | MP4/iTunes metadata | Same | Same |
| WAV | RIFF INFO, ID3v2 | Same | Same |
| WMA | ASF/Windows Media | Same | Same |
| MKA/MKV, WebM | Matroska tags, chapters, image attachments | Same | Same |

The matrix does not mean every listed tag type is written to every format. In particular, APEv1 reading/removal is not APEv1 writing.

Windows ID3 settings independently select read/write/remove types. Only one tag type is displayed at a time; APE is prioritized above ID3v2 above ID3v1. Write choices include ID3v2.4 UTF-8, ID3v2.3 UTF-16 LE, and ID3v2.3 ISO-8859-1; an optional rule writes ID3v2 only when ID3v1 is insufficient. APEv1 is upgraded to APEv2 on writing. [Mpeg settings](https://docs.mp3tag.de/customization/options/tags/mpeg/)

### macOS formats

The Mac feature page lists MP3, MP4, M4A, M4B, M4V, MKA, MKV, FLAC, OGG, OPUS, AIF, DSF, MPC, WV, WAV and WebM, with bulk editing of ID3v1, ID3v2.3/v2.4, iTunes MP4, Vorbis Comments and APE tags. Do not transfer Windows-only matrix entries such as WMA, OptimFROG, TAK or Speex to Mac without further evidence. [Mac features](https://mp3tag.app/features/)

## 2. Arbitrary fields, mappings, multiple values and preservation

- **Flexible field editor:** Windows Extended Tags edits supported standard and user-defined field names, multiline values and cover art for one or multiple files. It supports field cut/copy/paste and previous/next navigation. Differing values in a batch show a keep marker. [Extended Tags](https://docs.mp3tag.de/extended-tags/)
- **Format-specific mappings:** common names map to ID3 frames, MP4 atoms, Matroska targets, ASF and RIFF fields. Vorbis Comments and APEv2 normally use their stored names; custom mappings can normalize names across formats. The reference includes BPM, AcoustID identifiers/fingerprints, MusicBrainz identifiers, lyrics, sorting and specialist metadata. These entries establish metadata handling, not calculation of BPM, fingerprints or lyrics. [Field mappings](https://docs.mp3tag.de/mapping/)
- **Mac multivalues:** multiple fields of the same kind are represented with a double-backslash separator in the UI; that separator itself is not stored as field content. The Tag Panel and Extended Tags support single/batch editing. [Mac basics](https://community.mp3tag.de/t/basics-and-overview/51957)
- **True multivalue operations:** merging fields joins values using a separator; splitting makes separate values; removing duplicates can retain the first field and optionally restrict removal to identical contents. These are distinct operations, not merely display formatting. [Merge](https://docs.mp3tag.de/actions/merge-duplicate-fields/), [split](https://docs.mp3tag.de/actions/split-field/), [duplicate removal](https://docs.mp3tag.de/actions/remove-duplicate-fields/)
- **Compatibility/preservation:** Windows can store MP4 multivalues inside one parent atom with multiple data atoms; reuse unchanged MP4 atoms and ID3v2 frames; configure ID3 comment language and rating display. It can list existing MP4/Matroska chapters as separate entries for chapter-title editing, or expose global tags. This establishes an editing view of existing chapters, not a complete chapter timeline authoring suite. [Advanced tag settings](https://docs.mp3tag.de/customization/options/tags/advanced/)
- **Mac settings:** separate MP3 read/write/remove controls; editable field mappings; MP4 multivalue storage choice; reuse of unchanged MP4 atoms and ID3v2 frames. Reuse options concern retained representation and compatibility; they do not imply semantic editing of arbitrary unsupported frames. [Mac preferences](https://community.mp3tag.de/t/preferences-and-configuration/56296)

## 3. Artwork

Windows supports adding, removing, replacing and extracting embedded images; clipboard cut/copy/paste; cover type and description; display aspect ratio; drag/drop; and batch operations. Supported image extensions encountered by the Add Cover chooser include JPEG, PNG and WebP. Replacement retains the selected cover's position, type and description. Type support varies by tag format. [Cover menu](https://docs.mp3tag.de/menus/cover/)

The Windows Manage Covers dialog provides multiple-cover ordering, batch presence indicators and applying a cover found in only some selected files to all selected files. [Manage Covers](https://docs.mp3tag.de/manage-covers/)

Adjust Cover can change image format to JPEG/PNG or retain it, set quality, target minimum/maximum pixel size, scale proportionally, scale to square, or crop centrally to square. These are actual image transforms. [Adjust Cover](https://docs.mp3tag.de/actions/adjust-cover/)

Batch artwork import resolves relative/absolute filenames from tag placeholders. Batch export derives names similarly, chooses the extension from image MIME type, and can select cover type and duplicate-export behavior. [Import Cover](https://docs.mp3tag.de/actions/import-cover/), [Export Cover](https://docs.mp3tag.de/actions/export-cover/)

Mac documents multiple embedded covers, navigation and bidirectional drag/drop, plus downloading/adding art. Its action documentation explicitly describes resizing, JPEG/PNG conversion, square crop and cover property editing. Do not assume every current Windows size/quality or Manage Covers control has a Mac counterpart. [Mac basics](https://community.mp3tag.de/t/basics-and-overview/51957), [Mac product](https://mp3tag.app/), [Mac actions](https://community.mp3tag.de/t/quick-actions-and-action-groups/51882)

## 4. Actions and reusable automation

Windows supports immediate Quick Actions and named, ordered Action Groups; enable/disable groups, selection presets, duplication, rearrangement, mnemonic access and menu subgroups. Changes save when actions run. [Actions](https://docs.mp3tag.de/actions/)

Complete documented Windows action-type inventory:

| Action | Actual function / advanced detail | Official source |
| --- | --- | --- |
| Adjust Cover | Image format, quality, sizing and crop/scale transforms | [Details](https://docs.mp3tag.de/actions/adjust-cover/) |
| Case Conversion | Lower, mixed, sentence, upper; configurable word beginnings; special targets include all tags, filename, directory, and APEv2/Vorbis field names | [Details](https://docs.mp3tag.de/actions/case-conversion/) |
| Convert Codepage | Reinterpret selected fields from a specified legacy charset to Unicode | [Details](https://docs.mp3tag.de/actions/convert-codepage/) |
| Export | Run a text export configuration; executes after other actions regardless of list position | [Details](https://docs.mp3tag.de/actions/export/) |
| Export Cover to File | Parameterized external artwork extraction | [Details](https://docs.mp3tag.de/actions/export-cover/) |
| Format Value | Compose/copy/assign fields or rename files/directories using expressions | [Details](https://docs.mp3tag.de/actions/format/) |
| Generate Playlists | Partition selected files into multiple playlists; set-level work runs at group end | [Details](https://docs.mp3tag.de/playlists/) |
| Guess Values | Parse a source format string into fields using a target matching pattern | [Details](https://docs.mp3tag.de/actions/guess/) |
| Import Cover from File | Embed artwork found by literal or parameterized path | [Details](https://docs.mp3tag.de/actions/import-cover/) |
| Import Text File | Put a text file's contents into a selected field, e.g. externally supplied lyrics | [Details](https://docs.mp3tag.de/actions/import-text/) |
| Merge Duplicate Fields | Join multiple values into one value with selected separator | [Details](https://docs.mp3tag.de/actions/merge-duplicate-fields/) |
| Remove Duplicate Fields | Keep first value; optional identical-content restriction | [Details](https://docs.mp3tag.de/actions/remove-duplicate-fields/) |
| Remove Fields | Delete selected fields; COVER/PICTURE targets artwork | [Details](https://docs.mp3tag.de/actions/remove-fields/) |
| Remove Fields Except | Retain an allowlist, including optional artwork | [Details](https://docs.mp3tag.de/actions/remove-fields-except/) |
| Replace | Literal replacement, optional whole-word and case sensitivity | [Details](https://docs.mp3tag.de/actions/replace/) |
| Replace with Regular Expression | Pattern replacement, capture references and case control | [Details](https://docs.mp3tag.de/actions/replace-regexp/) |
| Set Cover Properties | Type/description changes for all covers or a selected type; preserve markers | [Details](https://docs.mp3tag.de/actions/set-cover-properties/) |
| Split Field by Separator | Make separate values, trimming surrounding spaces | [Details](https://docs.mp3tag.de/actions/split-field/) |

Mac's action inventory: Adjust Cover, Case conversion, Convert codepage, Export Cover, Format tag field, Import Cover, Import tag fields, Import text file, Merge duplicate fields, Remove duplicate fields, Remove fields, Remove fields except, Replace, Replace with Regular Expression, Set Cover Properties, Split tag field. Groups can be organized into categories and applied by category. Special targets include tags, filename, directory and tags-plus-filename. Mac regex uses ICU. The Mac page does **not** list Windows's Export or Generate Playlists actions; its Import tag fields covers patterned field extraction. [Mac actions](https://community.mp3tag.de/t/quick-actions-and-action-groups/51882)

## 5. Converters, import and numbering

Windows converters:

- **Tag → Filename:** generate filenames and directory trees, with expressions, relative/absolute paths and numeric padding. Relative creation uses the working directory. [Rename](https://docs.mp3tag.de/converters/rename-files/)
- **Filename → Tag:** extract from filenames and parent folders using matching placeholders; discard unmatched/unwanted portions with dummy placeholders. [Import filenames](https://docs.mp3tag.de/converters/import-tags-from-filenames/)
- **Text File → Tag:** patterned line-based import, including CSV-like data; can accept direct text input instead of a file. Match sequentially to selection or by filename/full path. This is structured text import, not an audio codec converter. [Text import](https://docs.mp3tag.de/converters/import-tags-from-text-files/)
- **Filename → Filename:** capture up to nine parts, reorder/remove/transform them and create folders without using stored tags as the input source. [Rearrange](https://docs.mp3tag.de/converters/rearrange-filenames/)
- **Tag → Tag:** copy and compose fields with format functions; filename and directory special targets are available. [Copy/compose](https://docs.mp3tag.de/converters/copy-compose-tags/)
- **Auto-numbering:** start track/disc numbers, totals, zero padding, advance disc on directory/field change, reset tracks for a new disc, reset discs on field change. MP4 stores numeric track/disc values, so leading zero display/filename padding does not become stored integer formatting. [Numbering](https://docs.mp3tag.de/converters/auto-numbering/)

Mac documents Tag → Filename with absolute/relative folder creation, Filename → Tag including directory extraction and dummy placeholders, and line-based Text File → Tag. It uses `/` for folder structures, relative to each file's location. Tag → Tag and bulk track/disc numbering are also advertised. Windows Filename → Filename and its full counter-reset options are not established by these Mac sources. [Mac converters](https://community.mp3tag.de/t/rename-files-and-import-tags-from-files/51720), [Mac features](https://mp3tag.app/features/)

## 6. Format strings and function language

Windows distinguishes matching patterns (parse existing strings) from generating expressions (create field values, paths, output). Any supported field can be referenced. Optional bracketed parts and literal quoting are supported. Read-only technical placeholders cover codec, rate, bit depth, duration, art dimensions/types/count, present/displayed tag types, unknown ID3 frame IDs, chapter index/count, file paths, timestamps and file sizes. Export adds relative paths, checksums/audio hashes, totals and playlist placeholders. A technical placeholder is an observation, not an editable audio property. [Format reference](https://docs.mp3tag.de/format/)

Windows function families include string replacement, regex, case conversion, trimming/substrings, edit distance, deduplication/sorting, Unicode characters, number/date/duration formatting, path validation, boolean branching/comparison, integer arithmetic, random/UUID generation, environment variables and metadata enumeration/value selection. Functions nest. Important multivalue functions are `$meta(field)`, `$meta(field,index)` and `$meta_sep(field,separator)`; `$list` enumerates fields; column functions expose list data. Integer arithmetic is documented, not a general floating-point programming environment. [Windows functions](https://docs.mp3tag.de/scripting/)

Mac documents the same broad string/boolean/arithmetic/metadata families, including UUID, column values, multivalue access and date/duration functions. Its reference spells the joined metadata accessor `$metasep`, and comparison functions include `$gt`/`$lt`; Windows lists `$meta_sep` and `$grtr`/`$less`. Use the relevant platform reference instead of assuming expression portability. [Mac functions](https://community.mp3tag.de/t/scripting-functions/51689)

## 7. Filtering, finding and selection

Both platforms document plain-text filtering across tags/filenames; field-specific HAS/IS; AND/OR/NOT with grouping; integer GREATER/LESS/EQUAL; PRESENT/MISSING/ABSENT; and MATCHES regex. Format expressions can be evaluated as operands, with a speed cost; wildcard field selection is available for HAS/MATCHES. Operators are case-insensitive and keywords uppercase. Filtering may apply while typing or manually. [Windows filters](https://docs.mp3tag.de/filter/), [Mac filters](https://community.mp3tag.de/t/filtering-and-searching-files-and-tags/53271)

Windows also documents a separate Find dialog, all/invert/unselect operations and selecting similar/next-similar files by the active sort criterion. [Edit menu](https://docs.mp3tag.de/menus/edit/)

**Documentation conflict:** Mac's older Basics overview says complex expressions are planned. The dedicated Mac filtering reference documents them as supported; this catalogue follows that specific reference and excludes the obsolete future claim. [Overview](https://community.mp3tag.de/t/basics-and-overview/51957), [specific reference](https://community.mp3tag.de/t/filtering-and-searching-files-and-tags/53271)

## 8. Export and playlists

Windows text exports are configurable templates suitable for TXT/CSV/HTML and similar text outputs. They use placeholders and functions, sorted loop structures, variables, filename/encoding directives (ANSI/UTF-8/UTF-16), optional BOM, append mode and grouped multiple-output generation. Templates can be added, duplicated, edited and reused. This is report generation rather than transcoding. [Export](https://docs.mp3tag.de/export/)

Mac has File → Export text output, applying a format string once per selected file to produce one output line per file; filename configuration and formatted clipboard copy are documented. Windows's template loops and grouped multi-output machinery are not established by the Mac export article. [Mac export](https://community.mp3tag.de/t/export-tags-to-text-files/70192)

Windows loads and saves M3U/M3U8 playlists, preserves listed order but suppresses duplicate references to the same file in its File List. It saves all or selected files, supports rearrangement and generates multiple playlists by a partition expression, with a preview. Sorting is controlled by the main file list. [Playlists](https://docs.mp3tag.de/playlists/)

Mac preferences document a playlist filename expression and M3U8 UTF-8 output. The reviewed Mac documentation does not establish the full Windows playlist-management/generator feature set. [Mac preferences](https://community.mp3tag.de/t/preferences-and-configuration/56296)

## 9. Online databases and extensible Web Sources

Windows defaults are **Discogs and MusicBrainz**, plus cover-only sources. Search is album-based and user-guided: enter metadata, choose release, select fields/art, align tracks to files and confirm. Discogs Release ID can bypass result search. MusicBrainz can show a match score. Title/length sorting suggestions and saving images to disk are available. The import page says confirmation replaces the tag with visible result fields: do not assume safe merging from the provider name alone. [Import workflow](https://docs.mp3tag.de/tag-sources/import/)

Mac documents the same default providers, release-ID route, field/art selection and track remapping; explicitly distinguishes **Save** (replace) from **Merge** (combine, overwriting existing field contents). It supports custom sources copied into the Tag Sources folder and `#` submenu organization. Custom services are not automatically bundled or guaranteed operational. [Mac Tag Sources](https://community.mp3tag.de/t/import-tags-from-online-tag-sources/56329)

The shared Web Sources Framework uses source-description files for URLs, search parameters, encoding, result layout, parser scripts and includes. It can consume static HTML, XML or JSON APIs, not execute JavaScript/PWAs. Parser facilities include navigation/search, output buffers, replacements, conditions, bounded looping, JSON selection/iteration and debugging. Sources can specify platform minimum versions and schema-driven settings dialogs. Mac's official source article links this framework reference. [Developer reference](https://docs.mp3tag.de/tag-sources/development/)

The Windows marketing homepage still mentions freedb, while the dedicated import documentation identifies Discogs/MusicBrainz as defaults. Treat freedb as a marketing/reference mention, **not verified current bundled-service availability**. No provider requests or third-party script compatibility were tested. [Homepage](https://www.mp3tag.de/en/), [import documentation](https://docs.mp3tag.de/tag-sources/import/)

## 10. UI, library, tools and settings

Windows has configurable File List columns: separate display expression, editable field and sort expression, numeric alignment/sort behavior, visibility/order and saved configurations. [File List](https://docs.mp3tag.de/customization/file-list/)

The Tag Panel supports custom fields, text/check/multiline controls, defaults, custom value lists, sizing/order/visibility, saved configurations, directory navigation, headings, separators and read-only format-expression fields. [Tag Panel](https://docs.mp3tag.de/customization/tag-panel/)

Windows Library is an **optional metadata cache database**, refreshed when file modification timestamps change, with directory inclusion limits and orphan cleanup. Binary cover data can remain in the database to reduce memory use. This does not establish a streaming/media-server library. [Library](https://docs.mp3tag.de/customization/options/library/)

Windows external Tools launch configured applications with path placeholders, optionally per selected file; up to 99 tools, first ten keyboard shortcuts and a default double-click application. These integrate external programs; they do not turn those programs' analysis/playback/conversion into native Mp3tag capabilities. The CLI documents opening/adding paths, not headless tagging automation. [Tools](https://docs.mp3tag.de/customization/options/tools/), [CLI](https://docs.mp3tag.de/command-line/)

Windows supports Undo/Redo, action repeat, tag clipboard transfer, formatted text copy, file copy/move/rename, list removal and disk deletion. [Edit](https://docs.mp3tag.de/menus/edit/)

Settings families include general behavior, confirmations, files/directories, appearance/library, tag formats/mappings, sources, field values, panel, playlists, exports, tools, networking, language and updates. [Options index](https://docs.mp3tag.de/customization/options/)

Appearance supports light/dark/system modes, toolbar and interface scaling to 200%, icon choices and alternating/grid-line list styles. Tag settings control selection-change saves, Extended Tags save policy and whether external folder artwork may be displayed when embedded art is absent. Displayed folder artwork is not necessarily embedded artwork. [Appearance](https://docs.mp3tag.de/customization/options/appearance/), [Tags](https://docs.mp3tag.de/customization/options/tags/)

Mac uses a native Tag Panel/File List, Finder/Dock drag/drop and in-list editing; there is **no working directory** because files may come from multiple locations. Mac settings document custom field/value lists (including text import/export), timestamp preservation, invalid-NTFS-character replacement, custom panel/columns and export defaults. The reviewed official Mac pages do not establish Windows's optional Library cache or external Tools registry. [Mac basics](https://community.mp3tag.de/t/basics-and-overview/51957), [Mac preferences](https://community.mp3tag.de/t/preferences-and-configuration/56296)

Mac can import Windows `.mta` action groups, but the FAQ does not promise import of other Windows settings. Do not equate action-file import with full behavioral parity, especially regex/function differences. [Mac FAQ](https://community.mp3tag.de/t/frequently-asked-questions/51958)

## 11. Functional boundaries and audit caveats

- **Metadata presence is not computation:** BPM, ReplayGain-like metadata, fingerprints, IDs, lyrics, ratings and sort fields should be evaluated as field editing/import/preservation unless a separate official function documents generation. This inventory found no evidence establishing native BPM/key/loudness/fingerprint analysis; that is a research limit, not a universal negative claim. [Field reference](https://docs.mp3tag.de/mapping/)
- **Unknown data is not a feature editor:** an unknown-ID3-frame placeholder and unchanged-frame/atom reuse controls do not establish a UI for editing every unknown payload. [Placeholders](https://docs.mp3tag.de/format/), [preservation controls](https://docs.mp3tag.de/customization/options/tags/advanced/)
- **Media support is metadata support:** listed video containers, codec names and technical properties do not establish playback, re-encoding, waveform editing, silence detection or format conversion. The documented converters manipulate names, fields and text. [Converters](https://docs.mp3tag.de/converters/)
- **Sources versus service availability:** configured sources and official defaults establish integration mechanisms; working network service behavior, credentials, licensing and third-party script freshness remain untested.
- **Platform parity:** this catalogue explicitly distinguishes documented Mac behavior, Windows behavior and unresolved parity. Source lists are not a release-by-release changelog; no future plans are counted as delivered features.

## Verification record

Official pages linked above were fetched with web browsing and their returned content inspected; individual Windows action pages, tag settings and Mac documentation articles were checked where broad overview pages were insufficient. Specific discrepancies (Mac filtering overview and freedb marketing mention) are recorded rather than silently resolved by guesswork. No local Mp3tag installation, provider API, audio fixture or TagRex code was exercised. The deliverable was checked for required feature-family coverage and direct source links after writing.
