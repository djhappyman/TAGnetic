# Live field updates across Files, Staged and Inspector

Status: implemented and locally verified; authorized for commit, PR, merge and
the 1.4.8 published release. Hosted platform checks remain the merge gate.
Baseline: `1a61c8b101b57a0d364dc3212deb04e69afe9950`, branch
`fix/behaviour-refinements`.

## Required behaviour

Files, Staged and Inspector are three editable views of the same working fields.
An edit in any one updates the other two immediately. The agent/MCP contributes
to that same stage. A stage is never a separate frozen preview or a second editor
buffer.

Staged shows only files with pending changes and tag columns with pending
changes, retaining the identifying file/artwork structure. Each changed field
shows its saved value and its latest proposed value. The previous value is the
saved baseline, not the last keystroke or the preceding staged edit. Repeated
edits update the proposal without moving that baseline. Files keeps its normal
configured columns and displays the current working value and marker.

For an ordinary one-field edit, retain the existing row, cell, text node and
Inspector input. Change only the affected text, dirty/error/cleared marker and
relevant summary counts. An acknowledgement with identical presentation causes
no table mutations. Preserve focus, caret, IME composition, selection, range,
column widths and scroll anchor.

In Files, Discard restores the affected values and markers in place. In Staged,
files which cease to have changes legitimately leave that filtered view. Other
rows and cells retain identity. Apply and Undo retain their explicit native
journal boundary; typing and staging never write media.

## Findings at the baseline

- `app/ui/app.js`: `renderTracks` rebuilds the view model and forces
  `renderWindow`; its normal force path replaces the tbody children. This is the
  mounted window, rather than necessarily the whole loaded library.
- `enterDiffState` and `exitDiffState` both call `renderTracks`. Initial staging,
  clearing the final staged field and Discard therefore take a structural path.
- `renderPreview` has a narrower automatic-preview path, but it constructs a new
  row and replaces every cell inside each changed mounted row. The active row
  is skipped entirely while its contenteditable cell is being edited.
- `connectSharedStage` adopts native snapshots through `renderPreview` outside
  the automatic-paint flag. A stage acknowledgement can therefore enter the
  force-render path even when membership is unchanged.
- Inline input records a live field and schedules a preview. Inspector patching
  is a separate shared-stage hydration operation; it is not the common immediate
  route for every edit. The Inspector also retains its own touched-field map.
  The inline handler currently trims the value on every input event, so it also
  cannot be an exact mirror of the person's in-progress text.
- The current editable/review decision depends on the entire plan's contents.
  A mixed rename/artwork plan can affect editing and selection behaviour for tag
  cells that should remain individually editable.
- `shared-stage.js` builds optimistic merged plans over the stage and deliberately
  skips that work above 2,048 rows. This protects responsiveness but delays the
  mirrored table value for a large stage.
- Rust staging already tracks field claims. Its event publishes a revision and
  up to 100 changed paths, but the desktop listener requests a full snapshot.
  Snapshots clone the plan and human plan. Frontend summary/diff work repeats
  across these full representations.
- Search, suggestions and duplicate query work already use Rust `QueryIndex`.
  Transforms, masks, artwork processing and write execution are already native.
  The frontend still assembles the visible grouped row model and several maps.

Graph coverage for these recent files was stale or missing. Findings above were
checked against current source, not inferred from the older graph or architecture
document. The preceding table stability tests cover scrolling and selection;
they do not establish cell identity across the full edit/ack/discard lifecycle.

## Design: one working-value module and small changes

### App-wide update boundary

An edit invalidates its own `(file, field)` projections and only genuinely
dependent results. A title edit can affect a title-based filename mask, a
number edit can affect a derived position, and validation/search may depend on
the changed value. These dependencies must be explicit. An unchanged counter,
artwork image, input, table row or unrelated panel must not be rewritten as a
side effect of another field's update. Retain existing DOM and presentation
when the projected value is identical, including native acknowledgements and
Discard. Structural changes (selected files, visible columns, group membership,
tag-block conversion, or actual artwork changes) have their own update paths.

