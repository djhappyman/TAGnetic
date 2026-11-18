# Backend pause checkpoint — 18 November 2026

Implementation is paused at the owner’s request. Start with [resume issue #51](https://github.com/djhappyman/TAGnetic/issues/51)
and [the programme](backend-parity-programme.md). No implementation or review
agent remains active. Resume only when requested; no product feature was marked
complete by this pause.

## What is actually merged

[PR #50](https://github.com/djhappyman/TAGnetic/pull/50) merged at
`67625add6853e36e1f823ed3fb3b15df588e0bda`, containing reviewed head
`a7a8b52e0a6221f9aaeb274e8bc158b180f85776`. Linux, Windows, macOS and advisories
passed. It fixes service authority checks before history mutations, atomic
handle publication and cleanup of failed unpublished operation records.
The initial macOS runner-allocation cancellation was rerun successfully.
The clean publication branch/checkout were retired after ancestry verification.
This is the only newly merged code from the wider hardening programme.

## Preserved local work

| Component | Checkpoint at pause | Remaining gate |
| --- | --- | --- |
| Shared memory/cache/query/snapshot ownership | Component stress and independent reviews recorded | Complete DTO/response ownership, combined workloads, actual large-library/RSS proof |
| Playback/seek/control ownership | Multiple independent corrections/rechecks | Physical-device/native lifecycle proof |
| Copy/move/staging recovery | Fresh copy review: 437 passed, five optional ignored; strong ownership and duplicate-proof refusal | Native filesystems, power loss and low-latency busy cancellation |
| Exact held-file reads and selective candidate/receipt components | 100 combined tests, plus all six explicit manual probes passed; component reviews recorded | Their later integration must be reviewed anew |
| Protected media and held-parent opening | Fresh 71-test gate, one manual profile ignored; controlled parent substitution and real process-death evidence | Production coordinator and platform integration |
| Ordinary FLAC Apply/persistent Undo | Frozen author candidate: 850 core tests passed, 31 default ignored; 22 new public probes and two explicit manual probes passed | **NEW independent integration review — first on resume** |
| Artwork/EXIF, scalar CSV/TSV imports, structured report rendering/output ownership | Scoped implementations and reviews preserved | Native picture/stage/command/grouped export/append and UI integration |
| Stress coverage contract | 48 registered cases; 16 harness tests passed | **NEW independent runner review**, then full frozen-programme run |

These counts describe different overlapping suites and are not additive.
Author evidence does not replace independent acceptance or hosted results.

The new FLAC route covers whole plans of in-place, plain-text FLAC tag edits on
supporting Linux profiles/journals. Mixed artwork/block/transfer plans, typed
numeric/date fields, other formats and unsupported profiles remain outside this
route. The unchanged untouched-metadata preservation canary is green in that
candidate. No exact Stage/Review DTO or UI capability was exposed.

See [FLAC integration evidence](../performance/backend-native-flac-executor-readiness.md),
[stress coverage evidence](../performance/backend-stress-coverage-readiness.md),
[ranked findings](../performance/backend-roadmap-findings.md) and
[the protected-write design](protected-exact-mutation-integration.md). Earlier
component reports retain their historical hashes; the latest FLAC integration
changed some shared helpers and needs its own fresh review.

## Recovery location and how to resume

- Checkout: `/mnt/Games/Development/tagrex/native-field-foundations`.
- Branch: `feat/native-field-foundations`; committed base `9da71249a62c75189b2131c379fc326244aac3c4`.
- Recovery ref: `refs/checkpoints/backend-paused-2026-11-18` pins that base.
- Archive: `/mnt/Games/Development/tagrex/checkpoints/backend-paused-2026-11-18/`.
  It retains binary staged/unstaged/HEAD patches, untracked-source archive,
  file hashes/status, base history where needed and compact evidence. The archive
  preserves uncommitted work; it is not a reviewed feature commit or release.
- Keep the existing toolchain, dependencies, build cache and runtime profile.
  Source `/mnt/Games/Development/tagrex/dev-env.sh` before Rust verification.
  Do not reset/stash/rebase the parked checkout or change the separate UI work.

On resume:

1. Read `docs/WORKFLOW.md`, issue #51 and the two new readiness reports. Verify
   current source against their frozen hashes and the checkpoint manifest.
2. Use fresh independent reviewers for the FLAC coordinator/native journal and
   stress-runner contracts. Retain the original fault canaries. Fix and recheck
   findings before claiming either integration complete.
3. Run combined core/commands/service/player and selected actual media/stress
   checks against one unchanged source snapshot. The full stress runner is not
   verified by its harness tests. The earlier attempted programme invocation
   recorded a temporary compile failure honestly; it is not a final green run.
4. Reconcile already merged service PR #50, whose equivalent changes also exist
   in the parked tree. Curate independently reviewable code slices from current
   remote main; do not publish the whole dirty tree as one verified feature.
5. Require exact-head Linux/Windows/macOS/audit checks before each merge. Preserve
   upstream inline artwork images/waveform API; service upstream work waits for
   a compatible foundation. Then resume broader backend/UI work deliberately.

Broader exact staging/DTOs, mixed operations, native picture properties/order,
patterned exports, grouped reports/append, exact import mappings, extra pattern
helpers/mappings, composed recipes, portable settings/sources, additional
formats/timestamps/chapters and cross-platform/device/NAS/power-loss evidence
remain open in the product epics. Server-local SSH tagging remains the separate
critical future requirement in [#30](https://github.com/djhappyman/TAGnetic/issues/30).
