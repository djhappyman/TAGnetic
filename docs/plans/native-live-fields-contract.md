# Native live-fields contract (desktop ⇄ Rust stage)

Status: first draft written before implementation, owned by the native worker.
Codex owns `app/ui/**`; this file is the interface both sides code against.
Baseline `1a61c8b101b57a0d364dc3212deb04e69afe9950`. Parent owns the main design
doc (`live-field-updates.md`) and final integration.

The native stage already owns every working value. This contract replaces two
per-edit costs: sending the whole human/staged plan on a keystroke, and
re-reading/cloning the whole stage on every acknowledgement. It adds no second
write authority and no second index of stages: deltas are projections of the
existing field claim ledger plus its two plans.

## 1. Capability detection (works against the old binary)

Everything below is additive. The old commands keep their exact behaviour and
shape, so the current UI keeps working against a new binary and the new UI must
keep working against an old one.

Ask `mcp_status` (exists in every build). A build with the live-field backend
returns:

```json
"live_fields": {
  "version": 1,
  "delta": true,
  "intents": true,
  "recompute": true,
  "derived_scope": true,
  "session_check": true,
  "asset_refs": true,
  "session": "stage_<hex>",
  "history_revisions": 128,
  "history_fields": 8192,
  "max_delta_fields": 2048
}
```

`recompute`, `derived_scope` and `session_check` are additive flags (same `version`). The UI uses
the incremental route only when `live_fields.version >= 1` and the needed flag
is true. Otherwise it uses:

- `mcp_shared_stage` for state (full `SharedStageSnapshot`, unchanged),
- `mcp_publish_shared_stage` for publication (full plan, unchanged),
- `mcp_clear_shared_stage` for Discard (unchanged).

`live_fields` is also returned by the service's `shared_stage_status`. It is not
an MCP agent tool; external agent customers keep `staging.get` paging and the
existing draft tools unchanged.

## 2. Commands

All three are trusted-desktop Tauri commands (same window guard as every other
`mcp_*` command). They are not MCP agent tools and take no grant.

### `mcp_shared_stage_delta`

Args: `{ "since": 39 | null, "session": "stage_..." | null }`

Returns `StageDelta` (§3). `since`/`session` are the revision and session the
caller already holds. The first call (or any resync) passes what it had;
`null`/`null` is an initial sync.

### `mcp_publish_shared_intents`

Args:

```json
{
  "expected_revision": 41,
  "session": "stage_ab12",
  "recompute": false,
  "intents": [ { "path": "/music/a.flac", "field": "title", "value": "B" } ],
  "groups": [],
  "paths": []
}
```

- `field` is a tag storage key (`title`, `albumartist`, …). `value: null` or
  `""` clears the field. Any other value is stored verbatim; the raw string is
  never trimmed or normalised by this route.
- Every intent is a human field claim even when the value equals the saved
  value: ownership protection is published, the visible stage may not change.
- A same-field intent that returns to the saved value removes that staged field
  and its claim, exactly like the full-plan publish.
- `session` is optional (old UI omits it). When present it must equal the
  current stage session **at mutation**, even if `expected_revision` is
  numerically plausible. A mismatch changes nothing and returns
  `stage_session_mismatch`; the caller resyncs with `mcp_shared_stage_delta`.
- `recompute: true` allows `intents: []`. It is the explicit preset/scope
  recompute: `paths` is the scope, and `groups` is the currently enabled
  transform set (empty means no preset). Native restores the current raw human
  inputs and the current agent/other-contribution basis inside the scope,
  re-runs the supplied groups once, and publishes only the staged fields whose
  fingerprint actually changed. Fields a removed preset produced (transform
  output with no raw human input) are removed again; unchanged fields keep
  their existing owner. Without `recompute`, `intents: []` is still
  `invalid_argument`.
- `recompute` with new intents is allowed and applies the intents before the
  scope recompute; the intent raw value always overrides the seeded value.
