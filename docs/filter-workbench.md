# Library query command bar

This replaces the earlier Add filter workbench on `feat/library-query-command-bar`.
Use the existing interface tokens; visual refinement is a separate iteration.

## Behaviour

The search field contains removable, editable chips and ordinary search text.
Focusing an empty input shows nothing. Typing waits 160 ms before offering
bounded suggestions from the opened library, ranked by relevance and frequency;
matching ignores letter case. Searches are literal; regex and case toggles have
been removed, including their effects when recalling old presets or sessions.
Suggestions preview coloured chips with highlighted
matching text. Suggestion field/operator labels use strong normal text; only values have
muted, field-specific colour surfaces. Each field appears once, for example
`Artist contains: Hixxy`, counting every matching credit variant. Exact genre,
year and format values remain direct category chips. Choosing
`Genre: Hardcore` commits a filter and leaves the input ready for refinement.
Committed filters are single compact value pills with the coloured remove action
inside the same muted surface. Full field/operator names stay in accessible
edit/remove labels and the hover description; exclusions and complex rules retain
their meaning in the compact label.
Text after the chips searches within their results. Ordinary words remain text
until a suggestion is chosen. Existing field-prefixed text remains supported.

Separate chips match all by default. Adding another value to an existing
positive field chip makes alternatives inside that chip: Hardcore **or**
Jungle. The chip editor supports exclusions, missing/present, numeric comparisons
and inclusive ranges. **Add alternative** creates a match-any group; groups can
contain match-all or match-any subgroups. Limits are 128 nodes and eight levels.
The editor uses explicit rules. A small local phrase recognizer proposes chips
for common expressions such as `hardcore from 1996 missing artwork`,
`artist Hixxy or Sharkey`, and `bpm above 170`. Unknown words remain ordinary
search text. Proposals never apply implicitly. This combines the readable grouped
rules of [Notion](https://www.notion.com/help/views-filters-and-sorts) with the
recognition-and-confirmation interaction of
[Todoist Quick Add](https://www.todoist.com/help/todoist/features/schedule-a-date-and-time-for-your-todoist-tasks-q7VobO),
without an AI service or an attempt to interpret arbitrary natural language.
Quoted field values stay literal, so `title "Live or Die"` and
`title "Missing Artwork"` do not become Boolean or missing-tag instructions.

Artist and album-artist suggestions default to **contains**, covering all credit
spellings containing the typed text. This includes partial names such as Hixxy2;
the visible operator makes that broad scope explicit. The editor also offers
**includes a value** for individual artist membership: `Artist: Hixxy`
matches Hixxy alone, `Hixxy & Sharkey`, `Hixxy / Dougal`, and `Hixxy, Seduction`,
while excluding `Hixxy2`. Strong list separators and feat./ft./featuring/vs.
credits also work. Raw credits remain untouched. Unspaced `AC/DC` and mixed
punctuation names such as `Earth, Wind & Fire` stay intact; unspaced ampersands
split only when both individual artists are independently established in the
library. Ambiguous names can always use **is exactly** or **contains** instead.
Genre membership splits explicit semicolon/newline lists and preserves names
such as `Drum & Bass`. Values are deduplicated per file before counting facets.

Suggestions respect other conditions. A positive equality/member/contains condition on the
suggested field is omitted only at the top-level match-all boundary, allowing a
second genre to be offered without incorrectly rewriting nested logic. Member
conditions use the same facet rule. Counts reflect the emitted condition.

Arrow keys navigate suggestions; Enter chooses an explicitly highlighted option.
Enter with no highlighted option keeps the text as a search. Alt+Enter opens
the condition/group editor directly. Escape closes the popup without
clearing the query. Backspace in an empty input focuses the last chip rather
than deleting it. Counts settle politely, and selected files hidden by a query
are reported. Routine query work stays silent; errors, empty results and hidden
selection retain feedback. Narrow windows wrap chips inside the compound field. The bar stays
open permanently; obsolete disclosure state is ignored when restoring sessions.

## Modes and presets

The Duplicates toggle switches the field's scope without another mode dropdown.
Changing modes dismisses the previous text and chips. Entering Duplicates reveals
the inspector, including from the query toggle on compact or closed layouts.
The matching method stays above a centered ready message with **Scan files**
directly beneath it. Method explanations and review guidance live behind the
heading's information button. Scan/Cancel and progress stay in that sidebar.
The ordinary file table stays visible until an explicit scan starts, then clears
for progressively verified groups. Scans use all opened files, independently of
the current selection or result filter.
The duplicate bar searches only verified scan groups. Suggestions stay empty
until a verified group exists and never come from unrelated library entries.
Groups appear progressively under the saved track title (filename fallback),
with copy rows and their existing tag columns inside. Filtering a member keeps
matching companions available for comparison. New publications retain mounted
copy rows, selection, focused cells and the logical scroll anchor.
Selecting a result row or its group heading selects every copy in that group.
Enter/Space does the same; the caret and header Left/Right retain disclosure.
The sidebar compares shared saved tags, differing field names and each copy's
path, format, duration and artwork status. It derives this detail from loaded
TrackDtos without rereading files. Copy details mount at most 20 per page.
Selected groups incorporate newly verified members. Restart/cancel, method,
library or mode changes, and filtering the selected group away dismiss the
detail. Hidden selected paths and staged edits remain separate and preserved.
Switching back to Search cancels active or starting
scans and clears scan feedback. Scanning and filtering never remove files.

The table publishes `tagrex:duplicate-selection` with `{group: null}` or a group
containing `id`, `title`, `reason`, `tracks`, `sharedTags` (a string record) and
`differingTags` (field keys). `tagrex:request-duplicate-selection` hydrates new
sidebar subscribers. Only nonempty values equal across every saved copy appear
as shared tags; missing and empty values compare equally. The spreadsheet yields
duplicate Enter/Space activation to the table's group-selection owner.

Saved presets capture the query tree, remaining text, mode, duplicate kind and
existing sort/group preferences. They store no file selection or pending edits.
The v2 preset store migrates valid legacy searches without overwriting the
original stores. Once written, v2 is authoritative so deleted presets stay
deleted. Session view state stores its library root separately from `queryRoot`.

## Native boundary

Rust retains an immutable, versioned library index built from the initial scan's
DTOs. Warm matching does not rescan the filesystem or reread tags. Fields and
case-folded text are indexed outside Svelte; custom mask definitions derive a
cached index over saved values using the existing mask engine.

The frontend sends a small v2 query. Native query and suggestion jobs run outside
the serialized mutation queue. They have independent arrival generations;
frontend refresh resetting a request counter cannot make future queries stale.
Matching returns compact indices. A path manifest is sent only when its snapshot
changes, so frontend sorting/reordering cannot change an index's meaning.

All matching uses **saved metadata**. Pending edits, transformed previews and
selection remain separate. Staged rows retain their established review visibility.
Apply is blocked while query scope is pending or invalid, and in duplicate mode.
Malformed queries retain the last complete result set and
explain the error; they never silently broaden the operation scope.

Field prefixes and the three localized artwork-status labels remain supported.
Old pattern syntax is now ordinary literal search text. Ordinary unknown field-like text keeps its literal search meaning;
structured unsupported fields reject rather than matching everything.

Refresh, Apply, Undo, rename and library replacement invalidate affected snapshots.
Tag-only Apply refreshes dirty row paths and retains unrelated saved rows; a query
can perform that refresh itself before frontend readback. Topology changes and
Undo reconstruct the snapshot.
Cancelled or superseded work cannot publish a newer view. Duplicate scans use
filesystem stamps and bounded analysis caches, report progress/skips/errors and
observe snapshot invalidation as well as explicit cancellation.
The first verified duplicate group publishes immediately; later updates batch
at 300 ms and flush on completion. Status polls carry counts, revision and phase,
while query responses carry authoritative groups. Cancellation, failure,
supersession or snapshot invalidation clear partial results and reject late
publications. Likely-recording groups appear during verified pair comparison,
after fingerprint extraction; the progress text distinguishes those phases.

## Duplicate evidence

- Exact copies require SHA-256 candidates followed by complete byte comparison.
- Same audio uses the complete canonical decoded PCM, independently of tags and
  artwork. A decoding failure is skipped/reported, not treated as a match.
- Likely recording uses local acoustic fingerprints with corroborating duration
  and version checks. Fingerprint evidence covers a bounded excerpt and therefore
  remains a candidate for review, especially with edits, live sets or tape rips.

Nothing contacts an identification service. No confidence category authorizes
file deletion. Different mix/edit/live labels remain relevant evidence.

## Verification and performance

Run the existing frontend checks and behavioural suites, plus the native query,
snapshot-lifecycle and disposable-audio duplicate tests. Verify the main native
window without changing the user's media or Undo history. UI browser fixtures
stand in for the native API and are not evidence of Rust throughput.

`tools/bench-library-query.mjs` imports the actual evaluator from checkpoint
`213f06c` and feeds identical 10k/50k/100k synthetic DTOs to the Rust benchmark
example. Every query must have equal counts and result-index checksums. Report
index construction separately from warm median/p95 query latency, along with the
compiler profile, machine and iteration count. Development builds optimize the
commands crate at level 2, using the existing build cache. The isolated debug-only native
benchmark endpoints measure IPC and frontend responsiveness without replacing the
opened library. Audio-analysis cost and disk-scanning throughput are separate.

See [measured results](performance/library-query.md) and reproducible commands.
Record measured results under `docs/performance`. Claims must name which costs
were measured and which were excluded. The combined query/UI work is eligible
for a curated upstream `next` contribution; it depends on the interface work and
does not qualify as a standalone upstream `main` fix merely because it is Rust.

The [edge-case checklist](library-query-edge-cases.md) records validation targets
and future design choices, including featuring credits, ambiguous names, numeric
units, quoted text, stale replies and misleading duplicate evidence. It is not a
claim that every possible interpretation is implemented.
