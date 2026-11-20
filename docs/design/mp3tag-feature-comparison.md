# Mp3tag and Tagnetic feature comparison

Tagnetic already provides the central batch tagging workflow. The largest capability gaps are cross-field automation, arbitrary-field expressions, complete multivalue operations, extensible imports and exports, and additional containers. We should adopt those capabilities through the existing Staged, Apply and Undo model. Adding them does not require replacing the workbench with Mp3tag's interface.

Research snapshot: **17 November 2026**. Local baseline: `main`, `23286e0e668a7b0555a37a809768e4109a6fe08d`, workspace version 1.4.9. “Tagnetic” below means this personal TagRex fork, not every upstream branch or an independently verified installed binary.

## Scope and evidence

This comparison covers the documented Windows feature families, all 18 documented Windows action types, supported container families, and material macOS differences. It is a product-level inventory, not a claim that every preference, function signature, keyboard shortcut or file variant has been tested. The [official-source catalogue](mp3tag-official-features.md) contains the detailed platform distinctions and links.

Local evidence combines the knowledge graph with current source reads. The graph generation was 13 November; coverage checks reported changed files and untracked query code, so current source takes precedence. Runtime behavior, provider availability and cross-format round trips were not exercised for this research. **Present** means a corresponding implementation exists; **Partial** means a narrower implementation or a separate command exists; **Gap** means no corresponding implementation was found in the inspected production paths. A follow-up README audit checked the newer ribbon controller and corrected the original Format Value and local artwork-import assessments. These are not runtime certifications.

Owned outputs: this comparison, the source catalogue and the accompanying HTML chart. Completion gate: sourced inventory, local evidence, prioritized adoption plan, verified document links and chart rendering. This is research only: no implementation, commit, PR, release or runtime change. Future shared interface features route to curated upstream `next`; independent shared backend changes route to upstream `main`; personal identity/release policy remains fork-only, following [WORKFLOW](../WORKFLOW.md).

## Feature inventory

