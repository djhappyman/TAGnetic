# Service authority and handle publication

This checkpoint rechecks service authority at the point work is admitted, and
commits private result handles only after their response and aggregate quotas
have passed. It uses the existing Stage, Apply, Undo and operation receipt flow.

A public Undo request could pass its initial grant, wait for the App mutex, be
revoked, then change media and history before returning permission denied.
History now checks active history/write permission after acquiring the App and
under its existing state lock. Queued writes, staged contributions and handle
insertion also check active permission under the lock that commits their state.
An already admitted history operation remains serialized; this change does not
promise preempting an operation already running under that lock.

Provider image batches are prepared locally and admitted atomically. Provider
and query responses pass a streaming wire-size check before their handles are
stored. Malformed projections, oversized responses, quota refusal and revoked,
disabled or expired requests preserve earlier returned handles. An initial
operation-persistence failure removes the unpublished in-memory queue entry,
so a repaired same-key retry can start. Uncertain on-disk evidence remains intact.

## Verification

The focused slice was curated onto fork main `0beb5bb6`, separately from ongoing
backend work, then checked again. Local Linux evidence:

```text
cargo test -p tagrex-service --all-features --locked
152 unit + 11 lifecycle + 4 independent integration = 167 passed; 0 failed
cargo clippy -p tagrex-service --all-targets --all-features --no-deps --locked -- -D warnings
passed
cargo fmt --all --check
passed
npm run build
passed
cargo clippy --workspace --all-targets --locked -- -D warnings
passed
cargo test --workspace --locked
1208 passed; 0 failed; 19 existing optional tests ignored
dbus-run-session -- cargo test -p tagrex --locked mpris -- --ignored
2 passed; 0 failed
```

The 19 added tests include independent simultaneous Undo/Redo waiters at actual
grant expiry; queued requests across revocation, disable, expiry and cancellation;
heterogeneous provider batch contention; fallback image quota; control-escaped
wire expansion; malformed projection; and persistence failure followed by eight
same-key retries. Fixtures compare exact media, history, Redo state and durable
operation receipts on refusal. All writes use disposable fixture media/profiles.
Separate implementation, adversarial and clean-base curation reviews found no
remaining blocker within this scope. Hosted exact-head Linux/Windows/macOS and
audit checks remain the merge gate; local results do not verify native platforms.

Broader analysis-job lifecycle, complete DTO/JSON/RSS accounting, long-running
history cancellation, exact metadata writing and the remaining backend roadmap
are separate work. No live profile, runtime, library or Undo history was used as
a fixture.

## Upstream classification

Reusable service admission fixes are proposed for curated `tagrex/tagrex:main`,
waiting for a compatible service foundation. On 18 November 2026, upstream main's
accessible workspace manifest lists no service crate, and its
`crates/service/Cargo.toml` is absent. A standalone cherry-pick cannot build there;
this checkpoint does not submit the fork's whole service as an incidental fix.
Preserve upstream inline journal images and the waveform interface when that
foundation is prepared. Fork branding, storage and interface remain separate.
