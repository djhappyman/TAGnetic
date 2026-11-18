# FLAC command compatibility checkpoint

Publication checkout: `/mnt/Games/Development/tagrex/native-flac-publication`,
curated onto `c42a6cea`. This report concerns the required App-policy/admission
corrections for the narrow protected Linux FLAC text route. It is not completion
of the parked backend roadmap. No commits, PRs, runtime changes or collection
writes were performed by this worker.

## Confirmed failures and corrections

1. **P2 — peer settings invalidated native writes.** The original default-parallel
   commands run failed five existing App tests with policy/context/staleness
   errors; isolated Redo and Quick Write passed. App settings changed shared
   process globals during peers' reads/writes. `ModelPolicy` now owns one atomic
   immutable settings snapshot and unique generation per session. All 80
   synchronous public App instance entries and the private cached-read boundary
   enter its thread scope. Nested same-owner calls reuse the pinned generation
   without allocating another frame; different owners restore their predecessor
   on return/unwind, including non-LIFO guard drops. No mutex is held across IO
   or user callbacks. Explicit standalone model setters retain their own global
   defaults and invalidate a differing active same-thread operation.
2. **P2 — policy invalidation blocked rollback of an owned prefix.** Independent
   probes changed the separator or owned policy and cancelled after the first
   write. A private process-local RecoveryPolicy now captures the admitted
   semantic snapshot before native effects and seals it only for automatic
   Apply rollback / failed-Undo rollback. This restores the known-owned prefix
   while retaining source/root/receipt/conflict checks. Current-owner invalidation
   remains sticky afterward. Ordinary reopened Undo still refuses incompatible
   interpretation until compatible settings return. Public Executor Apply/Undo
   refuse a recovery ancestor before any progress/journal effects, including
   peer App and legacy/Vec adapters; the old snapshot cannot authorize new user
   operations through a private journal callback.
3. **P2 — invalidated reads could poison display/query caches.** ReadCache keys
   snapshots by valid policy generation and never reuses/admits MAX-invalid
   scoped data. Query snapshots carry their policy generation; invalid-scope
   indexes are invalid immediately, including retained Arcs handed to workers.
   Returning from an outer scope, errors and settings ABA force correct rebuilds.
4. **P2 — valid parallel operations hit unqueued nested admission.** Temporary
   phase diagnostics located actual failures at Store's initial working-lease
   acquisition, before directory/editor/ack effects. A private IO-phase gate now
   bounds synchronous native preparation/publication/retirement using the
   existing worker and byte ceilings: default capacity 2 derives from two
   nested worker slots plus the 128 MiB native writer and Store working buffers.
   All actual component leases/quotas remain unchanged. The gate is outside
   public progress callbacks, observes cancellation and refuses same-thread
   reentry before public callbacks or journal effects. Only the private held-file
   prepare adapter waits for typed initial Busy admission before any artifacts;
   existing public Store primitives remain nonblocking. No mutation or uncertain
   acknowledgement is blindly retried. Temporary diagnostics were removed.

The publication's query workers consume immutable rows; the inspected App/query/
read-cache paths do not launch metadata-reading workers. An App moved to a worker
enters its own scope at the synchronous entry. The public ModelPolicy API requires
explicit `enter()` on any future metadata worker; TLS does not inherit across
threads, and PolicyScope is not Send. It is a synchronous thread scope, not an
async task context. No broader worker/cache redesign was added.

## Evidence

- Original isolated Redo / staged Quick Write passed, while the original parallel
  commands gate failed 5 existing tests. No original assertions were changed.
- After policy isolation, a fresh parallel gate had no policy/context failures;
  remaining typed Busy failures produced only the confirmed initial Store phase
  diagnostics. The earlier interleaved diagnostic log is not used as a gate.
- After bounded IO admission: **356 commands tests passed, 7 existing ignored**,
  at default parallelism. This includes the original five workflows and three
  meaningful public App tests for reentrant peer settings, custom Quick Write
  rebasing, external Undo conflicts, same-thread policy refusal and unwind/retry.
- Two scope tests passed: nested/non-LIFO/unwinding restoration and explicit worker
  propagation with peer versus same-owner changes.