Artwork reads follow the selected file set and saved source, independently of
scalar tag revisions. Reuse an in-flight read and cached disk summary for that
subject; project staged artwork over the cache. Never clear/reload the cover
because a text input changed. Picker/write operations still use strict subject
freshness checks. Reverse or superseded reads cannot replace the current cover.

Changing the selected file also retains the displayed image until the next
image has decoded. Identical artwork keeps its image element and inspection
state. Selection reads and routine preview validation show no progress copy.
Inspector groups and rows are reconciled by their field identities: an extra
custom tag adds that row without clearing the core form or resetting its
scroll position. Reused inputs resolve their current selected paths at the
moment of an edit, including number pairs and compilation flags.

Selection is not an edit. Without active transformations, preview scope follows
the actual pending input paths; moving selection does not invalidate that
preview or pulse Apply through a busy state. Selection-dependent transformations
retain their genuine scope and validation gates.

Introduce one frontend working-value module, used by all three views. It is a
presentation cache of Rust-owned accepted staging plus the person's newest raw
input, rather than an additional write authority.

Its small interface should support reading a field, submitting an edit intent,
accepting a native update and subscribing to affected fields. Hide overlay
precedence, validation state, revisions, dirty state and reset handling inside
the module. Callers must not construct competing merged plans or independently
choose between saved tags, imports, staged tags and local input.

Use `(session identity, file identity, field)` keys. A pending rename changes a
display value, not file identity. Initially the existing source path can identify
a file within a session; an applied rename explicitly remaps that identity and
reuses its mounted row. No new persistent database ID is required for this work.

Each field projection includes saved value, current desired value, raw input
where applicable, validation/dirty state and revision. Publish exact changed
keys, including removals and validation changes. Keep a row's staged-field count
so changing one field need not scan every other staged file.

One edit follows this route:

```text
Files cell / Staged cell / Inspector input / agent update
    -> working field changes
    -> existing views patch that field
    -> native validation/transform/stage acknowledgement
    -> patch only presentation that actually differs
```

Frontend mirrors subscribe to changes; they do not synthesize another DOM input
event when updating a counterpart. Carry origin and revision to prevent echo
loops. Do not set the active input's value back to itself or rewrite its editable
DOM while typing/composing. Acknowledged local input is retired only when its
exact revision has been accepted; older responses cannot clear newer input.
Keep in-progress whitespace and partial numeric strings intact. Any intentional
normalization belongs to the validated/computed result, not an unnoticed rewrite
of the raw input on each keystroke.
Human field claims must still be published even when the text is unchanged:
ownership and protection are distinct from a visual no-op.

### Bidirectional editing contract

All six directions are required: Files to Inspector, Inspector to Files, Staged
to Inspector, Inspector to Staged, Files to Staged and Staged to Files. The two
table tabs can continue sharing one mounted table, with the inactive view deriving
the latest values when revealed. Do not mount a hidden second large table merely
to mirror it.

An inline cell targets its own file/field. An Inspector edit intentionally targets
the current selection. Multi-file mixed values, clear-field actions, paste/range
edits and compilation/number pairs use the same intent route. Saved read-only
and derived fields remain read-only; field locks apply identically everywhere.
The presence of artwork or a filename change must not make unrelated tag fields
read-only. Normal file selection and the subset included in Apply remain separate.

### Spreadsheet edit completion

Every input mirrors immediately. Enter is an optional way to finish an edit,
not a requirement for retaining or staging it. Tab/Shift+Tab, clicking another
field, clicking outside the editor, changing the selected file or switching
views all finish the current edit and preserve its newest value. Flush the
pending intent/validation through the shared stage; do not restore the saved
value simply because Enter was not pressed. No focus transition writes media.

