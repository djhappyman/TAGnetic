# Provider-aware search chips

Status: research and implementation plan, 2026-11-20. No application changes.

## Scope and completion

Provide field/value chips in Online search only when the selected provider's
implemented adapter applies that constraint. Preserve ordinary text search,
direct release links, selection, preview, staging, Apply and persistent Undo.
This plan owns this file and the companion capability chart; the research owns
[provider-search-capabilities-research.md](provider-search-capabilities-research.md).
Existing unrelated comparison notes and runtime state are outside scope.

Planning completion: review cited external capabilities, inspect current query
adapters, specify interactions, delivery phases, verification and open questions.
Implementation completion is a separate gate below. Shared interface behavior
routes to a curated `tagrex/tagrex:next` contribution; no upstream submission is
part of this research task. Follow WORKFLOW.md at implementation time, including
current fork main/base and branch limits, and current upstream dependency rules.

## Findings and recommended rollout

The research distinguishes documented capabilities, observed requests and
unverified leads. Unknown means unverified, not necessarily unsupported.

| Source | Current adapter actually sends | Native expansion evidenced | Proposed chip availability |
| --- | --- | --- | --- |
| Discogs | Artist, release title, catalogue number, format, general text; release results | Label and Album/Sampler format descriptors positively observed; additional field probes are candidates requiring positive validation | First implement Artist, Release title, Label, Catalogue number and Medium. Add Classification with tested provider values. Then validate Year, Country, Barcode, Genre, Style, Track and credits before enabling each. |
| MusicBrainz | Artist, release title, catalogue number, mapped medium, general text | Documented release fields include Label, Barcode, Country, Date, primary/secondary release-group type, Packaging, Language, Track count, identifiers and more | First implement Artist, Release title, Label, Catalogue number, Medium; then Barcode, Country, Date, Primary type, Secondary type. Remaining documented fields can be added with adapter tests. |
| Beatport | All supplied text fields joined into `q`, with `type=releases`; no independent field constraints | First-party search docs list release `artist_name`, `label_name`, dates, UPC and `release_type_id`; track search has a different allowlist | General search initially. Enable Artist, Label, UPC/date chips only after authorised adapter validation. Do not offer a title-only chip for release search: `q` searches broadly. Do not offer BPM/Key merely because track search supports them. |
| Bandcamp | General text through an internal autocomplete endpoint with album result category; direct release URL | No independently verified catalogue field-filter contract | General search and existing direct URL handling. No Label/Artist/title-only chips yet. Album result category does not mean musical Album classification. |
| Soundeo | Website `search?q=...`; direct release URL; local slicing of one result set | Website supports artist/label/style facets, but reusable search endpoint grammar is unverified | General search and direct URL handling. No field chips until those constraints work through our adapter. |
| GnuDB, future | No adapter in the inspected provider hub | Official protocol now documents Artist/Album text-search extension and `tracks`, plus CD TOC lookup. No label-filter evidence | Separate future integration. Validate text-query combinations, retrieval flow, access identification and track-count semantics. Offer only validated Artist/Album/Track count constraints; TOC lookup is a separate operation. |

Exact external field lists, primary links, access requirements and caveats live
in the [research note](provider-search-capabilities-research.md). This table is a
rollout recommendation, not a claim that all these filters already work here.

## React / Bonkers example and naming correction

`Label: React` + `Release title: Bonkers` must preserve the two constraints and
return provider-matching releases without an implicit Album restriction.
Discogs probes returned 34 matches, but adding `format=Album` returned zero;
counts are time-sensitive. A sampler named Bonkers on another label is correctly
excluded by Label: React. Search matches are provider-defined, not guaranteed
exact equality or every edition of the series.

Keep three concepts separate:

- Result entity: release versus artist/label/master. Keep the existing release
  workflow; do not introduce artist/label results that cannot populate it.
- Release title: name search. Suggest it for the alias `album`, but display
  **Release title** on providers using release-title semantics.
- Classification versus Medium: Discogs Album/Sampler descriptors are `format`
  values; MusicBrainz Album is a primary release-group type, Compilation and
  DJ-mix are secondary types, and CD is a medium. These are not interchangeable.

Do not map Discogs Sampler to MusicBrainz Compilation or Bandcamp album category.
Do not expose multiple Discogs format constraints until their combination
semantics are verified. Begin with at most one descriptor across its medium and
classification controls, or validate multi-value encoding before allowing both.

## Interaction contract

1. Typing at a valid field position offers supported field suggestions. Accept
   with Enter or pointer; ordinary words remain text until explicitly accepted.
   Reuse the current chip keyboard/IME/clipboard behaviour rather than interpreting
   every occurrence of `label` as syntax.
2. A search chip stores a field and an entered value. It is a query constraint,
   not a formatting token that substitutes the selected file's tag. Values can
   contain spaces and punctuation. No automatic selected-file substitution.
3. Different populated fields combine with AND only where the adapter has that
   established meaning. Start with one value per field; defer arbitrary OR,
   duplicate fields, negation and ranges until provider-specific tests exist.
4. Keep free text supported. When mixed with chips, define and test the provider's
   general-text scope explicitly. MusicBrainz currently uses dismax for text-only
   searches but unscoped release-title terms in a fielded query: do not claim that
   adding a chip preserves all-field dismax semantics. Chips must not broaden the
   query or fall back to general text after failure.
5. Keep a separate draft per provider, including values, chips and caret state.
   Switching restores that provider's draft; the first visit starts empty.
   Do not automatically transfer chips. This preserves the old source's work
   without silently dropping constraints or translating them into broad words.
   Cross-provider copying can be a later explicit operation for compatible fields.