| Capability | Tagnetic status | Current implementation and remaining work |
| --- | --- | --- |
| Batch editing and mixed selections | Present | Field editor, cell editing, staged bulk changes and differing-value handling. Preserve these entry points. |
| Arbitrary text fields | Partial | `TagField::Custom` and Add field exist. Addressing custom fields from the shared mask engine is explicitly unsupported. Format-specific mappings need separate coverage. |
| Field naming and mapping | Partial | Built-in mappings and preservation logic exist; no user-configurable cross-format mapping registry was found. Add alias resolution without changing unedited raw fields. |
| Multiple values | Partial | Explicit separator handling for Artist, Album Artist, Genre and Composer. The model uses strings, not a general list-valued field API. Repeated arbitrary fields and split/merge/dedup operations need deliberate support. |
| ID3 revisions and block inspection | Present | Read priority, v2.3/v2.4 choice, block removal/conversion and encoding repair exist. Conversion is different from preservation and can lose unmapped data. |
| Full tag read/write/remove policies | Partial | No equivalent of Mp3tag's complete independent format-policy settings was established. Audit exact ID3v1/APE/RIFF behavior rather than counting library capability as UI support. |
| Embedded artwork | Present | Multiple covers, types/descriptions in DTOs, embed/remove/export, external cover discovery and staged set operations exist. |
| Advanced artwork processing | Partial | Size/quality policy and tag-pattern local imports exist. Batch crop/format/property recipes, full picture-type fidelity, ordering and parameterized exports need further work or UI verification. |
| Tag to filename and directory trees | Present | Shared masks, folder organization, collision previews, portable names and sidecar handling. Current destination is bounded by the opened root. |
| Filename to tags | Present | Shared parser, folder extraction and skip placeholders. Custom target fields remain a gap. |
| Text to tags | Partial | UTF-8/UTF-16 files, pasted tracklists and positional mask import exist. Path/filename keyed matching is missing in `preview_tags_from_text`, which zips files with parsed lines. Simple delimiter patterns are possible; this is not a complete quoted-CSV mapper. |
| Filename to filename | Partial | Name-scoped transforms can replace/case-convert existing names. No dedicated capture-and-rearrange converter equivalent was found. |
| Tag to tag / Format Value | Partial | The ribbon Format values command fills common destination fields from patterns. Arbitrary target fields, recipe integration and composition over staged inputs remain incomplete. |
| Numbering wizard | Partial | Start, total, fixed disc, restart by group and preservation of vinyl positions exist. Automatic disc advancement/reset on directory or field changes and complete disc-total rules need extending. |
| Reusable action groups | Partial | Named ordered chains, enabled rules, per-rule scope and sample preview exist. Six transform kinds are narrower than Mp3tag's action catalogue. |
| Expression language | Partial | Nested string, boolean and decimal arithmetic functions exist. Metadata enumeration/indexing, regex expression functions, variables, date/path/Unicode helpers and compatible signatures are incomplete. |
| Search and structured filtering | Present | Chips, nested all/any conditions, numeric ranges, missing/present, exclusions, custom computed fields and saved queries exist. |
| Mp3tag query syntax and expression filters | Partial | Current UI deliberately uses literal search and structured rules. Backend legacy regex support is not proof of an exposed MATCHES/expression filter. Add an advanced mode only if useful; preserve current chips. |
| Configurable columns | Partial | Column layout and mask-defined values exist. Independently configurable display/edit/sort expressions and interchangeable layouts need a dedicated parity check. |
| Fully configurable tag panel | Partial | Custom fields and progressive grouping exist. Custom checkbox/multiline/value-list controls, default values, headings and saved panel layouts were not established as a user configuration system. |
| Text/CSV/HTML/XML exports | Partial | Built-in exporters and one-line mask reports exist. No general template loops, variables, encodings/BOM, append mode or arbitrary grouped output equivalent was found. |
| Playlists | Partial | M3U output and per-folder/per-album batches exist; grouped names accept M3U/M3U8 suffixes. Playlist loading and arbitrary expression-based partitioning were not found. |
| Online release import | Present | Discogs, MusicBrainz, Beatport, Bandcamp and Soundeo provider modules, field/art selection and matching exist. Working service access was not tested. |
| User-installable web sources | Gap | Compiled provider crates provide a seam, but users cannot currently install an Mp3tag-style source description/parser through an established framework. |
| Clipboard editing | Partial | Spreadsheet TSV copy/paste/fill exist. Whole-tag clipboard transfer, field-value-preserving repeated fields and formatted clipboard templates need separate implementation or verification. |
| External tools registry | Gap | No configurable external command templates, per-file invocation and tool shortcuts equivalent was found. OS path handover already exists and should be retained. |
| Persistent library acceleration | Partial | Read caches and immutable query indexes exist. They do not establish Mp3tag's optional persistent metadata/cover database with include/exclude and cleanup controls. Measure the benefit first. |
| Timestamp preservation | Gap | No user-facing preserve-file-modification-time policy was found in inspected settings/write paths. Implement carefully because query and external-change detection use file stamps. |
| Undo and redo | Present | Journaled batches and session redo APIs exist. Preserve tag/artwork/rename composition and journal recovery. Do not assume identical persistence semantics between products. |
| OS integration and configuration portability | Partial | Launch arguments, Open With/Dock handover, multiple dropped files/folders, themes and remappable shortcuts exist. Windows shell integration and import of Mp3tag settings/actions need separate work. |
| Existing chapter metadata editing | Gap | No chapter-entry editing workflow equivalent was found. Treat this separately from CUE export and arbitrary text fields. |