Track an explicit active edit session so focusout, keyboard navigation and view
changes use the same completion operation, exactly once. Finish before changing
the edit's file/selection scope or recycling its virtual row. Automatic
presentation patches must not create a spurious blur/commit cycle. Escape is the
explicit cancel operation for that edit, restoring its value at edit start
without discarding other staged fields. Invalid input remains visible with its
error state and blocks Apply; leaving focus never silently drops it. A failed
or delayed background publication leaves the pending value available for retry.

Keep raw editing and computed transform output distinct. Existing transforms
must not be applied twice or feed their output back as new raw input. Ordinary
direct edits mirror exactly; enabled transformations can intentionally produce
a different computed result, as today. Their result must never overwrite an
active raw input/caret. This semantic distinction must remain explicit in tests.

## Stable table presentation

Separate row construction, field patching and view reconciliation.

1. Construct a row/cell only when it first enters the viewport or a genuinely new
   column is added. Give each cell stable references to its current-value text,
   optional prior-value text and marker/accessibility metadata.
2. A field change resolves directly to its mounted cell. Update `Text.data` and
   only changed classes/attributes. Do not call `buildTrackRow`, replace row
   children or re-run spreadsheet range painting for an ordinary value change.
3. Unmounted rows require no DOM work. They read current working values when
   mounted. Retain the existing virtualization and measured-height index.
4. Track dependencies: track/disc changes can update Position; artwork changes
   update the existing artwork indicator; filename/mask-derived values update
   only where their dependencies require it. Never trigger artwork loading or
   metadata rereads for an unrelated title edit.
5. Measure geometry only when it can actually change. A fixed-height text edit
   does not reset all learned row heights or prefix sums. An old/new filename
   changing row height updates that row and the scroll anchor.

View membership/order is separate data: file IDs, groups, visible columns,
collapse state and layout revision. Staging a field in Files usually changes
none of it. Entering/leaving stage mode updates chrome and field markers without
rebuilding the mounted table.

When a filter, explicit sort, grouping, Staged membership or column change really
alters the view, reconcile by file/group/column identity. Insert/remove/move only
the affected items; reuse surviving nodes. Preserve current saved-versus-working
filter/sort semantics initially rather than silently adding live resorting while
the user types. Protect the active edit until its commit/cancel boundary. An
empty Staged view and newly required Staged columns are legitimate structural
changes, not excuses for resetting unrelated cells.

Discard is a working-state reset with the old changed-key set available for
reversion. It also advances a generation barrier so a delayed preview or old
hydration cannot resurrect discarded values. A genuinely new agent intent after
that barrier remains a new edit. Apply/Undo notifications patch saved bases and
remaining desired fields through the same route; preserve explicit sequential
write feedback and history behaviour.

## Rust and background work

### Include in this implementation

- **Native stage deltas.** Reuse Rust's field claim ledger. A versioned request
  from a known revision returns changed fields, removals, relevant validation
  metadata, affected-file counts and a new revision. Use bounded change history
  and an explicit full-resync result for a revision gap, new session or overflow.
  Full recovery snapshots also reconcile through the field cache; they do not
  require a DOM reset. Keep existing clients compatible during transition.
- **Small edit intents.** Send changed human fields and an expected generation,
  rather than repeatedly transmitting the whole human/staged plan on a keystroke.
  Preserve protected human fields, scoped Apply, principal isolation, idempotency,
  cancellation and recovery. Rust continues to seal the current validated plan
  for Apply.
- **Native validation and transforms.** These already run in Rust; narrow work
  to affected fields plus actual rule/mask dependencies. Reuse loaded metadata
  and prepared rule pipelines where correctness permits. Do not rebuild/re-read
  every pending file or re-encode staged artwork for a simple tag edit. Detect
  rules with broader dependencies and recompute their necessary scope explicitly.
