# Service snapshots: native row sharing and policy validity

Locally implemented and independently rechecked in the bounded service snapshot scope. Reusable changes are candidates
for curated upstream `tagrex/tagrex:main`. Fork storage and the upstream inline
journal remain separate compatibility concerns. No UI or live profile changed.

A public `files.snapshot` regression retained a second 23,281,056-byte native
index while the process retained budget was already full. Another public-service
regression changed the global multi-value separator through another App session:
a new service snapshot reused old interpretation because the cached App generation
was still accepted.

The service now caches DTO rows together with their admitted native QueryIndex.
Client snapshots fork immutable native row Arcs with independent cancellation,
query and derived-cache state. Full-scope clients share saved DTOs too. Scoped
clients retain an admitted position vector over the saved DTOs instead of cloning
their metadata. Native client row vectors also reserve their own shared-budget
retention before allocation. One ordered selection drives DTOs and native rows,
so a second path evaluation cannot misalign result indices. Source generations,
policy validity, row paths/counts and the opened root are checked before a handle
is inserted. Replaced cached owners are released after the App/state locks.

`App::query_snapshot_id` excludes stale policy generations. Production duplicate
analysis and empty preset indexes also use fallible admission. Synthetic service
scale fixtures keep their explicit caller-managed model constructor; the model
and native sharing are measured, without claiming to measure native IO/admission.
Production snapshot construction always shares the App-admitted base.

## Verification

```text
cargo test -p tagrex-commands --test query_base_memory_recheck --locked
4 passed; 0 failed
cargo test -p tagrex-service --test snapshot_resource_policy_recheck --locked
3 passed; 0 failed
```

The command probes cover admitted owner lifetime, pressure refusal preserving an
older snapshot, oversized/busy input, and independent fork validity with duplicate,
unsorted and out-of-range selections refused. Public service probes use paired
read-only principals and disposable FLAC fixtures in isolated processes. They
measure actual retained allocation separately from resource charges and retain
the original policy assertion. A third canary reproduced 585,520 bytes of unadmitted metadata cloning in a
scoped client. It now refuses under a full budget, succeeds after pressure
releases, keeps page/query positions aligned and releases the view charge with
its handle. The combined full service gate passed 150 tests plus these three public-service
regressions after the vector admission change; commands/service all-target, all-feature Clippy with denied warnings passed. A
separate [seven-test review](backend-service-snapshot-independent-review.md) passes
pressure, scope/query alignment, concurrent clients and policy generations. It
found a revocation race leaving an unreachable snapshot; authorization now
rechecks under the same state lock used for insertion. Its unchanged public race
canary passes. Final combined/platform delivery gates remain pending.

This does not account for every original DTO, JSON response or IPC
allocation, prove RSS limits, or guarantee a fixed 100,000-file library under the
default budget. Native platform and full roadmap completion remain open.