Mp3tag references for the rows above: [Extended Tags](https://docs.mp3tag.de/extended-tags/), [mappings](https://docs.mp3tag.de/mapping/), [tag options](https://docs.mp3tag.de/customization/options/tags/), [cover management](https://docs.mp3tag.de/manage-covers/), [converters](https://docs.mp3tag.de/converters/), [numbering](https://docs.mp3tag.de/converters/auto-numbering/), [functions](https://docs.mp3tag.de/scripting/), [filter](https://docs.mp3tag.de/filter/), [columns](https://docs.mp3tag.de/customization/file-list/), [tag panel](https://docs.mp3tag.de/customization/tag-panel/), [export](https://docs.mp3tag.de/export/), [playlists](https://docs.mp3tag.de/playlists/), [web-source development](https://docs.mp3tag.de/tag-sources/development/), [tools](https://docs.mp3tag.de/customization/options/tools/), [library cache](https://docs.mp3tag.de/customization/options/library/), [advanced tags and chapters](https://docs.mp3tag.de/customization/options/tags/advanced/). Detailed citations and Mac timestamp/configuration behavior are in the source catalogue.

[Open the action coverage chart](mp3tag-comparison-chart.html).

## All 18 Windows action types

Status here compares the operation with Tagnetic's reusable action system. A standalone command earns Partial when it cannot be composed as that action. It does not mean we lack the standalone feature.

| Mp3tag action | Status | Tagnetic equivalent or missing capability |
| --- | --- | --- |
| Adjust Cover | Partial | Size/quality controls exist; full image-processing recipe is incomplete. |
| Case Conversion | Present | Four case modes, per-field/name scopes and saved chains; directory/field-name targets remain narrower. |
| Convert Codepage | Partial | Legacy codepage setting and staged repair exist outside transform chains. |
| Export | Partial | Standalone exporters exist; export is not a recipe step. |
| Export Cover to File | Partial | Standalone extraction exists; parameterized recipe export is missing. |
| Format Value | Partial | Ribbon assignment from patterns exists for common fields; arbitrary targets and saved recipe integration are missing. |
| Generate Playlists | Partial | Standalone folder/album playlist batches exist; arbitrary partition recipes are missing. |
| Guess Values | Partial | Filename/text extraction exists; general source expression to target-field pattern is missing. |
| Import Cover from File | Partial | Ribbon imports support a tag-based filename or first image in a folder; saved recipe integration is missing. |
| Import Text File | Gap | No per-file text-content-to-field recipe, for example external lyrics. |
| Merge Duplicate Fields | Gap | No explicit list-to-single-value recipe. |
| Remove Duplicate Fields | Gap | No explicit repeated-value deduplication recipe. |
| Remove Fields | Partial | Field clearing/tag clearing exist; selective deletion is not a transform rule. |
| Remove Fields Except | Gap | No keep-only allowlist recipe. |
| Replace | Present | Literal, whole-word and case controls exist. |
| Replace with Regular Expression | Present | Regex replacement exists; engine semantics differ. |
| Set Cover Properties | Partial | Model carries kind/description; batch property recipe is missing. |
| Split Field by Separator | Gap | No explicit general field-to-multiple-values recipe. |

That is **3 corresponding transform operations, 10 partial equivalents and 5 gaps** across these 18 action categories. These counts are categorical inventory counts, not a feature-completeness percentage. “Present” does not mean identical suboptions. [Official action index](https://docs.mp3tag.de/actions/) and [individual action details](mp3tag-official-features.md#4-actions-and-reusable-automation).

## Format coverage

The scanner currently accepts 19 extensions across 12 modeled container/codec families. An extension match alone does not prove read/write compatibility, playback or lossless metadata preservation. [Scanner](../../crates/core/src/scanner.rs), [AudioFormat and TagEngine](../../crates/core/src/model.rs).

| Mp3tag family | Tagnetic assessment | Adoption decision |
| --- | --- | --- |
| MP3, AAC | Accepted and modeled | Verify each supported tag-block combination rather than add another format badge. |
| FLAC | Accepted and modeled | Native FLAC supported; Ogg FLAC is explicitly refused. Do not label all `.oga` files supported. |
| MP4/M4A/M4B, ALAC container | Accepted and modeled | Validate relevant audio variants independently. `.m4v` is not in the scanner allowlist. |
| AIF/AIFF/AIFC, WAV | Accepted and modeled | Audit RIFF INFO and ID3 coexistence/write policy. |
| Ogg Vorbis, Opus, Speex | Accepted and modeled | Codec-specific fixture tests required. |
| Monkey's Audio, Musepack, WavPack | Accepted and modeled | Preserve existing support and verify block/picture combinations. |
| DSF | Gap | Add a backend with checked ID3 preservation before advertising it. |
| WMA/ASF | Gap | Separate format/backend work; Windows Mp3tag documents it. |
| TAK, TTA, OptimFROG OFR/OFS | Gap | Investigate backend support and sample demand; do not route through an assumed generic reader. |
| Matroska MKA/MKV, WebM | Gap | Includes tags, chapters and attachments; substantially more than adding extensions. |

The authoritative comparison baseline is [Mp3tag's read/write/remove matrix](https://docs.mp3tag.de/file-formats/). Do not add DFF as a gap attributed to Mp3tag: DSF is the documented DSD entry. Do not claim standalone ALAC is missing just because there is no `.alac` extension. macOS's advertised list differs from Windows; see the [Mac feature list](https://mp3tag.app/features/) and source catalogue.

## Preserve and build on our current workflow

Keep the table, inspector, Staged review, field locks, explicit Apply, persistent Undo history, saved searches and library/session state. Keep the integrated player/waveform, BPM detection, local duplicate comparison, vinyl-position handling and sidecar-aware moves. These are implemented product capabilities worth retaining; this audit does not claim Mp3tag has no equivalent to every one of them.

The documented Mp3tag action workflow saves changes as actions run. Our equivalents should produce a plan first. Add quick access to a single operation and named recipes using the same machinery; avoid a second immediate-write route. [Mp3tag action behavior](https://docs.mp3tag.de/actions/), [our change-plan model](../guide/change-plans.md).

Every recipe needs a defined input state: begin from the current staged values, then evaluate steps in order against the result of preceding steps. Show the cumulative diff, affected-file count, skips and conflicts. Do not let a later operation silently read older on-disk tags while a prior step changed those tags in the recipe. Preview generation should remain deterministic and cancellable.

Audio/tag/artwork/path writes use the existing plan executor and journal. Exports can keep their current separate output workflow because they do not edit audio. For a recipe combining edits and exports, define whether it exports planned or saved values and when publication happens; a failed export must not imply successful tagging or promise journal rollback of an unrelated output. External programs need an explicit separate launch boundary because their writes are outside our Undo transaction.

Do not expand every field and option onto the main screen. Use existing inspectors, expression builders and preset editors, with optional advanced controls. Adding Mp3tag syntax compatibility should be an import/translation layer with diagnostics, not a change to the meaning of existing masks.

## Adoption order

| Phase | Deliverable | Completion criteria |
| --- | --- | --- |
| 1 — General fields and expressions | Custom-field references; explicit list values; metadata accessors; cross-field assignment; selective delete/keep-only; split/merge/dedup | One canonical resolver used by masks, actions and imports. Preserve existing masks and punctuation. Preview shows additions/removals and ordered values. Disposable MP3/FLAC/MP4 fixtures independently verify Apply and Undo. |
| 2 — Reusable recipes | Add Format Value, Guess Values, field removal, multivalue operations, per-file text import and artwork operations to current preset model | Existing six transform kinds migrate without behavior changes. Disabled/reordered steps compose correctly over staged input. Locks, cancellation, collision handling and one coherent Undo batch are verified. |
| 3 — Import and export interoperability | Filename/path keyed text mapping; quoted CSV/TSV mapping; filename capture/rearrangement; richer numbering; playlist input and custom grouping; report templates and encodings | Reject ambiguous matches. Preview unpaired rows and protected fields. Preserve playlist order. Output collisions and encoding choices are explicit. Keep current exporters and tracklist import. |
| 4 — Artwork and source extensibility | Batch artwork property/crop/format/path recipes; configurable field mappings; user-installable provider definitions | Preserve non-target pictures and raw types/descriptions. Bound image/parser work. Installed sources use the existing provider/import plan gate and existing credentials/network policies. |
| 5 — Container coverage and chapters | DSF and requested missing containers, then chapter/tag/attachment editing where supported | A published read/write/remove matrix and independent preservation fixtures for every advertised combination. Playback support is reported separately. No silent tag loss on unsupported payloads. |
| 6 — Integration and migration | External tools, timestamp policy, richer layouts/value lists, optional persistent cache, selected Mp3tag configuration import | Cross-platform launch behavior and external writes are clear. Timestamp policy cannot hide changes from caches or Undo conflict checks. Cache only after measured need. Imported settings report unsupported semantics. |

These phases are proposed dependencies, not delivery dates. Phase 1 provides the foundation for most high-value gaps; container work can proceed independently once a backend choice and fixtures are available. Full `.mta`, `.mte` or `.src` compatibility is a separate scoped project. Translate only supported semantics and give a precise rejection list for the rest.

Useful first recipe: **fill missing Album Artist from Artist → clean spacing → split/deduplicate Genre → derive destination from corrected tags → review one plan → Apply → Undo the whole batch**. Exporting a playlist can be offered afterward using the same chosen grouping. This exercises the foundation without changing the user's normal workflow.

## Compatibility decisions and verification backlog

- Expression compatibility requires a function-by-function table, not just matching names. Tagnetic currently uses nonempty-string truth and decimal arithmetic; Mp3tag documents integer arithmetic. `$greater` versus `$grtr`, substring naming, optional arguments and missing-field behavior differ. Windows and Mac Mp3tag also differ. Keep current semantics stable and translate explicitly.
- Regex replacement syntax and capabilities differ across Rust regex, Windows Mp3tag and Mac ICU. Reject or explain unsupported patterns instead of silently changing them.
- Custom-field mapping must distinguish standard fields, free-form names, language-qualified comments/lyrics, rating owner/scale, sorting fields and raw payload preservation. Preserving an unknown frame is not the same as exposing an editor for it.
- Multivalues need a lossless representation before generalized list operations. The current separator-based string map cannot express every original repeated-field distinction reliably.
- Artwork's simplified `CoverKind` enum groups several raw picture types together. Audit exact round trips before claiming a full type editor or using set-all operations.
- Mp3tag supports editing existing chapter metadata in relevant containers. That does not mean either app is an audio splitter, full chapter timeline editor, encoder or ReplayGain analyzer.
- No live service tests or installed Mp3tag checks were made. Retired-provider marketing mentions are not new requirements; the specific source docs take precedence. Before shipping parity claims, run representative Windows/Mac Mp3tag and Tagnetic fixtures side by side.

## Local evidence map

| Evidence | Relevant implementation |
| --- | --- |
| Supported extensions and format enum | [scanner.rs](../../crates/core/src/scanner.rs), [model.rs](../../crates/core/src/model.rs) |
| Custom fields, multivalue limitation, cover kinds, tag blocks | `TagField`, `is_multi_value`, `TagMap`, `CoverKind`, `TagBlockKind` in [model.rs](../../crates/core/src/model.rs) |
| Mask field allowlist and file properties | `field_from_name`, `file_value_from_name`, `tag_placeholder_doc` in [mask.rs](../../crates/core/src/mask.rs) |
| Expression capabilities and arity | `Function`, `arity`, `tolerates_missing_tags` in [functions.rs](../../crates/core/src/mask/functions.rs) |
| Six transformation kinds and group serialization | `TransformRuleDto`, `ActionGroupDto`, `build_chain` in [commands lib.rs](../../crates/commands/src/lib.rs) |
| Positional text pairing and tag/artwork/block preview commands | `preview_tags_from_text`, `preview_tag_edits`, `preview_cover_set`, `preview_fix_encoding`, `preview_convert_tag_block` in [commands lib.rs](../../crates/commands/src/lib.rs) |
| Playlist batches and confined output publishing | `export_playlist`, `export_playlists`, `export_report` in [commands lib.rs](../../crates/commands/src/lib.rs); [exporters.js](../../app/ui/js/exporters.js) |
| Ribbon field formatting and artwork imports | `connectRibbonActions` in [app.js](../../app/ui/app.js), [RibbonTools.svelte](../../app/ui/components/RibbonTools.svelte) |
| Numbering and spreadsheet clipboard | `numberingEdits` in [generator.js](../../app/ui/js/generator.js), [spreadsheet.js](../../app/ui/js/spreadsheet.js) |
| Tag format and separator controls | [editor.js](../../app/ui/js/editor.js), [settings.js](../../app/ui/js/settings.js) |
| Query, duplicates and cache boundaries | [query.rs](../../crates/commands/src/query.rs), [duplicates.rs](../../crates/commands/src/duplicates.rs), [read_cache.rs](../../crates/commands/src/read_cache.rs), [query behavior](../filter-workbench.md) |
| OS file handover and command exposure | [dragdrop.js](../../app/ui/js/dragdrop.js), [main.rs](../../app/src/main.rs) |

Negative findings are bounded by these production modules, command registration, settings, provider crates and targeted source searches. Inherited documentation is supporting context only; the follow-up read included RibbonTools.svelte, but a full native UI audit was not performed. “Needs verification” items must not be promoted to complete parity based solely on this document.