- **Artwork references.** Return stable lightweight artwork identity/state in
  stage deltas. Fetch thumbnail/bytes only when the relevant artwork actually
  changes or a view needs them. Retain native pixel/byte limits and image safety.
- **Scheduling.** Coalesce superseded validation requests before costly work;
  cancel obsolete read-only work between chunks. Work over immutable native
  snapshots can run separately from serialized mutation admission. Do not merely
  add threads around the same `App` lock. Keep Apply/Undo/recovery serialized and
  never abandon a mutation midway without its existing rollback contract.

### Include where measurements justify the native calculation

- Extend the existing Rust query snapshot with large-library sort/group/view
  manifests, returning ordered IDs and group metadata. This can remove repeated
  frontend whole-library passes. Preserve hand order, folder order, duplicate
  grouping, rename destinations and Staged semantics exactly. Do not create a
  second native index or send all track DTOs back on each query.
- Native aggregate field values for very broad Inspector selections, using a
  session selection handle and working-stage revision. A one-file edit still
  mirrors locally; recomputing a 100k-file mixed-value selection must not block
  the frontend. Measure first and reuse native snapshots rather than reread disk.
- Dependency-aware custom-column rendering over staged values, using the existing
  Rust mask engine. Keep saved-value versus desired-value semantics explicit.

Keep focus, caret, selection visuals, pointer interaction, animation, viewport
windowing, pixel measurements and direct text updates in the frontend. These
must not wait for Rust. Native libraries already handle the expensive algorithms;
the main gain here is eliminating unnecessary work, payloads and shared-lock
contention. Moving DOM-oriented bookkeeping to Rust would add round trips.

## Delivery order and ownership

1. **Reproduction and contract:** extend browser/native-shaped fixtures to count
   row, cell and text-node retention across the exact reported lifecycle. Capture
   a failing baseline before changing the app. No live-media writes or backups.
2. **Working fields and mirrors:** introduce the working-value module and route
   Files, Staged and Inspector through it. Replace row rebuilds with field
   patches, including first edit, last-field revert and Discard.
3. **Structure and dependencies:** reconcile Staged rows/columns and real view
   changes; preserve editor identity, geometry, selection and derived columns.
4. **Native deltas and scheduling:** replace full stage refresh/publication with
   revisioned changes and dependency-scoped validation. Keep snapshot recovery.
5. **Measure and gate:** profile album-sized sets and 20k/50k/100k libraries with
   small and large stages. Advance optional native view/aggregate work only when
   measurements show a meaningful cost after the earlier fixes.

Likely frontend ownership: `app.js`, `state.js`, `live-preview.js`, `shared-stage.js`,
`editor.js`, `generator.js`, `columns.js`, `staging.js`, and small new working-field
and cell/view modules. Native ownership: `crates/service/src/staging.rs`, the
desktop MCP control adapter, affected preview helpers and their tests. Touch
spreadsheet/geometry only where the new narrow update interface needs it.

Do not layer another full renderer beside the existing one. Remove superseded
diff-state repaint branches as their callers migrate. Avoid unrelated styling,
framework rewrites, journal migrations or a backend-wide refactor.

## Completion gate

- All six mirror directions work per input event, including Staged edits inside
  agent-generated mixed tag/artwork/rename plans.
- Staged contains only changed files/tag columns with saved-to-latest values;
  repeated edits preserve the saved baseline. Identifying structure and existing
  artwork/rename-only review remain usable.
- Tab, Shift+Tab, click-away, selection changes and view switches retain the
  latest edit without Enter. Completion happens once and targets the original
  file(s); invalid values survive focus loss and no focus event writes media.
- Ordinary one-field typing, native validation and acknowledgement retain every
  mounted row/cell/input. Only affected text/markers/dependencies change. An
  identical acknowledgement produces zero table mutations.
  Count application-driven replacements separately from the browser's own
  contenteditable/IME text-node operations; the application must not rewrite the
  active editing DOM to satisfy a mirror update.
