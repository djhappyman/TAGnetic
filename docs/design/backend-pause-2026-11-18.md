# Backend pause checkpoint — 18 November 2026

The finish-and-pause boundary is protected Linux FLAC
[PR #54](https://github.com/djhappyman/TAGnetic/pull/54), reviewed head
`bd2e6091518e77a15fc3c4c9080fc35cbe8fcdf7`. Its hosted matrix and merge
record are the authoritative delivery evidence. This handoff must land after
that code slice. All remaining implementation is paused until the owner asks
to resume. Start with [issue #51](https://github.com/djhappyman/TAGnetic/issues/51)
and [the programme](backend-parity-programme.md).

## Delivery records

- Service authority/lifecycle [PR #50](https://github.com/djhappyman/TAGnetic/pull/50)
  merged at `67625add6853e36e1f823ed3fb3b15df588e0bda`, containing reviewed head
  `a7a8b52e0a6221f9aaeb274e8bc158b180f85776`. All four hosted checks passed.
- Protected FLAC [PR #54](https://github.com/djhappyman/TAGnetic/pull/54) includes
  the reviewed shared file-authority/copy/move recovery prerequisites, native
  receipts/journal, per-App policy isolation and bounded compound IO admission.
  Coordinator/journal reviews corrected eight P2 finding groups; final policy
  review corrected two more groups and admission review corrected private-sink
  reentry. Portable review then corrected the shared operation boundary and
  native final-completion bypass; 36 distinct probes passed. Crash-harness review
  passed 43 behavior tests plus three transparent dispatchers. Original failing
  probes remain, and all counts overlap. Descriptor IO review accepted 31 scoped
  probes after macOS exposed shared descriptor cursors and lost filename hints;
  fresh sessions retain held-source identity and independent peer requests.
- Clean publication: 1,638 workspace tests passed, zero failed, 34
  optional/manual ignored; workspace all-target/all-feature Clippy and formatting
  passed. Three newly added child dispatchers are included in the 1,638 count.
  Two private D-Bus tests, UI check/lint/build and 216 unit tests passed.
  See [publication evidence](../performance/native-flac-publication.md) and its
  seven independent review reports for source hashes, gates and limits.

The native route covers whole plans of in-place plain-text FLAC edits on
supporting Linux profiles/SQLite journals. Mixed artwork/block/transfer plans,
typed numeric/date fields, other formats and unsupported profiles retain their
existing route. No exact Stage/Review DTO or new UI capability was exposed.
Windows/macOS hosted checks do not establish native protected-write support.
Old binaries must not be used to process native history; retain compatible code
and recovery material. Weak legacy transfer history remains readable but cannot
authorize unproved deletion/relocation.

## What remains parked

Broader resource/cache/query/DTO ownership, playback/seek/control, artwork/EXIF,
scalar CSV/TSV imports, report rendering/output ownership, bounded pattern helpers
and the stress-runner implementation remain preserved local WIP. Their component
reviews do not replace future integration review. The 48-case runner's 16 harness
tests do not prove the full backend programme. A NEW independent runner review
is the first verification task on a future resume. Reconcile its named cases
against the new actual-death and renamed same-thread unwind probes first.

Exact staging/DTOs, mixed/typed operations, native picture properties/order,
patterned exports, grouped reports/append, exact import mappings, extra pattern
helpers/mappings, recipes, portable settings/sources, formats/timestamps/chapters
and actual large-library/RSS/platform/device/NAS/power-loss evidence remain open.
Server-local SSH tagging remains the separate future requirement in
[#30](https://github.com/djhappyman/TAGnetic/issues/30). No product feature was
marked complete, no installer published and no live profile/Undo history changed.

## Recovery and resumption

- Parked checkout: `/mnt/Games/Development/tagrex/native-field-foundations`.
- Branch: `feat/native-field-foundations`; committed base `9da71249a62c75189b2131c379fc326244aac3c4`.
- Historical archive: `/mnt/Games/Development/tagrex/checkpoints/backend-paused-2026-11-18/`
  and ref `refs/checkpoints/backend-paused-2026-11-18` preserve the earlier snapshot.
- Updated archive: `/mnt/Games/Development/tagrex/checkpoints/backend-remaining-2026-11-18/`
  and ref `refs/checkpoints/backend-remaining-2026-11-18` preserve remaining WIP,
  binary patches, untracked source, manifest and compact review evidence. Its
  actual restore matched all 909 recorded source paths (160 untracked files),
  including bytes, modes and links; zero mismatches.
- The parked tree contains older equivalents of already integrated #50/#54.
  Final per-App/common-policy, admission, descriptor IO and actual-death harness fixes live in
  the publication commit/main, not
  retrofitted across the broad parked tree. Start from current merged main and
  curate remaining changes; NEVER apply the whole historical patch over main.
- Preserve the existing toolchain/dependencies/build cache/runtime profile. Source
  `/mnt/Games/Development/tagrex/dev-env.sh` before Rust checks. Do not reset,
  stash or rebase the parked checkout or alter the separate UI/runtime work.

On resume, read workflow, issue #51 and current main first. Verify archive/source
hashes and identify which remaining slice is being integrated. Use fresh
independent review for changed seams, then combined real workloads on one frozen
source. Explicitly retain unsupported/platform gaps. Require exact-head Linux,
Windows, macOS and advisory checks for each publication; release packages need
the separate build/release matrix. Upstream backend curation waits for compatible
foundations and must preserve upstream inline journal images and waveform API.
