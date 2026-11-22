# External agents

TAGnetic exposes music-management operations to MCP clients through the same
native service used by its desktop. An agent has its own granted scope and
shares the desktop’s actual Staged area. A connector process never opens a
second library or journal.

## Connect a local client

1. Open a music folder, then go to **Settings → MCP connection** and choose
   **Pair Codex** or **Pair OpenCode**. Tagnetic enables the connection and saves
   the private credential file automatically. Each pairing grants the complete
   workflow within that folder: reading, shared staging, writing, exports,
   providers, presets, history, playback and UI access. Trash is not granted.
2. Click **Copy install command**, paste it into your terminal and run it.
   This registers the connector with the chosen client. Pairing alone does not
   install it, and no hand-edited JSON or TOML is needed. The generated command
   includes only the credential file's path, never its secret contents.
3. Run `codex mcp list` or `opencode mcp list` and check that `tagnetic` is listed.
   Start a new agent session and ask it to show the music folder open in Tagnetic.
4. Keep the desktop open. The connector reports unavailable when no desktop
   owns that profile; it does not silently launch another session.

Pair Codex and OpenCode separately. The header's **Agents** popover shows live
client names, current work and brief stage or save results. When supplied by the
client, the model appears beside its name. A client can report it in MCP request
metadata under `io.tagnetic/model` or set `TAGNETIC_MCP_MODEL` in its connector
environment.

Use the row controls to pause or resume edits, stop an active save, or
disconnect and reconnect that session. Paused sessions can still read the
library. Hiding a row keeps its session active. Use **Show in Agents** in
Agent Connections settings to restore it.

Use **Connect an agent** in the empty popover or the plus beside **Agents** to
open setup. If the listener is disabled and a pairing is still valid, choose
**Reconnect agents** in the popover. Draft review, Undo and
interrupted-operation recovery are available through the desktop.

Example Codex configuration (substitute the absolute installed paths):

```toml
[mcp_servers.tagnetic]
command = "/absolute/path/to/tagrex"
args = ["--mcp-stdio", "--mcp-profile", "/absolute/app-data/profile", "--mcp-credential", "/private/path/codex.credential"]
```

Example OpenCode V2 project configuration, `opencode.jsonc`:

```json
{
  "$schema": "https://opencode.ai/config.json",
  "mcp": {
    "servers": {
      "tagnetic": {
        "type": "local",
        "command": ["/absolute/path/to/tagrex", "--mcp-stdio", "--mcp-profile", "/absolute/app-data/profile", "--mcp-credential", "/private/path/opencode.credential"],
        "disabled": false,
        "codemode": false,
        "protocol": "auto"
      }
    }
  }
}
```

The command is an argument array, not shell text. Paths containing spaces stay
single arguments. On Windows use the installed `.exe` and JSON-escaped
backslashes; on macOS use the executable inside the application bundle. For an
AppImage, use its stable installed AppImage path, not a temporary mount path.
The profile is the directory selected by the desktop's application-data path,
not a music folder and not the connector's working directory.

Instead of the two path flags, the connector also accepts
`TAGNETIC_MCP_PROFILE` and `TAGNETIC_MCP_CREDENTIAL_FILE`. These variables hold
paths, never credential values. Explicit flags take precedence.

Start with `session.status` and inspect the server's tool catalogue. Do not
infer permission from a tool being listed: the service enforces each client's
grant on every call. Provider credentials remain inside TAGnetic. Granting a
provider operation does not disclose its account token.

## Workflows and recovery

Read projected saved-file pages, then stage desired edits. They appear immediately
in the same Files, Staged and Inspector views used by the person. `staging.get`
reads the current shared differences within your grant; `files.read` reads saved
disk values. Artwork is metadata-only in staging replies.

A draft handle names an explicit live set of file/field keys. Its `scope_digest`
(`digest` is a compatibility alias) authorizes that scope, not an old copy of its
values. At Apply admission, the native service seals the CURRENT shared values
in that scope and records their separate `accepted_digest`. A human’s newer title
therefore replaces the agent’s older title; removing that field means there is
nothing to write for it. Unrelated fields and files are outside the handle.
A new idempotency key applies the current values; retrying an old key returns
its original receipt even if the staging area has changed again.

Human field edits stay protected while staged. Agents can continue changing
other fields on the same file. A conflicting agent request reports protected
fields; competing agents cannot silently replace one another’s differing intent.
Discarding a handle removes only its still-owned contribution. Disconnecting or
revoking an agent leaves accepted staged edits visible for the person to manage.
Human Apply writes the current selected shared stage, and successful writes remove
only matching applied values, preserving newer edits and excluded rows.

Direct Write uses the same native validation and journal protections without
adding edits to the shared stage.