- Discard and last-field reversion restore values without remounting surviving
  Files cells. Staged membership changes preserve all surviving node identities.
- Focus/caret, IME, paste, Escape, invalid values, locks, mixed selections,
  selected rows/ranges, scroll anchors and column widths remain correct.
- Out-of-order results, discard during pending validation, concurrent agent
  fields, user correction of an agent field and a dropped delta/resync do not
  lose newer work or resurrect old work. Test fixture mutation ordering must
  reflect native atomicity rather than introduce impossible intermediate states.
- No whole-stage/library scan, full-plan stringify/copy or artwork transfer on
  the frontend's ordinary single-field input path. Offscreen rows are not mounted
  for updates. Work scales with affected keys and mounted dependants.
- Profile input-to-next-paint and task duration on the actual Linux WebKit app,
  alongside browser fixture mutation assertions. Target next-frame local
  feedback under the stated test workload; record hardware, p50/p95/max and
  background contention rather than promise a universal timing bound.
- Run focused regression checks first, then the appropriate integrated suites
  and exact-head platform gates at the implementation checkpoint. Preserve the
  current native app, pending user work and Undo; write tests use disposable
  fixtures only.

Upstream route: shared UI behaviour is eligible for a curated `tagrex/tagrex:next`
contribution after its foundation; standalone compatible native improvements
route to `main`. Fork-specific storage, identity and release policy remain local.
The user has now authorized implementation with Codex Sol 6.1 and DeepSeek V4.1
Flash 1017 through OpenCode Go, integrated verification, commit, PR and merge.
The user subsequently requested a new published release after merge and
retirement of the merged branch. The prepared version is 1.4.8.

## Integrated result and verification

- Small human intents, bounded stage deltas and lazy artwork references are
  implemented in Rust. Raw values and transformation bases stay distinct;
  session rotation and expired bases prevent stale or compounded updates.
- Files, Staged and Inspector retain existing field DOM, raw input and scroll.
  Saved artwork reads follow their own subject, retain identical images and
  remain independent of scalar edits. Staged saved/new values keep their
  baseline; blur retains the latest edit.
- Collections up to 512 view items remain mounted. Larger collections retain
  buffered windows; scrolling inside that buffer performs no row work, and
  disjoint jumps retain the table body/spacers. The scroll-event guard follows
  the last painted offset, preventing a stale blank window after a reset.
- Native WebKit comparison identified missing scrollport layout/paint
  containment as the remaining major scrolling cost. Controlled alternating
  comparisons reduced p95 frame time from 81–129 ms to 18 ms on this machine.
  See `docs/performance/live-field-scroll-results.json` for method and limits.
- Integrated local checks: 196 JS units, 127 service tests, 61 desktop tests
  and 2 catalogue checks passed; formatting, workspace Clippy, Svelte checks,
  lint and production build passed. The full UI run had 617 passes, 11 opt-in
  skips and two obsolete small-library virtualization assumptions; both were
  corrected to 1200-file fixtures. The final four affected scrolling,
  transformation, keyboard-contrast and playing-row checks all passed.
- Review findings fixed: block-removal deltas, early resync retention across
  bulk intent chunks, transformation scope transfer into the filename pipeline,
  and coalesced withdrawal scopes. Native metadata identifies exact derived
  fields without treating unrelated human inputs as transformation output.
- Optional native view/Inspector aggregate migrations were measured and
  deferred. The existing native query manifests already cover ordering; adding
  another transfer/index without a demonstrated remaining bottleneck is not
  justified. Single-field presentation and pixel measurements remain local.

Native intent behaviour is covered by service tests and native-shaped browser
fixtures. The original live binary/profile was retained for compatibility and
WebKit measurements; no new-native GUI execution, Windows/macOS GUI test or
live-user-media write is claimed. Existing user staging and Undo were preserved.