6. The active value's field hue colours the single compound search input focus
   outline. Completed chips retain their stable field hues. With no active chip,
   use the normal theme focus accent. Error styling has priority; colour never replaces
   visible field/value labels, keyboard focus or accessible announcements.
   Apply this to the search editor, not every result/file selection border.
7. Clearing/removing constraints broadens only the current draft. Empty/incomplete
   field values prevent submission and are announced accessibly. Removing an
   incomplete chip restores ordinary typing. URL lookup remains available as its
   existing direct route, not a field filter combined with unrelated constraints.
8. On provider/query changes reset pagination and discard older results and
   thumbnails. Load more reuses the exact normalised query. Retry preserves the
   filters; auth/rate/network failures do not silently rerun an unfiltered search.

## Technical plan and ownership

Phase 1 — native capability contract and Discogs/MusicBrainz queries:

- Own `crates/core/src/provider.rs`, `crates/commands/src/lib.rs` and the two
  provider adapters. Define capability metadata as the single authority shared
  with the UI: field ID, label/aliases, value kind, allowed values, result context,
  supported combinations and enabled implementation status.
- Use explicit `text`, `release_title` and `label` query concepts. The legacy
  `album` property currently carries general text, while `title` carries release
  title. Preserve old caller compatibility through an explicit conversion; never
  reinterpret stored `album` text as a title restriction without migration.
- Keep Catalogue number and Barcode distinct. Existing catalogue input maps to
  Discogs `catno`; its code comment mentioning barcode is not evidence of barcode
  search support. Map MusicBrainz Barcode to `barcode` when implemented.
- Backend validation rejects unsupported constraints, values and combinations
  before requests. Advertise only fields actually implemented and verified.
  Externally documented but unavailable fields remain disabled internally and
  absent from suggestions. Capability invalidation handles stale saved drafts.
- Generate Discogs parameters separately; generate escaped MusicBrainz terms
  with explicit conjunction. Keep typed values out of query syntax injection.

Phase 2 — search editor integration:

- Locate the current Online source/search controller through the graph at
  implementation time. Reuse chip presentation and field hues, but keep the
  search value model separate from Format fields pattern serialisation.
- Own the search controller, focused search editor component and capability DTO
  boundary. Read the Svelte and TagRex UI skills before inspecting/editing those
  components. This audit did not inspect Svelte source and does not prescribe
  unverified component names.
- Implement the interaction contract above; keep Format fields behaviour intact.
  No installed release replacement, library writes or runtime restart for planning.

Phase 3 — expand providers independently:

- Beatport: verify documented release filter encoding and conjunction using the
  authorised API route; test release-type values before naming Album/EP/etc.
- Bandcamp/Soundeo: verify the actual integrated endpoint supports native field
  constraints before enabling them. Website facets alone are insufficient.
- Discogs/MusicBrainz: incrementally enable the researched advanced fields with
  positive/negative and combination tests; do not expose a huge metadata catalogue
  simply because fields exist in returned releases.
- GnuDB: separate adapter/access/protocol task. Historical freedb shut down; name
  the live provider GnuDB. Resolve text-result retrieval requirements before
  claiming a complete search-and-fetch workflow.

## Verification and acceptance gate

Implementation checks must establish:

- Per-provider suggestions exactly match backend-enabled capabilities; direct API
  requests with unsupported constraints fail explicitly.
- Label + release title are encoded as independent conjunctions, including the
  React/Bonkers example, no implicit Album. Distinguish no matches from errors.
- Text-only and legacy saved queries keep their intended semantics. Barcode is
  never silently sent as catalogue number. Quotes, spaces, Unicode and punctuation
  round-trip without changing query operators.
- Per-source draft restoration, duplicate/incomplete-field handling, rapid source
  changes, stale replies, retries, pagination and thumbnail races behave correctly.
- Chip acceptance/deletion/caret, copy/paste, local undo and IME work in WebKit and
  Chromium. Check both themes, narrow windows, labels and focus/error precedence.
- Existing release fetch, track matching, preview/stage/Apply and Undo remain
  intact. Any media write verification uses disposable fixtures and the safe-audio
  skill; source-search testing itself is read-only.

Run focused hosted native/query and frontend contract checks first, then the
applicable full Linux/Windows/macOS gate at the exact candidate. Respect the
existing hosted-only workload restrictions; this planning task runs no build,
formatter, benchmark or application test. Add bounded read-only provider probes
where access permits; unstable live counts do not become fixed regression data.

## Local audit evidence and remaining limits

Graph coverage checked for all seven backend evidence files; the graph generation
was 2026-11-13 and six files reported changed metadata. Current search methods,
MusicBrainz query builder, DTO/conversion and hub routing were therefore read
directly. Core query definition matched graph metadata and was also read.

- `crates/core/src/provider.rs`: `SearchQuery`.
- `crates/commands/src/lib.rs`: `SearchQueryDto`, `to_search_query`,
  `ProviderHub::provider_search` (five provider routes).
- `crates/providers-discogs/src/lib.rs`: `DiscogsProvider::search`.
- `crates/providers-musicbrainz/src/lib.rs`: `search`, `build_query`, `mb_format`.
- `crates/providers-beatport/src/lib.rs`: `search` joins fields into `q`.
- `crates/providers-bandcamp/src/lib.rs`: `search`, internal album autocomplete.
- `crates/providers-soundeo/src/lib.rs`: `search`, website text search.

This is a bounded backend/search audit, not an exhaustive application inventory.
External evidence limits and outstanding access questions remain explicit in the
research note. No implementation, commit, PR, build or installed-app change is
claimed by this plan.
