# DeepSeek Rust reliability and performance review

Requested model: OpenCode Go / DeepSeek V4.1 Flash 1017.
Starting checkpoint: `0554022c`, including code checkpoint `b657f65d`.
Scope: review, implement justified fixes/optimizations, test edge cases and
iterate. Rust only; retain stage/review/Apply/Undo and recursive root scope.

## Parallel ownership

1. Player and queues: player crate, desktop blocking queue and playback adapters.
2. Search and staging: query/read caches and shared-stage projection/concurrency.
3. Media integrity and outputs: artwork, export/codec limits, Apply/Undo/recovery
   and service plan/output accounting.

The parent owns integration, this ranking ledger, shared documentation and the
final combined checks. Worker evidence lives in separately named report files.
Workers must not restart the development app or touch user media/runtime state.

## Ranking criteria

- Critical: confirmed data loss, corruption or broken recovery boundaries.
- High: crashes, incorrect saved/staged results, races, deadlock or major stalls.
- Medium: boundedness, cancellation and demonstrated scaling/allocation costs.
- Low: smaller evidenced improvements; style preferences are not findings.

Each finding must include its trigger, location, evidence/confidence, plain-English
impact, chosen fix, edge cases, tests and unresolved limitations. Agent work is
ranked by correctness/evidence, useful impact, coverage and regression risk;
finding count alone does not determine rank. No findings have been ranked yet.

Completion gate: parent-reviewed implementation, relevant tests and combined Rust
formatting/Clippy/tests, with limitations stated and backend/UI roadmap gates
retained. Reusable changes target curated upstream `main`; incompatible fork
service/storage changes require separate curation. No publication is implied.


## First returned findings: player pass (provisional parent ranking)

1. **High, unresolved hypothesis:** rodio seek waits indefinitely if a device
   stops pulling. Source-backed, not hardware-reproduced; follow-up requested.
   A timeout that leaks worker threads or weakens ordering is not an acceptable fix.
2. **Medium correctness:** seek can update the clock after audio has played out.
   Returned guard is under review, including the check/use race as audio ends.
3. **Medium robustness/performance:** bound the MP4 box walk before decoding.
   Returned deterministic regression is useful; guard/fallback preservation needs
   parent validation before acceptance.
4. **Medium efficiency:** reduce empty-player polling; command wake responsiveness
   must remain tested. The implementation still has a 60-second safety timeout,
   so it reduces periodic wakeups rather than eliminating them entirely.
5. **Low contention:** avoid storing identical paused snapshots. Parent inspection
   found path allocation still occurs before comparison, so the claimed removal
   of paused allocations is not accepted. Follow-up implementation requested.
6. **Low input correctness:** clamp finite desktop volume before narrowing f64.
7. **Test coverage:** hardware-free playback seam and concurrency tests provide
   useful regression coverage; no claim of real-device validation.

Player agent assessment: promising implementation and meaningful test coverage,
but conditional acceptance until report accuracy and the seek edge cases are
resolved. No relative agent-work ranking is assigned until other passes return.
All worker edits remain uncommitted and subject to parent verification.


## Second returned findings: media integrity (provisional parent ranking)

- **Medium defensive hardening, corrected after parent reproduction:** symlink
  cycles caused repeated traversal, but Linux stops resolution at ELOOP. A
  disposable reproduction of the original scan shape returned false after 41
  directory calls; no stack-exhaustion claim is accepted for that platform.
  Worker link refusal/depth cap remain useful conservative protections.
- **High, parent-discovered follow-on:** bulk deletion after a successful scan
  can remove real files created meanwhile. Parent replaced bulk deletion with
  conservative leaf cleanup and empty-directory removal, and added a deterministic
  scan/change/cleanup regression covering nested and direct new files.
- **Medium, measured component work:** revision/copy/pruning indexes avoid
  repeated Undo/rollback path scans; export snapshots avoid a second metadata pass.
- **Medium, memory admission:** retained draft scope/identity accounting and
  actual owned analysis path storage. Parent tightened path admission before
  aggregate allocation, including vector/string capacities.
- **Medium, parent-discovered regression:** skipping native TrackFile conversion
  for playlists also skipped unreadable-file refusal. Parent restores the refusal
  before format dispatch and extends the fixture to all seven format aliases.
- **Unresolved policy issue:** interrupted recovery may require external value
  restoration under strict Undo. This is deliberate fail-closed behavior, not
  permission to silently accept ambiguous new values. A new confirmed-recovery
  mode would be a separately specified feature; it is not introduced here.

The media area currently has the highest confirmed reliability impact after
parent-discovered file-deletion corrections; this is not an endorsement of the
worker's overstated symlink crash claim. Acceptance remains conditional on the parent corrections
and combined gate. The search/staging pass has not yet returned; no final
three-agent ranking or integrated completion is claimed.


Parent verification of the media corrections: filtered pruning and output tests
passed, core all-target/all-feature Clippy and service lib/all-feature Clippy
passed. This is not the combined gate. The Linux symlink probe was disposable
and removed automatically. The media worker's raw component timings remain
microbenchmarks; no end-to-end speedup is inferred.


## Third returned findings: search/staging (provisional parent assessment)

