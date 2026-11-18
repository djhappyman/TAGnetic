# Service handle and history admission hardening

Implementation is ready for a separate adversarial review. Local Linux gates
pass; this is not approval of the whole service, native platform matrix or
backend roadmap. No live profile, media, UI, commit or PR changed. Reusable fixes
are candidates for curated upstream `tagrex/tagrex:main`; fork storage differences
and upstream inline journal images remain separate compatibility gates.

## Reproduced findings

**P1: a revoked history request could still mutate files.** The public Undo
request passed its initial grant, acquired the mutation lane, then waited for the
App mutex. Revocation completed during that wait. After the App became available,
Undo changed the media and journal; only the outer result check returned
`permission_denied`. A normal retained test reproduces this with exact file and
history comparisons. It uses an observational mutex barrier, without a production
fault hook. The final authorization check now runs after App acquisition under
the existing history state lock, before any history mutation. Revoked Redo and
disabled Undo controls check unchanged bytes, history, Redo state and durable
operation receipts too. This does not promise interrupting a history operation
already admitted and running under that lock.

**P2: unreturned service owners survive refusal.** Public calls reproduced:

- A provider result received after revocation or disable retained release/image
  receipts. A later client could no longer admit its otherwise valid full batch.
- Artwork received after revocation retained an asset; a later client's 256th
  small upload failed against the exhausted handle count.
- Provider image quota refusal inserted part of a batch before returning an
  error, leaving image receipts without returned handles.
- Oversized provider responses and query pages hit the outer response cap after
  retaining handles. Invalid query projection input also retained a selection.
- Failed initial `operations.json` persistence left an in-memory queued operation
  with no worker. A retry could reuse that operation and never start.

The original failure assertions remain represented in the retained tests; these
are actual Service calls and fixture backend completions, not synthetic registry
insertions. Failures before corrections are recorded in the task-owned external
`tmp/backend-*-red.log` files.

## Resulting behavior

Final handle insertion and query-lane registration validate active authority under
the same state lock as the insertion. Sealing checks stage authority before
publishing any shared-stage contribution; write acceptance and execution also
recheck their grant at the relevant state boundary. No new state lock is held
across provider calls, image decoding, response construction or App-lock waits.

Provider release/image receipts are generated locally, including random IDs and
response construction, before touching registries. The complete aggregate image
count is admitted atomically, including the fallback cover path. Failed batches
preserve earlier returned handles. Query projection is validated before native
work; the final query page and provider response pass a streaming 1 MiB wire-size
check before their handles are committed. The counting sink does not allocate a
second serialized JSON buffer.

If initial operation persistence fails, its unpublished in-memory queue entry is
removed before returning. No worker or media effect has started. Uncertain
on-disk evidence is left intact; reopening retains the existing conservative
recovery behavior. The same-key retry can create a real operation after the
storage fault is repaired, while earlier receipts and the prepared draft survive.

## Stress and verification

```text
cargo test -p tagrex-service --all-features --locked
153 unit tests + 11 lifecycle integration tests + 7 snapshot review tests
+ 3 snapshot policy/resource tests = 174 passed; 0 failed; 0 ignored
cargo clippy -p tagrex-service --all-targets --all-features --no-deps --locked -- -D warnings
passed
rustfmt --edition 2021 --config skip_children=true --check <owned Rust paths>
passed
git diff --check -- <owned production paths>
passed
```

The lifecycle tests include 12 rounds of eight simultaneous 40-image batches.
Exactly six complete batches fit the 256-receipt limit; refused batches leave no
partial owners. After releasing returned handles, a full 256-image batch succeeds
on every round. Additional conditions include actual grant expiry while a backend
is blocked, revocation, disable/re-enable, fallback cover admission at capacity,
malformed projection, large Unicode metadata, a filesystem persistence failure,
and a successful real write after repair. All filesystem changes use disposable
WAV/media/profile fixtures that clean themselves on success and failure.

Reviewed implementation SHA-256 at this local checkpoint:

- `service/src/lib.rs`: `adba73d20a3cbef795f97cc58c57b8dbbe3937522b32adbf5e82db5d2389bbe7`
- `service/src/domains.rs`: `c43392248ba25f9e6cf093972bc61f6a474a6963560ab3b9d1280777bb356d94`
- `service/src/plans.rs`: `c75ac7f67467f3d1106e10ad33c1d51e1ca899abd4ec47ab279dc07641cd54f4`
- `service/src/json_accounting.rs`: `015f1fce06428342f0faaa1d13288bc12247c5b14275e4a73e1b82b42d1a250e`
- `service/src/handle_lifecycle_tests.rs`: `2c34d987875152f09c07dae1e1aaeeeb5b6097fd092c0d55f014e5fec01842f8`
- `service/tests/handle_lifecycle.rs`: `9f2628b5355d02abfffef75fd5f23bbbd38cae3e27bbb91b7e4c501254a234b5`

Code discovery used Verify-tier `tagrex-backend-readiness`, generation
`2026-11-18T12:48:55Z`, rooted in this checkout. The bounded structural search
returned all 43 matching service symbols. Coverage marked lib/domains/outputs
changed; plans/tests matched recorded metadata with a best-effort caveat. Current
source was read for every material creation, authorization and mutation path;
new test/helper files are outside that graph generation.

## Remaining gates

A fresh reviewer must challenge the admission boundaries, failed persistence
retry semantics and actual mutation refusal. Native Windows/macOS behavior,
expiry during already-running history I/O, analysis registry lifecycle in
`service/outputs.rs`, all DTO/JSON allocation ownership, RSS and provider transport
limits remain outside this checkpoint. The response check limits the wire result;
it does not claim that every provider-owned input or intermediate JSON allocation
is covered by the shared resource coordinator. No physical-device, kernel power
loss or whole-backend readiness claim follows from this gate.