When a reply is lost, inspect operation status before retrying and reuse the
original idempotency key for the same intent. The connector never automatically
replays a write. Closing an agent connection does not imply its accepted job
has stopped. Use the cancellation tool or the desktop's Stop control; rollback
and recovery states are reported by the native operation owner.

MCP request cancellation and operation cancellation are distinct. A cancelled
request may stop waiting for a result while accepted native work continues.
Do not interpret a timeout, EOF or disconnected client as a successful rollback.

Metadata queries are read-only and use separate per-client cancellation lanes.
The connector forwards request cancellation to that query's exact generation;
it cannot cancel another client's query or the desktop's search. Clients can
also use `queries.cancel` for a chosen lane. Cancellation is advisory: a query
that already completed may have returned a result. Write operations require the
operation-cancellation tool and a checked terminal state.

Reusable masks, transformation groups and filter presets have native tools;
use their revision checks when saving. A late agent save must not replace a
person's newer settings document.

## Typed intents and composition

Each preview receives a top-level `selection` handle or `paths` array. An edit
is `{ "path": "/approved/track.mp3", "field": "title", "value": "Title" }`;
`value: null` removes that field. For one value across a selection, use
`values: { "artist": "Artist" }`. A nested `{path, values}` is not an edit.
Keep the explicit selection when composing an existing `draft`:

```json
{
  "paths": ["/approved/track.mp3"],
  "draft": "draft_handle_from_previous_preview",
  "edits": [
    {"path": "/approved/track.mp3", "field": "title", "value": "Corrected title"}
  ]
}
```

Explicit later desired values replace earlier intent, including when the later
value equals the saved value and therefore needs no disk change. A preview's
`files` count means files with proposed changes; a zero-change preview does not
mean that no files were evaluated. Staging never changes saved metadata.

For `presets.put`, read the current revision with `presets.list`, then use a
kind-specific `preset`: filters wrap the versioned model as `{ "query": ... }`;
masks use `{ "mask": "%artist% - %title%" }`; transforms use
`{ "scope": "tags", "rules": [...] }`. The name is supplied separately.
The revision covers the shared settings document, across all preset kinds.
After a conflict, reread and reapply the intended change; do not replace that
document with a cached copy.

A featuring-credit replacement can use:

```json
{
  "kind": "replace",
  "from": "Feat.",
  "to": "feat.",
  "regex": false,
  "case_sensitive": false,
  "whole_word": false
}
```

The native whole-word option uses word boundaries; punctuation such as the
period in `Feat.` makes that option inappropriate for this literal cleanup.
Preview the rule before committing, and preserve meaningful line breaks when
tidying whitespace.

Artwork previews include old/new roles and counts. The default `native` mode
retains the desktop's existing policy, including unrelated back/Other images.
Choose `replace_front` to preserve non-front roles, `append` to add an image,
or explicitly choose `replace_all` after inspecting the collection when the
whole collection should be replaced. A sole image tagged Other is not silently
treated as a front cover. Use owned artwork handles and inspect the resulting
diff; image bytes are fetched separately rather than embedded in every preview.

## Diagnose a connection

- **Unavailable:** open the intended desktop profile and enable external agents.
  Check that client and desktop use the same profile path.
- **Authentication refused:** pair that client, select the matching credential
  file, and check expiry/revocation. Reconnecting never expands a grant.
- **Not private:** the credential or connection files are readable by another
  local identity, are symlinks, or have an unexpected owner. Re-export the
  credential to a private location. Unix files require owner-only permissions;
  Windows uses a protected owner/LocalSystem DACL.
- **Busy:** bounded native work slots are occupied. Poll the operation you
  already started; avoid repeatedly submitting the same write. Control requests
  retain capacity separate from ordinary work.
- **Response too large:** ask for a smaller projected page or binary chunk.

The connector writes only MCP JSON-RPC messages to stdout. Diagnostics go to
stderr and do not contain credentials or request bodies. It never exposes the
development webview/JavaScript bridge as a production tool.

See [transport design](transport.md) for framing, ownership and compatibility.
The [checkpoint validation](validation.md) records exercised workflows and the
remaining platform-testing boundaries.

Client configuration references: [Codex MCP](https://developers.openai.com/codex/mcp),
[OpenCode MCP](https://opencode.ai/docs/mcp-servers/).

## Native field recipes

`draft.transform` accepts the backend `format`, `remove_fields` and `keep_fields`
rule kinds in addition to ordinary text transformations. These read the current
shared working values, retain protected human edits and produce the existing
shared stage. The same groups can be saved through `presets.put`.

See [Native field recipes](../guide/field-recipes.md) for the JSON contract,
artwork policy, custom-field names and limits. These operations edit text tags;
use the existing rename tools for paths and filenames.