- The recompute scope is exactly `paths`: derived output from a previous preset
  run on a path outside this list is left staged. A caller that disables a
  preset and wants every earlier scope retired must send the union it still
  owns; this keeps the operation bounded to the real scope it declares.
- `groups`/`paths` are optional and only for enabled transformation groups. When
  `groups` is non-empty the native preview recomputes the whole supplied scope
  because a rule can rewrite any field in its scope. Ordinary typing sends
  neither and only the named fields/read are touched.
- `artwork`, `filename` and `block:*` are refused here with
  `invalid_argument`: use `mcp_publish_shared_stage` (unchanged) for them.

The raw-input/derived-result distinction is bounded native state inside the
existing stage: at most 8,192 recorded fields and 8 MiB of recorded text
(shared by raw inputs and derived bases), evicted oldest-first. It is not a
second plan, index or write authority. It is cleared by Discard and by a
library-session rotation, and retired for written paths by Apply. While a
record survives, a non-idempotent transform cannot compound: the derived output
is never stored as the next raw input. The field claim records whether its value
is derived. If both records for such a field are evicted, recomputing its scope
fails with `raw_basis_expired` without changing the stage. An explicit new intent
for that field supplies a fresh basis; otherwise Apply or Discard clears that
stage. The previous transformed output is never silently fed back as input.

Groups containing enabled `filename`/`fileext` rules return `full_plan_required`
on this tag-only endpoint. The UI retains the full-plan route for those rules.

Returns `StageIntentOutcome` (§4). The delivery is synchronous with respect to
the desktop session queue: by the time it resolves, the fields are in the stage
or the result says why not. No media is written.

### `mcp_shared_stage_asset`

Args: `{ "asset": "art_<hex>" }`

Returns `{ "asset", "mime", "kind", "description", "data_base64", "bytes",
"revision" }`. Only assets named by a delta the caller received are guaranteed
to resolve while the stage revision that named them is current (see §5).

## 3. `StageDelta`

```json
{
  "session": "stage_ab12",
  "revision": 41,
  "human_revision": 40,
  "from": 39,
  "resync": false,
  "resync_reason": null,
  "files": 812,
  "staged_fields": 1204,
  "changed_files": 1,
  "changed_fields": 1,
  "removed_fields": 0,
  "changes": [ { "path": "/music/a.flac", "field": "title",
                 "tag": { "old": "A", "new": "B", "invalid": false,
                          "old_chars": 1, "new_chars": 1, "truncated": false } } ],
  "removed": [],
  "removed_paths": [],
  "snapshot": null,
  "snapshot_required": false,
  "elapsed_ms": 0.4
}
```

Semantics:

- `changes` contains one entry per **currently staged field** that changed after
  `from`. A field that left the stage appears only in `removed`
  (`[{ "path", "field" }]`). A file with no staged field left also appears in
  `removed_paths`.
- A reported field is authoritative for that `(path, field)`. Consumers must not
  infer other fields from it.
- `human` is true when a human touch currently owns/protects that field (an
  agent-only field is false). The reported value is always the merged stage
  truth, not the layer value.
- `old` is the saved baseline, `new` the latest proposal. Repeated edits move
  `new` only. Values are capped at 4096 characters per string with
  `truncated: true`; `old_chars`/`new_chars` are the full lengths. Full values
  remain available through `mcp_shared_stage`/`staging.get` for fields that
  legitimately need them.
- `files`, `staged_fields` are whole-stage totals (O(1) in native, from the
  claim ledger). `changed_files`/`changed_fields`/`removed_fields` describe this
  delta only.
- A delta never reports more than 2048 affected fields: a larger change (a whole
  stage discard, a 100k-file agent sweep) returns `resync` with reason
  `overflow` instead of a multi-megabyte payload.
- `resync: true` means: discard incremental state and adopt `snapshot` (a full
  `SharedStageSnapshot`, identical shape to `mcp_shared_stage`) if present.
  Otherwise `snapshot_required: true` means `files` is over the inline bound
  (4096 files / 8 MiB) — call `mcp_shared_stage`. On a resync `changes` and
  `removed` are empty; membership comes from the snapshot.
