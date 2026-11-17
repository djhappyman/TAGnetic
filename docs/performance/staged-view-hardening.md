# Staged view and plan accounting hardening

Staged agent views perform one filesystem scope decision per candidate path,
outside the shared state lock. Field visibility is checked first. Reacquiring
the lock verifies the stage revision and the complete current grant, including
expiry and revocation, before projecting any data. A concurrent change returns
a conflict instead of leaking an outdated view. Root grants remain recursive;
authoritative write validation is unchanged.

Plan projections clone only retained fields, artwork, blocks and rename details.
Shared artwork asset references still remap into the selected asset pool. JSON
plan size and digest operations now stream through a capped counting/hash sink,
without allocating a serialized byte buffer or an owned JSON copy of the plan.
All authorization inputs remain in digests. Newly generated private draft and
accepted-plan digests use native struct serialization; these are opaque server
outputs. Live scope and direct-intent digest serialization remains unchanged.
Existing stored operation digests and Undo data are not rewritten.

Verification: 138 service tests passed, including deterministic checks that
filesystem admission runs without the state lock, once per path; stage/grant
changes fail closed; root scope includes nested files; invisible fields require
no path check. Accounting tests verify wire byte counts, SHA-256 compatibility
for Value serialization, escaping/Unicode, exact limits and early termination.
Existing scale, shared artwork, Apply, recovery and Undo tests passed. Clippy
with all targets/features and workspace formatting passed. This establishes
behavior and removed allocations, not a measured wall-clock speedup or RSS cap.

Route: fork service implementation; reusable ideas require curation against
upstream `main`. No PR, installation or hosted platform verification claimed.
