# Upstream relationship and contribution ledger

Tagnetic builds on [tagrex/tagrex](https://github.com/tagrex/tagrex). This fork's `main` is the canonical personal interface; upstream contributions use separate, curated branches. Personal branding, README, roadmap, versioning and release automation stay in the fork.

## Current coverage

### Protected FLAC and required recovery/policy foundations (18 November 2026)

Fork [PR #54](https://github.com/djhappyman/TAGnetic/pull/54), reviewed commit
`f7e258562505c925d08f49c544c495d450d4a22e`, carries Linux native FLAC text
Apply/persistent Undo, shared file-authority/recovery prerequisites and required
policy/cache/admission corrections. Four scoped independent reviews completed;
its exact-head hosted matrix and merge record are linked in the PR.

Upstream status: **waiting** for compatible native-field/text-codec foundations.
Upstream main has no matching values/exact metadata modules at this checkpoint,
so this focused integration is not a standalone buildable cherry-pick there.
Reusable changes target `tagrex/tagrex:main`; no upstream PR/acceptance is claimed.
Curate from an upstream base, preserve inline journal artwork and the stable
waveform API, and exclude fork identity/releases and incompatible pooled storage.

### Service authority and handle lifecycle (18 November 2026)

Fork [PR #50](https://github.com/djhappyman/TAGnetic/pull/50) merged at
`67625add6853e36e1f823ed3fb3b15df588e0bda`, containing reviewed head
`a7a8b52e0a6221f9aaeb274e8bc158b180f85776`. Linux, Windows, macOS and
advisories passed on that exact head. Revoked history admission, atomic handle
publication and failed initial-operation retry cleanup are reusable backend
fixes for `tagrex/tagrex:main`.

Upstream status: **waiting** for the compatible service/query foundation.
Upstream main lacks the service crate, so the focused fix is not independently
buildable there yet. No upstream PR or acceptance is claimed. Preserve upstream
inline journal images and the stable waveform API when preparing that foundation;
personal branding/release changes are excluded.

### 1.4.8 live fields and table stability

Fork branch `fix/behaviour-refinements` builds on the released 1.4.7 MCP service.
Feature checkpoint: `cdc0c66` (live fields and table stability).
Native incremental intents/deltas and transform basis tracking are eligible for
`tagrex/tagrex:main`; stable working-field projections, artwork isolation,
keyboard selection and scroll containment belong on `tagrex/tagrex:next`.
Status: **waiting** for the proposed 1.4.7 service/query foundation and the
unmerged Next interface foundation [#729](https://github.com/tagrex/tagrex/pull/729).
There is no compatible upstream service base for a standalone cherry-pick of
these native changes yet. Keep the backend and UI curation separate, preserve
upstream inline artwork journals/waveform APIs, and omit the personal 1.4.8
version, release identity and packaging changes. No upstream acceptance or
new upstream PR is claimed by this checkpoint.

### 1.4.7 collaborative MCP

The service/transport, shared working-value command helpers and scoped operation
protections are **proposed** for upstream `main`. Shared staging, Inspector and
consent interface behavior belongs on upstream `next`, after its interface
foundation. The current integration depends on the fork's library-query and
journal interfaces; curation must preserve upstream's inline artwork storage
and waveform API. No upstream acceptance is claimed by the personal release.
Personal versioning, application identity and release automation remain fork-only.

The 1.4.4 backend audit candidates are being integrated in the personal release.
Their upstream status remains **proposed**, with no additional upstream PR
submitted by this release task. Preserve upstream's inline artwork journal and
waveform API when preparing the corresponding focused contributions.

| 1.4.4 concern | Classification and route |
| --- | --- |
| Interrupted Undo, exact block preflight, pruning recovery and whole-block before/after fingerprints | Standalone backend → `tagrex/tagrex:main`; the fingerprint follow-up is fork commit `bbdd892` |
| Operation cancellation/admission, Beatport session lifecycle and bounded text/image work | Standalone backend → `tagrex/tagrex:main` |
| Provider SOCKS, release identity/URL validation and Beatport pagination | Standalone backend → `tagrex/tagrex:main` |
| Read diagnostics, durable settings, unavailable output recovery and safe export names | Standalone backend → `tagrex/tagrex:main` |
| Per-window navigation policy | Standalone backend → `tagrex/tagrex:main`; adapt application origins to upstream configuration |
| Read-error, audio-output and settings-recovery interface feedback | Shared interface → `tagrex/tagrex:next`; reconcile with the unmerged foundation below |
| 1.4.4 version, changelog and personal packages | Fork-only personal release metadata |

New whole-block batches persist content fingerprints; legacy history remains
readable and retains its older evidence rules. Native audio-device recovery,
live provider authentication and OS-specific filesystem/runtime behavior need
their own validation beyond ordinary compilation and fixture tests. MCP live
collaboration is a separate design exercise, not part of these changes.

| Concern | Upstream route |
| --- | --- |
| Tagnetic branding, icons and personal release metadata | Fork-only; independent product identity |
| Component workbench, spreadsheet, workspace and artwork interface | [#729](https://github.com/tagrex/tagrex/pull/729), refreshed as the one-time foundation against `next` |
| Blocking command queues | [#737](https://github.com/tagrex/tagrex/pull/737), accepted/merged into `main` |
| Interrupted rollback recovery | [#738](https://github.com/tagrex/tagrex/pull/738), accepted/merged into `main` |
| Metadata-only Undo history | [#739](https://github.com/tagrex/tagrex/pull/739), accepted/merged into `main` |
| Provider clients and streamed duplicate hashes | [#740](https://github.com/tagrex/tagrex/pull/740), accepted/merged into `main` |
| Display metadata cache | [#741](https://github.com/tagrex/tagrex/pull/741), accepted/merged into `main` |
| Shared artwork transport and encoding reuse | [#742](https://github.com/tagrex/tagrex/pull/742), open/proposed, `main` |
| Guarded/staged exports | [#743](https://github.com/tagrex/tagrex/pull/743), accepted/merged into `main` |
| Linux experimental compositor workaround | [#744](https://github.com/tagrex/tagrex/pull/744), Draft, `next`; native evidence pending |
| Allocation-free natural comparison | [#750](https://github.com/tagrex/tagrex/pull/750), accepted/merged into `main` |
| Sidecar filename filtering and per-preview directory batching | [#751](https://github.com/tagrex/tagrex/pull/751), accepted/merged into `main` |

These are submitted/proposed changes, not a claim that upstream has accepted them. The older combined #734 overlaps the focused native split and is not an additional implementation to merge. The maintainer controls merges into upstream `next`; this fork cannot make that branch current unilaterally.

The foundation is intentionally the one broad exception while it remains unmerged. Subsequent interface work should be small follow-ups once that base is available. Backend PRs must land on `main` and flow to `next`; the foundation's bundled native snapshot must be reconciled with the reviewed implementations before merge. The open proposals retain their documented platform/evaluation gates; ready-for-review status does not imply acceptance.

## Workbench/theme checkpoint reconciliation (2026-11-14)

Historical checkpoint details follow. Since that snapshot, fork #12 merged at
`3f740bf`, upstream #743 and #761 were accepted, and #742 was reconciled with
main at `104a6fc` with all four hosted checks passing. Tagnetic branding is
fork-only and is not proposed to upstream main or Next.

Fork [#11](https://github.com/djhappyman/tagrex/pull/11) is merged into fork `main` at `a1a2ad31046dbcab1e3278c2e3421b2cac200f0d`. All four checks passed on head `0e35c26`: Linux, Windows, macOS and advisories. It carries all six UI checkpoints, staging catalogue quoting and a Chromium-compatible input-width assertion; the assertion retains its original threshold and changes no production layout. Compatibility follow-up [#12](https://github.com/djhappyman/tagrex/pull/12) now targets actual fork `main` and requires its own exact-head platform/filesystem gate before merge.

The native compilation-field metadata is separately proposed to upstream `main` in [#761](https://github.com/tagrex/tagrex/pull/761), head `e4b9f93`. It adds optional metadata for the existing Preview/Apply path, without new file writes or journal/player changes. It is included as an explicit pending-main dependency in #729. The latest review adds fallback from an incapable read block to the primary write tag type, with real-file cached-row wiring and key-mapping regressions.

All six workbench/theme UI checkpoints and their catalogue correction are now proposed within the existing one-time foundation [#729](https://github.com/tagrex/tagrex/pull/729), at `b938880b0cfe4e7a5aa7adc762e3651954b5aef8`. The additive compilation-field metadata from #761 is included as an explicit pending-main dependency. This closes the UI publication gap without opening follow-ups that repeat the entire unmerged foundation. Its native snapshot now reconciles accepted main/cache changes with the artwork/export proposals, compilation metadata and primary-writer fallback. Upstream approval of pending native proposals and the experiment's platform/performance evaluation remain gates; publication does not mean acceptance. Prepared intermediate refs remain in the verified recovery bundle and are historical, not separate open PRs. Future contributions should be small once the foundation lands.

Fresh local checks: fork source has 91 unit tests and 404 browser tests passing (6 optional skips), Svelte check/build and 617 core/commands tests passing (9 ignored). Prepared Next source has 85 unit and 403 browser tests passing (6 optional skips), Svelte check/build; the subsequent quote-only correction passes native and JS catalogue checks. These Linux checks do not establish native Windows/macOS appearance or hosted success.

## Maintainer review follow-ups (2026-11-14)

- #741: cache documentation restored; helper/item documentation and spacing added. Current-main reconciliation retains both cache and sidecar modules. The optional core constructor refactor is deferred to keep the reviewed change narrow. The same documentation correction is in the fork.
- #742: upstream uses a simpler request-scoped SHA-256 encoding map, with current-main conflicts resolved. The personal fork deliberately retains its faster identity-cache implementation: a release helper comparison at 100 references to one 1.9 MB image measured 3.26 ms versus 88.84 ms, median of nine runs, with identical output. This is not app-level timing. The [maintainer was informed](https://github.com/tagrex/tagrex/pull/742#issuecomment-6879665997).
- #743: guarded exports now support Unix filesystems without exclusive rename or hard links through a checked rename fallback. The last fallback has an explicitly documented race between absence check and rename; permission errors and pre-existing targets do not enter it. Windows cloud tags are distinguished from path-redirecting reparse tags; traverse rights replace directory-list access for ancestor pins. FAT/exFAT CI is separate from ordinary platform CI. macOS FAT32 and Windows exFAT passed on the first revision; corrected ExFAT-image/PowerShell test setup still needs exact-head hosted verification. These fixes are also in the fork follow-up.
- #744 is now accurately marked Draft. Native flash and restart/launch-path evidence remain outstanding; a green build does not close them.
- New upstream Ogg-codec refusal, AppleDouble scanning and dependency safeguards are adopted in the fork without changing its interface, identity or release policy. They refuse unsupported Ogg FLAC before an unsafe AAC misidentification, exclude macOS `._` files, enable the trash 5.3 Linux backend explicitly, and keep incompatible Windows/zbus major updates out of automated proposals.

The maintainer has merged #741, #743 and #761 into main. #742 remains open for maintainer review. No upstream merge was performed by this agent. Fork follow-ups require their own full hosted gate before merging.

## Explaining intentional differences

- Table dragging is designed around model-order selection, including grouped/offscreen rows. Filename-area gestures select rather than silently reorder files; spreadsheet editing and existing modifier semantics remain explicit. Preserve the tests for this workflow when reconciling main's different drag implementation.
- Automatic previews enable fast comparisons, but staged tagging/artwork/rename changes still require Apply. Validity and scope guards keep invalid or pending filters from applying a misleading selection; Apply retains an independent freshness/validity boundary.
- The component interface adds npm/build dependencies in exchange for retained controls, keyboard/focus behavior and consistent interaction surfaces. Its performance and maintenance costs must be measured rather than assumed better.
- A UI-specific workaround should not be imposed on stable main without a reproduction there. The compositor draft remains scoped to Next and retains launch-path and native-rendering acceptance gates.

When implementations conflict, document the concrete behavior, the reason it fits this interface, the safeguards/tests, and the tradeoffs. Do not replace an upstream interaction simply because it is newer, or describe this fork as universally more robust without evidence.

## Deliberate compatibility boundaries

The personal branch retains its existing pooled-artwork journal implementation. That on-disk format is not submitted as part of the Next refresh: upstream explicitly requires stable-readable inline image records, and those records and the cross-version probe are preserved there. No existing journal is silently converted. Sharing artwork in memory/transport and encoding reuse are separate from the journal format.

The compact personal player no longer uses the waveform analysis API. The upstream refresh retains that shared API and its tests because stable main still uses it; removing the control from one interface does not justify removing another interface's backend capability. These are intentional differences, so interface parity does not mean byte-identical Rust/configuration trees.

## Selected upstream adoption (2026-11-15)

The personal fork adapts upstream main's APE/WavPack write guards (`c106b1c`,
`741dde3`), concrete read fallback (`23416bc`), filename matching (`abc6388`),
unmatched-import scope (`25f074f`), manual-order retention (`49ecbb9`), shared
length offset/order confirmation (`c0f7c85`, `0830f44`) and session Redo
(`8e17613`). These behaviors already exist upstream; no duplicate native PR is
needed. Tests retain upstream's disposable malformed-file fixtures.

The integration preserves this fork's captured selection, live staged inputs,
query snapshots, pooled artwork journal and component ribbon. Those compatibility
adaptations belong to the personal implementation. The duration evidence runs in
Rust over cached reads; the frontend confirms an explicit reorder and rechecks
its captured subject after asynchronous replies. Existing release identity and
font choices remain intact.

The legacy-JS undefined-name check adapts `e40bfda`; output-test token scoping
adapts `2947a3d`. Only the missing condensed-font coverage from `fe97686` is
adopted. The other bundled font families already contain Greek and Vietnamese.

## Backend integration and upstream review (2026-11-18)

The personal integration combines fork main `d4df9a35` with the completed
backend checkpoint `9da71249` from `feat/native-field-foundations` (19 commits).
Scope: native field recipes, imports/extraction/numbering, bounded playlist/text
loading, cache/query/staging/export/artwork/queue/player hardening, their tests
and guides. Completion requires combined-source verification and hosted Linux,
Windows, macOS and advisory checks before merging to fork main. No release or
new interface feature is implied. Existing uncommitted research and the live
runtime remain outside this integration.

Shared native fixes remain eligible for focused `tagrex/tagrex:main` curation;
service/query integration needs compatibility review, preserving upstream inline
artwork journals and stable waveform APIs. Existing fork UI adaptations route
to `next` after foundation #729. This integration does not claim new upstream
submissions or acceptance. The separate journal-permission fix is already in
fork main at `805bdd4d`; its upstream proposal is
[#791](https://github.com/tagrex/tagrex/pull/791). The shared-artwork and compositor
proposal branches must not replace the fork's intentional implementations.

Upstream main reviewed through `bf8e68475de702d7a43a76bc3b43094364476d9b`:

- [Combined generator plan](https://github.com/tagrex/tagrex/commit/ff5479fe988352307265d9a1f73498f870bd0018):
  vinyl conversion, numbering and transforms run in order with one preview/Undo
  entry. Useful candidate; adapt to the fork's native batch and staged-value
  model rather than replacing its component generator with the static UI.
- [Editor presets](https://github.com/tagrex/tagrex/commit/10221c42d15cc6cd0de2c0d6b4b167e58b669120):
  opt-in, independently selected tag presets on typed field values. Useful
  candidate requiring Inspector/staging integration and stale-result guards.
- [Write and next](https://github.com/tagrex/tagrex/commit/eea2a1e8deb5b5f187d5adffcca217de83703721):
  single-file editor write, then advance and restore field focus. Useful candidate;
  retain refusal for multiple selection, staged plans and invalid fields.
- [RPM 6 updater](https://github.com/tagrex/tagrex/commit/ee0a394095e51f1603f3bae6ded783177e8ffb77):
  signed-download validation followed by polkit installation of an unsigned RPM,
  retaining digest checks and clearer errors. Evaluate separately against the
  personal updater/release identity; it is not adopted by this integration.
- [Font-subset recipe](https://github.com/tagrex/tagrex/commit/4e7347ad45cc9a78a9ebe46a9a5a2c347eaff62c):
  reproducible version-pinned font tooling with a non-writing check mode. Useful
  maintenance candidate; account for the fork's additional font families.

The upstream review is a shortlist, not adoption. No upstream release identity,
version numbers or experimental Next branch is merged into personal main.
The backend's documented stalled-output seek limitation remains open; this
integration does not claim physical-device loss or gapless validation.

PR #29 compatibility follow-up: the new bounded queue admission uses an explicit
compare-exchange loop instead of `AtomicUsize::fetch_update`, which Rust 1.100
deprecates. This preserves Rust 1.90 compatibility and strict-warning builds
without suppressing the lint. It belongs with the new queue implementation's
focused upstream curation; it is not a standalone patch to upstream's older
queue implementation.

PR #29 playlist follow-up: retain lexical and canonical spellings of each
explicitly approved root, so absolute `/var/...` entries match a macOS
`/private/var/...` root. Read only the matched suffix beneath the canonical
root through the existing pinned, no-follow input path. Root-alias deduplication
and refusal of final/escaping symlinks have disposable-fixture regressions.
This shared backend fix belongs with focused playlist-loader curation to upstream
`main`; upstream does not yet have this fork's loader implementation.