This pass currently leads on measured performance evidence: component timings,
result checksums, byte-equivalence/digest compatibility and regression tests are
reported. The cache gain deliberately raises the per-index facet ceiling from
32 to 96 MiB; it does not reserve that amount up front or impose a process-wide
cap. Parent accepts the trade provisionally for the measured working set,
without extrapolating a whole-app speedup or a verified 200k-row result.

Parent review removed another avoidable derived-reference allocation before
resync rejection: borrowed references now serialize directly from claims, and
the >4096-file fast path does not collect them. Existing byte-equivalence tests
now also include derived artwork and ordinary non-derived claims. The worker's
VmHWM numbers indicate high-water changes, not proof of zero allocations.

Provisional work-quality order while player iteration remains open:
1. Search/staging: strongest measured CPU/copy improvements and compatibility tests.
2. Media integrity: useful indexes/accounting/export optimization; parent corrected
   the symlink severity claim and fixed deletion-race/export-refusal gaps.
3. Player: useful test seam and bounded scanning; allocation/seek claims are under
   a second implementation pass and cannot yet receive final acceptance.

Finding priority remains separate from agent rank: the parent-confirmed cleanup
file-deletion race has the highest confirmed data-integrity impact so far; the
stalled-device seek risk remains high but not yet reproduced/fixed.


Parent performance repeat: rebuilt the release query_bench example from the
reviewed source and replayed the 100k-row synthetic fixture. Warm suggestions
measured 6.30 ms median / 6.80 ms p95. All nine query match/checksums and the
complete suggestion list match the worker's pre-change baseline. The pre-change
500.90 ms measurement remains worker-generated; parent independently repeated
the final side, not the baseline build. Compact before/parent-after evidence and
the synthetic fixture generator are under docs/performance/evidence. Fixture
memory/files are disposable and no user library was opened.


## Final acceptance and work ranking

1. **Search/staging:** strongest measured performance work, byte/digest equality
   tests and parent-repeated final benchmark with baseline result agreement.
   Accepted with the explicit per-index 96 MiB retention trade and the parent's
   borrowed-derived-reference correction. Service changes still need fork-aware
   upstream curation despite wire compatibility.
2. **Media integrity/outputs:** accepted indexes, retained accounting and removal
   of duplicate reads, after parent corrections for newly created real files,
   unreadable playlist refusal and bounded path allocation. The self-cycle crash
   claim is downgraded on Linux, where ELOOP terminated the original scan shape.
3. **Player/queues:** meaningful hardware-free coverage and accepted idle/status/
   seek/scan fixes, but required a second model pass and parent boundary/security
   corrections. The parent fixed the pre-seek queue-advance interval and made
   MP4 inspection-budget exhaustion fail closed. The dangerous padded-table
   fallback from the first pass is not part of the accepted code.

Ranking is based on verified useful impact, evidence accuracy, compatibility and
regression burden, not finding count. Parent corrections are attributed to the
parent rather than credited as discoveries by the delegated agents.

### Finding priorities and remaining work

- High and fixed: cleanup must not bulk-delete real contents created after a
  scan; MP4 inspection limits must not bypass dangerous sample-table guards.
- High and OPEN: rodio seek can block the player actor if the output stops
  consuming. Deterministic stalled-sink tests demonstrate blocked Stop/order;
  physical device-loss triggering remains untested. A track-bound acknowledgement
  protocol or supported nonblocking dependency API needs its own implementation
  and device/gapless validation. No leaking timeout worker workaround was added.
- Recovery policy OPEN: interrupted journal entries without observed outcomes
  remain fail-closed. A separate confirmed-recovery feature is required before
  allowing ambiguous old/new values; this review does not loosen ordinary Undo.
- Medium and fixed: repeated Undo scans, redundant export reads, stage temporary
  copies, warm facet thrashing, repeated field-key scans, retained-memory gaps,
  played-out/advanced-source seek handling and paused publication allocations.
- Deferred: global derived-memory admission, cold facet representation, full row
  text duplication, artwork orientation/ICC, line-break policy for line-oriented
  exports, device lifecycle and retry rate limiting. No full parity or
  certification claim is made.

### Final combined gate (18 November 2026)

`cargo test --workspace --all-features`: **1,188 passed, zero failed, 19 ignored**
across 46 test binaries/doc-test groups. `cargo clippy --workspace --all-targets
--all-features -- -D warnings`, `cargo fmt --all -- --check`, `git diff --check`,
`npm run check` (zero diagnostics), `npm run test:unit` (205 passed) and the actual
production frontend build passed. No desktop embed override was used for the
parent combined gate. Real-device and Windows/macOS validation and hosted CI are
not claimed. The existing dev process, Vite attachment and user media were not
changed. Focused local commits follow; no push, PR or merge is implied.


## Accepted local checkpoints

- `f959bb18`: hot query facets, field-name memo and streaming staged accounting.
- `c8373a04`: conservative pruning, indexed Undo, shared export reads and retained
  plan/analysis path accounting, including the parent's regressions/corrections.
- `5d412c62`: allocation-free unchanged status, bounded idle polling, playback
  test seam and seek/MP4 safety corrections, plus queue tests and volume adapter.

The running dev session was not restarted or upgraded by this review. No user
media was written. Open findings above remain open; these commits are scoped
reviewed improvements, not full application certification. The README adds only
backend sub-capabilities; all 207 top-level feature entries retain their states.
