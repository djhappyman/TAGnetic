use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolDefinition {
    pub name: String,
    pub description: String,
    pub input_schema: Value,
}
/// Compact, explicit application API. Every method is permissioned at execution;
/// descriptions and read-only annotations never serve as authorization.
pub fn tool_catalog() -> Vec<ToolDefinition> {
    let entries:[(&str,&str,&str);58]=[
        ("session.status","Inspect the shared session, this client's grants, and recovery state.",""),
        ("session.open","Open approved files/folders in the existing desktop session; refuses pending human work.","paths"),
        ("session.refresh","Refresh the current desktop file table without replacing pending desired edits.",""),
        ("schema.catalog","Read supported fields, masks, built-in transforms, query grammar example and limits.",""),
        ("files.snapshot","Capture approved open files once; returns an opaque snapshot for paged reads.",""),
        ("files.page","Read up to 500 rows from a captured snapshot, optionally projecting tag fields.","snapshot offset limit fields"),
        ("files.query","Query a captured snapshot in native Rust; returns a private selection handle and a page.","snapshot query offset limit fields lane query_request"),
        ("files.select","Select snapshot indices in explicit order, or all rows; private to this client.","snapshot indices"),
        ("files.read","Read up to 500 currently saved tracks in an explicit private selection.","selection paths fields"),
        ("files.locks","Read global field locks; plans cannot bypass these locks.",""),
        ("files.write","Direct metadata write from desired values, bypassing visible staging while retaining validation, journal and human edits. Requires idempotency_key.","selection paths edits values idempotency_key"),
        ("files.trash","Move selected files to system trash under an explicit trash grant; ordinary Undo is unavailable.","selection paths confirmation"),
        ("draft.create","Stage desired tags immediately into the same live Staged area as the desktop, without writing disk. Require top-level paths or selection. edits items are {path,field,value}; values is a field-to-string/null map applied to that scope. Explicit values replace prior draft intent even when equal to saved tags. Returned files counts changed files, not evaluated files.","selection paths edits values draft"),
        ("draft.get","Read the sanitized, paged diff of an owned draft; files is the number with actual changes, not the number evaluated. Artwork bytes use separate handles.","draft offset limit"),
        ("draft.discard","Discard an owned draft without writing files.","draft"),
        ("draft.transform","Preview ordered tag rules against current shared working values. Native kinds: format (scope destination, to pattern), remove_fields and keep_fields (from semicolon-separated names). Keep-only preserves artwork. Existing text transformations remain supported.","selection paths rules groups scope draft"),
        ("draft.rename","Preview renaming with the application's mask grammar, including sidecars.","selection paths mask draft"),
        ("draft.move","Preview move/copy under an already-authorized destination. Directory pruning is disabled.","selection paths mask destination copy draft"),
        ("draft.from_name","Extract tags from selected filenames using a reusable mask.","selection paths mask draft"),
        ("draft.from_text","Extract tags from ordered text lines using the app's mask grammar.","selection paths mask text draft"),
        ("draft.number","Preview ordered track/disc numbering with optional totals and zero padding.","selection paths start step total width write_total disc disc_total draft"),
        ("draft.import","Preview release tags with explicit file order and zero-based release-track indices. Known track totals are per disc, not whole-release counts. Require top-level paths or selection. Uses a fetched release receipt or typed import values.","selection paths release track_indices import vinyl_sides_to_disc draft"),
        ("draft.artwork","Preview owned artwork on top-level paths or selection. mode native preserves existing non-front images; replace_all explicitly replaces every picture; replace_front preserves other roles; append retains the pending set. remove=true removes all. Role metadata is shown in the diff.","selection paths asset remove mode draft"),
        ("draft.clear_tags","Preview removing saved tags; full field grant required when multiple fields are touched.","selection paths draft"),
        ("draft.remove_block","Preview stripping one tag-block kind, retaining native recovery images.","selection paths kind draft"),
        ("draft.convert_block","Preview converting between supported tag blocks, with loss details.","selection paths from to revision draft"),
        ("draft.fix_encoding","Preview rewriting legacy text encoding using native tag-block protections.","selection paths draft"),
        ("plan.commit","Apply CURRENT shared values within the owned live field scope. scope_digest (or digest alias) authorizes the scope, not frozen values; accepted_digest in the operation receipt seals the actual admitted values. Exact idempotency-key retries reuse the original receipt.","draft scope_digest digest idempotency_key"),
        ("operations.status","Inspect this client's durable operations; accepted writes survive a disconnected client.","operation"),
        ("operations.cancel","Request cancellation/rollback of an owned operation; inspect status for the terminal outcome.","operation"),
        ("history.list","Read this client's committed batches and the guarded session-only Redo state.",""),
        ("history.undo","Strictly Undo a batch owned by this client's operation; no foreign/human batches.","operation"),
        ("history.redo","Guarded replay of this operation's exact top Redo entry; requires latest redo_revision.","operation redo_revision"),
        ("settings.get","Read redacted music-management preferences and their revision; no account/proxy secrets.",""),
        ("settings.patch","Patch allowed music preferences with compare-and-swap; refuses stale or security settings.","revision patch"),
        ("presets.list","List native reusable filter, mask, or transform presets with CAS revision.","kind"),
        ("presets.put","Create/update one validated reusable preset. kind filter requires preset={query:{version:2,...}}, mask requires preset={mask:pattern}, transform requires preset={scope:field,rules:[...]}. revision is SHARED across all settings/preset kinds: use the returned revision or read again after every change.","kind name preset revision"),
        ("presets.remove","Remove one named preset with expected revision.","kind name revision"),
        ("masks.render","Render up to 500 selected working tracks with the actual Rust mask grammar.","selection paths mask"),
        ("providers.status","Read connected metadata-source status without exposing tokens.",""),
        ("providers.search","Search an authorized connected metadata source using shared rate limits and accounts.","source query"),
        ("providers.release","Fetch a release and receive owned release/image receipts for explicit track mapping.","source id"),
        ("artwork.fetch","Fetch a provider-issued image receipt; arbitrary authenticated URLs are not accepted.","image"),
        ("artwork.load","Validate and pool a local image inside approved roots; returns an artwork handle.","path kind"),
        ("artwork.upload","Validate/pool a small base64 PNG/JPEG/WebP; large assets should use artwork.load.","data_base64 kind"),
        ("artwork.inspect","Read artwork digest, MIME and byte size without duplicating image bytes.","asset"),
        ("artwork.read","Explicitly read a bounded base64 chunk from an owned artwork handle.","asset offset limit"),
        ("artwork.embedded","Read one selected file's embedded cover into an artwork handle.","selection paths"),
        ("artwork.export","Save an owned artwork asset (asset+filename) or selected embedded covers under an authorized root using confined native outputs.","selection paths asset root filename basename overwrite preview offset limit"),
        ("exports.create","Export selected tracks as playlist, cue, csv, html, xml or mask-based report under an approved output root.","selection paths root filename format mask overwrite split pattern preview offset limit"),
        ("analysis.duplicates","Run an explicitly scoped offline duplicate analysis; action start/status/cancel.","selection paths kind action job group offset limit"),
        ("analysis.bpm","Run native BPM analysis for an explicit selection, subject to configured limits.","selection paths action job offset limit"),
        ("playback.status","Read the existing desktop player's status.",""),
        ("playback.play","Play one explicitly selected approved file using the desktop player.","selection paths"),
        ("playback.pause","Pause the desktop player.",""),
        ("playback.stop","Stop the desktop player.",""),
        ("playback.seek","Seek the desktop player to a time in seconds.","seconds"),
        ("playback.volume","Set desktop player volume (0..1).","volume"),
    ];
    let mut tools = entries
        .into_iter()
        .map(|(name, description, keys)| ToolDefinition {
            name: name.into(),
            description: description.into(),
            input_schema: schema(keys),
        })
        .collect::<Vec<_>>();
    tools.extend([
        ToolDefinition{name:"staging.get".into(),description:"Read bounded metadata-only current shared staged differences within this client's granted roots and fields. Saved disk values remain available through files.read; no artwork bytes or foreign handles are exposed.".into(),input_schema:schema("offset limit")},
        ToolDefinition{name:"queries.cancel".into(),description:"Cancel only this client's named query lane; desktop and other clients are unaffected.".into(),input_schema:schema("lane query_request")},
        ToolDefinition{name:"files.selection_page".into(),description:"Read bounded pages of a private query/selection result without recreating its handle.".into(),input_schema:schema("selection offset limit fields")},
        ToolDefinition{name:"providers.release_tracks".into(),description:"Read a bounded page of tracks from an owned fetched release, with explicit zero-based indices.".into(),input_schema:schema("release offset limit")},
        ToolDefinition{name:"draft.list".into(),description:"List this client's live shared-stage scope handles and scope digests.".into(),input_schema:schema("")},
        ToolDefinition{name:"files.field".into(),description:"Explicit bounded Unicode chunks of one saved tag (for large lyrics/comments).".into(),input_schema:schema("selection paths field offset limit")},
        ToolDefinition{name:"ui.present".into(),description:"Present an existing application tab or owned draft; typed desktop actions only.".into(),input_schema:schema("module path draft principal_id")},
        ToolDefinition{name:"ui.preferences".into(),description:"Read/set supported desktop view preferences without arbitrary JS or shell execution.".into(),input_schema:schema("theme palette value_font table_font_px")},
        ToolDefinition{name:"handles.release".into(),description:"Release an owned snapshot, selection, artwork, release or image handle.".into(),input_schema:schema("handle")},
    ]);
    tools
}
fn schema(keys: &str) -> Value {
    let mut properties = serde_json::Map::new();
    for key in keys.split_whitespace() {
        let value = match key {
            "offset" | "limit" | "start" | "step" | "total" | "width" | "disc" | "disc_total"
            | "redo_revision" | "group" | "table_font_px" => json!({"type":"integer","minimum":0}),
            "seconds" | "volume" => json!({"type":"number","minimum":0}),
            "preview" | "copy" | "write_total" | "vinyl_sides_to_disc" | "remove" | "overwrite" => {
                json!({"type":"boolean"})
            }
            "paths" | "fields" => json!({"type":"array","items":{"type":"string"}}),
            "indices" | "track_indices" => {
                json!({"type":"array","items":{"type":"integer","minimum":0}})
            }
            "edits" => {
                json!({"type":"array","description":"Exact {path,field,value} items. Top-level paths or selection is also required.","items":{"type":"object","properties":{"path":{"type":"string"},"field":{"type":"string","description":"Storage key, e.g. title, tracktotal, custom:MYFIELD"},"value":{"type":["string","null"]}},"required":["path","field","value"],"additionalProperties":false}})
            }
            "rules" | "groups" => json!({"type":"array","items":{"type":"object"}}),
            "query" | "values" | "import" | "preset" | "patch" => json!({"type":"object"}),
            _ => json!({"type":"string"}),
        };
        properties.insert(key.into(), value);
    }
    let mut result = json!({"type":"object","properties":properties,"additionalProperties":false});
    let keys = keys.split_whitespace().collect::<Vec<_>>();
    if keys.contains(&"paths") && keys.contains(&"selection") {
        result["anyOf"] = json!([{"required":["paths"]},{"required":["selection"]}]);
    }
    result
}