- The real controlled pressure probe passed: **8 Apps × 3 rounds of
  Apply → Undo → Redo → Undo**, divergent separators/settings, bounded callback
  rendezvous, native audio/private-field preservation and resource-owner return
  to baseline. This is functional evidence, not a throughput measurement.
- The author's corrected independent policy gate passed **11 tests**, including
  the observed-red prefix restoration, display/query caches, ABA, incompatible
  reopened semantics and external-owned-field preservation. The reviewer then
  accepted **17 independent tests**, including retained-query
  validity, actual sealed-callback rejection and scope unwind.
- Admission reviewer independently accepted **54 passed, 3 existing ignored**
  covering pressure, cancellation, public Store refusal, zero-callback sink
  Apply/Undo reentry and panic/retry on these frozen production hashes.
- Final formatting passed. Final all-target/all-feature Clippy for core, commands
  and service is recorded separately; the only intermediate lint failures were
  three test parity expressions, corrected in the new pressure test.
- Root's final clean-publication workspace gate passed **1598 tests, zero
  failures, 34 ignored**. Workspace all-target/all-feature Clippy and formatting
  passed; the private-DBus probes passed two tests. Policy and admission reviews
  both accepted their frozen scopes. Do not replace these clean-publication
  results with the parked programme's 869-test result. The original unchanged
  metadata_preservation_recheck remains a
  separately verified Linux audit in that parked checkout; publication has the
  admitted-route preservation tests. Portable legacy preservation remains open.

Logs under `/mnt/Games/Development/tagrex/tmp/`:
`flac-policy-commands-fresh.log`, `flac-policy-media-admission-diagnosis.log`,
`flac-policy-admitted-commands.log`, `flac-policy-scope-tests.log`,
`flac-policy-app-focused.log`, `flac-policy-review-corrections.log`,
`flac-policy-eight-apps.log`, `flac-policy-final-clippy.log`,
`flac-policy-final-fmt.log`. Root's combined evidence is
`flac-final-workspace-tests.log`; reviewer logs/reports are independent evidence.

## Scope and route

Only this authorized FLAC publication slice is being completed; broader work
stays paused. Linux native capabilities remain required. Other formats, mixed
cover/block/transfer plans and typed fields retain their legacy behavior; no UI
exposure or general exact Stage DTO migration was added. Reusable policy,
coordinator and preservation fixes are eligible for curated `tagrex/tagrex:main`.
Preserve upstream inline images and stable waveform interfaces; do not import
unrelated parked programme changes or fork-only pooled-storage representation.

## Frozen production hashes

| File | SHA-256 |
| --- | --- |
| `crates/core/src/model.rs` | `1e864a3d1940a15e8a7e0f683bcad58927759f27f617e747e95c105857f559ab` |
| `crates/core/src/model/exact.rs` | `dd2130b64a501b5631143cf5cd08cf0f1776e44cec08aaffcba0423806dbaa00` |
| `crates/core/src/model/policy.rs` | `5f0ff1c02b1f2467e40b7c62f1e711a072eb1e1b1258ac83093d39f7d172f11e` |
| `crates/commands/src/lib.rs` | `802f0577d77f7e72ab39f5f40d6b5364e3b7a21a4a4535f0c5627ad78d7a1e89` |
| `crates/commands/src/read_cache.rs` | `8a17a575e8fbd158694844f54cb920962f2b752bc0509d3e8a1ad541fc7f7f45` |
| `crates/core/src/plan.rs` | `930c9843ee0148e70f56122549cc228333785c616ac62c57e937149b2c36a2f1` |
| `crates/core/src/plan/native_flac.rs` | `9b099cff59c406d4c80606e33e0ce48fb8c9df711ca527cb2b7d9d7757a11c63` |
| `crates/core/src/media_mutation.rs` | `a649306f23bc8a4c254ccbb32cbe5b020b1ef6879ea86a77f906acbd640d64ef` |
| `crates/core/src/resources.rs` | `a416ae31c50d45ec3aeaee114dea0b00a48aa04827ec0a35e9ec05ca1df0c4e2` |
| `crates/core/src/model/native_patch.rs` | `9f64ce7ca94d083cae86760e8d5148fb2ff8f9386b2839bdb35d50595582c348` |
