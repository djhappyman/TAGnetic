# Backend handoff for the next session

> Superseding owner request, 19 November: consolidate and merge achieved work,
> move the development server to main, and retire non-main working branches.
> Follow [the consolidation record](backend-consolidation.md). The pause
> instruction below describes the preceding checkpoint. No local workloads.

Latest owner instruction, 19 November 2026: update README and GitHub issues,
push the documentation and merge PR #53 after its required checks pass, then
stop. This supersedes the earlier request to continue all backend work. Tomorrow
we will look at connecting work already built to the frontend; this is a
starting point for that session, not authorization to begin the wiring now.

## Execution constraints

Builds, tests, benchmarks and stress execution stay off the owner's machine.
Use hosted runners for runtime verification. Keep local work to lightweight
source review and edits, cap active agents, and retire their unused helper
processes. Preserve the live app, library, profile, dependencies and Undo history.
A passing historical Linux checkpoint does not verify a changed candidate.

## Delivered foundation and next step

Frontend #52 and service #50 are on main. Backend #54 merged at
`cb4f280b2b2a686b7cc613d4a890b2e4a94f880f`, containing exact checked head
`0ace5efebf2464df64fed2c10c228b257d58abe7`. Linux, Windows, macOS and advisories
all passed. Documentation is delivered through #53; its merge record is the
final evidence for this stopping point.

Begin the next session from merged main. Inventory which existing frontend
entry points can call the delivered backend without bypassing staging, field
locks, unrelated pending edits or persistent Undo. Keep unsupported capabilities
explicit; do not assume backend foundations already have complete UI exposure.
Do not change the running app/source attachment or restart it until requested.

The broader checkout and recovery archive remain preserved. The runner received
an independent static recheck: 40 source test methods, zero new local execution.
Windows birth-owned process supervision is still an open P2; hosted runner
contracts and actual programme workloads have not passed. This is not complete
backend readiness. Preserve the whole remaining WIP; do not apply its historical
patch wholesale over main.

## Preserved future backend order — not active work

1. Reconcile remaining WIP against merged main; independently review and repair
   the stress coverage runner, including real-death and renamed unwind cases.
2. Integrate shared resource/cache/query/response ownership and playback/seek/
   control/lifecycle slices with bounded memory and stale-result refusal.
3. Complete exact Stage/Review DTOs and repeated-value propagation through
   native reads/writes, persistence, Apply/Undo, mixed and typed operations.
4. Integrate artwork processing, EXIF, picture properties/order and guarded
   patterned export; verify aggregate decoded-image limits.
5. Complete tabular mappings/match diagnostics, playlist loading, grouped reports,
   encodings/BOM and safe append through existing stage/output ownership.
6. Complete pattern/value/path helpers, mappings and composed recipes; then
   versioned portable settings/recipes and validated metadata sources.
7. Implement the tracked server-local SSH tagging backend (#30), preserving
   remote preview/Apply/Undo and avoiding transfer of audio payloads; verify its
   transport, identity and recovery boundaries with disposable hosted fixtures.
8. Complete promised format policies, timestamp/container/chapter support with
   explicit per-format preservation evidence and unsupported outcomes.
9. Run combined hosted fault, cancellation, pressure, concurrency, restart and
   scale workloads on frozen source. Track physical-device, filesystem/NAS and
   power-loss evidence honestly; do not substitute unit tests for those gates.
10. Reconcile README, roadmap, issues and upstream routing with actual merged
   behavior. Backend delivery does not mark unfinished UI features complete.

Use issue #51 as the paused checkpoint/resumption tracker and the programme for capability
scope. Curate remaining changes from main; never apply the old whole-programme
archive over already integrated fixes. Shared backend work targets upstream main
when compatible foundations exist, retaining inline images and waveform APIs.

Completion means implemented, independently reviewed, hosted-verified and merged
scopes with remaining limitations explicit. Do not report the whole programme
complete while required work or evidence remains.
