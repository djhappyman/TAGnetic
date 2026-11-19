# Backend platform review — 19 November 2026

Three independent source-review lanes examined PR #55 from `origin/main` to
`59ce27e3d13add8cce87f7c80f1d108edd8b6371` in the same consolidation worktree,
then implemented assigned corrections without creating branches or worktrees.
The orchestrator inspected their reports and combined diffs. No local build,
test, formatter, benchmark, media write or application restart ran.

Status: **review corrections prepared; exact-new-head hosted verification pending**.
Source review and clean Git whitespace checks do not establish runtime passes.

## Ranked corrections

| Priority | Finding | Correction and required evidence |
| --- | --- | --- |
| P2 | Table imports and shared recipes sampled standalone policy between App-owned reads | Pin the owning App for the whole operation; check after callbacks including final publication. Sweep public callback boundaries, require no media writes on owner change and exact values under peer changes. |
| P2 | Recovery locators could advertise a foreign/stale directory after rename | Darwin uses the held descriptor's `F_GETPATH`; Linux procfd and logical fallbacks are validated against the held device/inode. Missing locators stay explicit errors. Retain malicious replacement assertions and test rename/deleted-name reuse. |
| P2 | Recovery manifest serialization could panic on valid non-UTF8 Unix names after namespace effects | Use explicit display text plus bounded lossless Unix-byte/Windows-UTF16LE base64 evidence, streamed into the held manifest file. The new Linux forced-recovery canary requires `RecoveryRequired`, raw-byte round-trip and surviving original/foreign bytes. No automatic replay authority is added. |
| P2 | Original-output deduplication still used a cached legacy Windows identity | Use the same fallible live native identity witness as other ownership checks. Run the real hard-link refusal case on Windows NTFS as well as Unix. A real ReFS legacy-ID collision is not executed evidence. |
| P2 | Table admission errors lost the shared resource error codes | Route desktop and service errors through existing resource conversions. Exhaust real job slots, require canonical Busy, unchanged stage/media, and successful exact-value retry after releasing pressure. |
| P2 | Four emitted backend messages were absent from catalogues | Add resource Busy/Limit and artwork inspection Busy/Unknown messages to all seven existing locales, preserving plural forms and the unchanged catalogue invariant. |
| Contract | Older policy fixtures changed standalone defaults but expected an owning-App generation change | Change the actual owner, retaining obsolete-query refusal and historical-page assertions; add explicit standalone and peer isolation controls. Do not restore global policy coupling. |
| Contract | APFS fixtures assumed arbitrary invalid Unix byte names could be created | Keep raw-byte Unix coverage; use distinct supported Unicode on Apple, require invalid-byte refusal without effects, and verify NFC/NFD alias overwrite/duplicate-original behavior. |

## Broader review scope

Each lane reviewed applicable changed production paths and integration seams in
cache/query/admission ownership, service snapshots and grants, preview queues,
callback cancellation, metadata policy, artwork/EXIF processing, bounded masks
and reports, scalar tabular imports, player seek/conversion/router lifecycle,
held-file output publication/recovery and platform CI. Platform reviewers read
relevant dependency implementations and existing exact-head failure logs.
Representative test contracts, dispatchers and ignored-probe selectors were
inspected; this is not an exhaustive audit of every corpus entry or test line.

The graph index is for a different checkout and is stale against this candidate.
Coverage reports the consolidation paths as outside that project. Conclusions
therefore use exact source/diffs/logs, not graph absence/completeness claims.

## Hosted gate

Require all applicable exact-head checks: Linux, Windows, macOS, advisories,
Windows exFAT and both macOS FAT-family volumes. Pending, failed, skipped and
cancelled results remain blocking. The frozen head's native volume jobs passed;
ordinary Windows/macOS workspace jobs failed and do not validate these fixes.

Linux CI additionally selects six previously ignored independent FFmpeg pixel/
EXIF and Python CSV/XML consumer readbacks, alongside real decoder, tabular
media/pressure and cache-pressure probes. All declared function selectors were
checked against source. Probe groups collect failures rather than stopping at
the first failed command; any failure still fails the job. They can run after a
workspace test failure when its prerequisite build succeeded. The formatting
failure gate remains enforced. No new test passes are claimed before logs exist.

## Remaining limits

- P3 follow-up: Windows destination-name duplicate keys still use a lossy Unicode
  lowercase display key. Some native UTF-16 or case-sensitive NTFS names may be
  conservatively refused as duplicates. No foreign overwrite was established.
  Native probes and a filesystem-aware filename contract remain future work.
- No actual ReFS collision, physical audio-device latency/disconnect, NAS or
  power-loss certification follows. Windows decoder/manual external-consumer
  evidence is narrower than the Linux hosted component probes.
- Protected native FLAC remains the narrow supporting-Linux/SQLite route already
  merged by #54. General repeated-value, mixed/typed/other-format and UI exposure
  remain separate. Unconfirmed/unsupported Apple output volumes refuse before
  staging; refusal coverage is not export support on those volumes.
- Recovery manifests remain nonautomatic inspection evidence; an unlocatable
  held directory is reported honestly instead of advertising an unrelated path.

Full lane reports and exact logs are retained in the workspace checkpoint
`checkpoints/backend-consolidation-2026-11-19/reviews/` and its `evidence/`
directory. Shared backend work remains eligible for curated upstream main after
compatible foundations, preserving upstream inline journal images/waveform APIs.

## Hosted behavior verification after review

On `52a845b8`, Windows, macOS, advisories and all three native filesystem jobs
passed. Linux completed frontend checks/build/UI tests, Clippy, workspace build,
workspace tests, decoder/import/cache-pressure probes, all six independent
artwork/report consumer oracles, and private D-Bus tests successfully. The final
Linux job failed only its enforced formatting gate. Its hosted formatter patch
was downloaded and applied; no formatter ran locally. These behavior results
were collected after the runner applied that patch to its workspace.

[Linux evidence](https://github.com/djhappyman/TAGnetic/actions/runs/44501793692/job/135483311999).
The formatting correction still needs the full matrix on the final pushed head
before merge; consult PR #55 for that authoritative current delivery gate.