- `resync_reason`: `initial`, `session`, `gap`, `overflow`, `structural`.
  `structural` is used when a rebase after Apply/rename moved file identities;
  a stale incremental walk could otherwise resurrect old paths.
- Native never returns fields older than `revision`; a delta computed at
  revision *n* must be treated as stale if the caller already holds a newer
  revision. Ignore any acknowledgement whose `revision` is lower than the
  revision the caller has adopted.

### Field payloads

Only one payload object is present per field entry:

Each entry also carries `human` ownership and `derived` provenance. `derived`
is true only when a human transformation changed that field's raw/base value;
unrelated raw fields in the same request remain false. Full snapshots add
`derived_fields: [{path, field}]` so a reloaded UI or a switch to the full-plan
filename pipeline retains exactly the earlier transformation scope. Consumers
must not infer that every field in a transformed file is derived. The UI gates
native preset recomputation on `derived_scope` as well as `recompute`.

- tag: `"tag": { "old", "new", "invalid", "old_chars", "new_chars",
  "truncated" }`. `new: null` with `invalid: false` is a pending clear.
- artwork: `"artwork": { "empty": false, "invalid": false,
  "old": [ cover ], "covers": [ cover ] }`. `covers` is the new (proposed)
  set and is retained for compatibility; `old` is the saved baseline. No image
  bytes in a delta; `empty: true` with `covers: []` is a pending removal of all
  images. `invalid` is true when any new cover exceeds the native image budget
  or is not an image MIME; each `cover` also carries `invalid` so a consumer can
  mark/locate the bad image. Old refs are pooled too, so a pending artwork
  change can render both sides lazily without taking a whole snapshot. An
  expired reference still fails `mcp_shared_stage_asset` with `asset_expired`
  and the consumer must resync.
- cover: `{ "asset", "mime", "kind", "description", "bytes", "invalid" }`.
- filename: `"rename": { "to", "copy", "creates_folder", "rename_root",
  "sidecars": [[from, to]], "folder_extras": [path] }`.
  `rename_root` is the mask-built root (absent for a change no mask built) and
  `folder_extras` names the sidecars that belong to the folder rather than the
  track. Both travel so a cached scalar row keeps destination grouping and
  folder-extra cleanup exactly as the full snapshot would.
- tag block: `"block": { "kind", "revision", "text_to_unicode", "chars",
  "truncated", "invalid", "text", "covers": [artwork cover] }`. `text` is the
  block's lyric-bearing tag, capped like a tag value. A block edit still uses the
  full-plan publication route and the snapshot for its complete content; the
  block delta remains a bounded summary with explicit full-snapshot fallback.

## 4. `StageIntentOutcome`

```json
{
  "accepted": true,
  "revision": 42,
  "human_revision": 42,
  "session": "stage_ab12",
  "files": 1,
  "staged_fields": 1,
  "locked": [ { "field": "title", "files": 1 } ],
  "delta": { }
}
```

- `accepted: true`: the intents were applied at `revision`. `delta` is the
  change set from `expected_revision` to that revision, so the common case is
  one request and no follow-up read.
- `accepted: false`: a **newer human revision** already exists. Nothing was
  applied. `delta` still describes what the caller missed from its
  `expected_revision`, so the UI can retire stale input and retry only its
  newest raw value. A stale acknowledgement must never be painted over newer
  text.
- `expected_revision > revision` is the one hard error: the caller belongs to a
  different session; adopt a snapshot.
- `locked` mirrors the existing `locked_skipped` report for the same fields.
- `human_revision` advances only for human input; an agent publication raises
  `revision` without invalidating a pending human intent (it merges).

