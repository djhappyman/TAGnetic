# Local MCP transport contract

`tagrex-mcp` pins the official Rust SDK to `rmcp = 3.10.0` (declared MSRV 1.88;
the application workspace requires Rust 1.90). The SDK implements the public
MCP wire protocol, including the 2025-11-25 initialization lifecycle and the
2026-07-28 per-request metadata/discovery lifecycle. The adapter advertises
tools only; operation polling is available without optional Tasks, resources,
subscriptions or elicitation support.

## Public connector

The installed executable dispatches `--mcp-stdio` before Tauri, window creation
or single-instance initialization. It reads a private credential file,
authenticates against the desktop and obtains the executing service's tool
catalogue before accepting MCP traffic. Its only persistent configuration is
the client-supplied profile and credential-file paths.

SDK stdio frames are UTF-8 newline-delimited JSON-RPC. Incoming lines are capped
at 1 MiB using the SDK's bounded reader. Tool output is both JSON text content
and structured content. Domain permission, stale-handle, validation and native
operation errors are caller-visible tool errors, rather than opaque protocol
errors. The adapter does not log tag values or credentials.

## Private desktop RPC

The connector-to-desktop channel is an internal authenticated RPC protocol,
not an additional public MCP binding. No third-party client should implement
it; use the installed stdio connector.

Discovery lives at `<profile>/mcp/endpoint.json` and contains only:

```json
{"protocol":1,"transport":"unix","address":"<private local socket>","instance":"<desktop instance nonce>"}
```

The Windows transport is `windows-pipe`. On Unix discovery stays in the profile,
while the socket uses the short private `/tmp/tagnetic-mcp-<uid>` directory.
This avoids the Unix socket path limit even with deeply nested profile folders.
Its filename binds a SHA-256-derived canonical profile identity to the random
desktop instance. Both directories are mode 0700; the socket and discovery are
0600, and current-user ownership is verified. Before transmitting a credential,
the connector verifies the exact expected filename, directory and socket type,
owner and permissions. An arbitrary local address in discovery is refused.
Symlink discovery/credential files are refused; opened Unix files are checked
again after `O_NOFOLLOW` open. On Windows files/directories have a protected
DACL allowing only their actual user owner and LocalSystem. Named pipes use an
explicit current-user/LocalSystem DACL and reject remote clients.

An exclusive profile lease prevents two hosts. Socket/pipe addresses contain a
fresh instance nonce. Shutdown deletes discovery only if it still names the
same instance; it never blindly unlinks another host's endpoint. Stale discovery
is replaced only while holding the exclusive host lease.

Each private frame is a big-endian 32-bit byte length followed by UTF-8 JSON.
Zero lengths, malformed JSON and frames over 1 MiB are refused before an
unbounded allocation. The first frame is a versioned hello containing the
instance and credential. Authentication is enforced by the service, independently
of the MCP client's self-reported name. Authentication and request admission have
five-second deadlines. Credentials do not appear in discovery or tool results.

After the authenticated acknowledgement, the connection carries one catalogue
or tool request and its result. Every new call authenticates again, so a revoked
client cannot reuse an old attachment. Native calls revalidate their principal
epoch as well. One request per connection lets status/cancel calls bypass another
request's long-running native work without adding a second session owner.

There are at most 64 simultaneous admitted connections and 14 ordinary native
workers. New ordinary work is refused when its budget is full. Operation status,
cancellation and session status bypass ordinary worker permits. These limits
bound blocking-pool and connection growth; they are resource limits, not a
measured throughput claim.

The connector never automatically retries a tool request. Accepted operations
remain owned by the service even when the socket closes. Native operation IDs,
sealed plan identities and idempotency receipts handle uncertain outcomes.
Transport EOF is not a cancellation or rollback acknowledgement.

## Verification boundaries

Unit/integration coverage exercises bounded frames, malformed input, private
credentials, invalid authentication, revocation, exclusive ownership, guarded
cleanup, retained control capacity, and official SDK clients using both protocol
eras through the private host. The root integration suite additionally launches
the installed executable and actual Codex/OpenCode clients. Cross-platform
compilation is distinct from running Windows/macOS permission and IPC tests;
report those separately.

Upstream route: standalone service/transport support belongs in
`tagrex/tagrex:main`; desktop consent and work-tray interface contributions belong
in `tagrex/tagrex:next`. Branding, release configuration and fork-only storage
compatibility are not transport changes.

Primary references: [official Rust SDK](https://github.com/modelcontextprotocol/rust-sdk),
[RMCP 3.10.0](https://docs.rs/rmcp/3.10.0/rmcp/),
[MCP stdio](https://modelcontextprotocol.io/specification/2026-07-28/basic/transports/stdio).