Measured native cost of this route at a 100k-file stage (unoptimised test
profile, one file's real preview plus the patch and delta): first intent after a
full-plan publication 63–130 ms, because the per-layer path index is rebuilt
once (roughly 30–65 ms per 100k-change layer); every following intent 0.1 ms and
the delta itself 0.005 ms with a ~600-byte payload. The old full-plan publication
of the same stage measured 2.45–3.17 s. The index is intentionally lazy: a publication that
no delta/intent follows pays nothing, and the one-off rebuild happens at the
first edit rather than on every keystroke.

## 5. Artwork references and lifetime

- A delta reports stable content-addressed ids (`art_<sha256>` over the image
  data plus mime/kind/description), not bytes. Ids are stable across revisions
  and identical for identical images.
- `mcp_shared_stage_asset` serves the exact pooled bytes the stage would embed
  (no re-encode, no decode path of its own). Same size/quality limits as
  artwork upload/embed apply upstream; this command adds no new image input.
- An asset is fetchable while it is named by the current stage plan, and for a
  bounded grace window after (native keeps at least the most recent 512
  references / 64 MiB). After eviction the command fails with `asset_expired`;
  refresh the delta (or the snapshot) — do not assume old ids stay valid
  forever.
- Only fetch artwork when the artwork field actually changed, or when a view
  first needs that image. A title edit never carries artwork bytes.

## 6. Events

`tagrex://agent-event` payloads keep every existing key. `shared_stage_changed`
now also carries:

```json
{ "session": "stage_ab12", "revision": 42, "human_revision": 42,
  "files": 1, "staged_fields": 1, "changed_fields": 1,
  "changed": [ { "path": "/music/a.flac", "field": "title" } ],
  "changed_paths": ["/music/a.flac"], "resync": false }
```

`changed` is capped at 200 entries; `resync: true` (also capped) means the
consumer should call `mcp_shared_stage_delta` (or `mcp_shared_stage`) rather
than trusting the list. The event is a wake-up, not the payload: the delta
command is the payload. Events are emitted after the mutation and revision are
committed.

## 7. Revision, session and reset rules

- `revision` is monotonic per native stage session. Human intents, agent
  publications, Discard and Apply-completion each increment it.
- `human_revision` is the last revision produced by human input. A human intent
  whose `expected_revision` is below it is rejected (`accepted: false`); an
  agent publication between the preview and the publication does not reject it
  and is merged.
- `session` is opaque and per stage lifecycle. A different/unknown session always
  resyncs. History is bounded (128 revisions / 8192 changed fields); an older
  `since` resyncs with `gap`/`overflow`.
- Opening a library (or a file set) starts a new stage lifecycle: native clears
  the in-memory stage, advances `revision` and `human_revision`, rotates the
  session and records a structural boundary. A queued or delayed intent from the
  previous library session can therefore never stage old paths into the new
  session, even if its revision number would otherwise be plausible. The
  optional `session` argument on `mcp_publish_shared_intents` is checked against
  the current session under the mutation lock; a mismatch is
  `stage_session_mismatch` and changes nothing.
- Discard (`mcp_clear_shared_stage`) advances the revision and the human
  revision: a delayed preview or intent from before the Discard cannot
  resurrect discarded values. A genuinely newer intent after Discard is a new
  edit. Incremental deltas crossing a Discard report the removals, so no DOM
  reset is needed for the survivors.
- Apply/Undo keep the existing journal boundary. Apply-completion removes the
  applied fields through the same delta route and retires their raw-input/derived
  records; a rebase that moved identities forces `structural` resync.
- The full snapshot commands (`mcp_shared_stage`, `mcp_publish_shared_stage`,
  `mcp_clear_shared_stage`) keep their exact request/response shape and remain
  the fallback when the capability flags are absent.

## 8. What this contract does not move

Focus, caret, IME, selection, scroll, geometry, view/column reconciliation,
number parsing and local optimistic mirroring stay in the frontend. Native
provides field truth, validation results, membership, counts and artwork bytes
on demand. The frontend keeps its working-value module as a presentation cache;
it must not build a second stage authority.
