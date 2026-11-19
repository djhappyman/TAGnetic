# Backend programme: ranked findings and correction gates

Paused engineering evidence, 18 November 2026. Resume through [tracker #51](https://github.com/djhappyman/TAGnetic/issues/51) and [the handoff](../design/backend-pause-2026-11-18.md). Priorities describe consequences
inside disposable fixtures; they are not a public vulnerability assessment.
Passing a corrected scope does not certify the whole backend or platform matrix.

| Priority | Confirmed failure | Practical consequence | Current gate |
| --- | --- | --- | --- |
| P1 | Undo/rollback accepted replacement copied or moved objects | Another file could be deleted or relocated and history discarded | Strong copy/native move evidence implemented; replacement/restart/rollback canaries and subsequent scoped reviews pass. Private move staging and typed role corrections have separate fresh-review gates below. |
| P1 | Legacy copied-file history has a strong hash but no typed native identity | An identical foreign replacement can be deleted and its history discarded | Retained public Executor/SQLite canary reproduces the old predicate. New copy ownership requires typed native proof; candidate and fresh independent copy-checkpoint review pass; older unproved history remains readable but cannot authorize deletion. |
| P1 | Title-only scalar edit changes untouched native lists | Literal separators split; duplicate-case values and empty custom values disappear | The unchanged canary passes in the new whole-plan, in-place plain-text FLAC candidate; coordinator (135 tests) and journal (61 tests) independent reviews completed after corrections. Final per-App policy/admission compatibility review accepted its corrected scope in #54. Mixed/typed/other-format routes and exact stage migration remain open; not merged or UI-exposed. |
| P1 | Protected-write publication detects changed recovery/candidate bytes after writing | The sole original image can disappear, or foreign candidate bytes reach the original | Candidate/image sealing, alias exclusion and terminal proof corrections pass successive independent rechecks, including 175 retirement mutations. Narrow Linux FLAC Executor/Apply/Undo integration is included in #54; broader operations and native platforms remain open. |
| P1 | `$cutmix` splits at an invalid Unicode byte boundary | Some real text can panic the renderer | Retained regression and correction pass the separate bounded-render review, including Unicode and resource-pressure probes. |
| P1 | Revoked history request mutates media after waiting for the App lock | Undo/Redo can change files/history while returning permission denied | Same-lock history/write checks pass the separate lifecycle/history review. Focused service authority PR #50 merged at 67625add after Linux, Windows, macOS and advisories passed on reviewed head a7a8b52e. |
| P1 | Seek/control pipeline crosses source/output ownership boundaries | Stalled output, stale seeks or changing formats can hang or publish incorrect audio state | Fixed-output pipeline and seek ownership corrections pass multiple separate reviews; physical-device/native lifecycle checks remain open. |
| P1 | Recovery artifacts can be selected through a path alias as new media | A second publication overwrites the first operation’s sole old preimage | Physical identity exclusion and sealed recovery objects implemented. Original alias regressions and subsequent terminal-proof review pass within the Linux primitive; Narrow Linux FLAC integration is included in #54; other profiles/operations remain open. |
| P1 | Output rollback removes a foreign replacement at an earlier published path | A later collision can delete unrelated replacement bytes | Original regression passes after ownership hardening. A separate review passes seven new tests, 18 fault cases and 40 overlap rounds after two further corrections; durable restart/append and native platforms remain pending. |
| P2 | Exact-read compatibility decoding expands before later native limits | Dense ID3 lists, large Latin-1 scalars and MP4 UTF-16 can exceed the 64 MiB work reservation before refusing | Early member/raw/decoded/aggregate bounds pass 14 independent allocator and valid-boundary probes, with no budget increase. Eight fresh held-file probes also pass after correcting the final callback ordering, which could otherwise accept changed source or policy after validation. |
| P2 | Media preparation accepts an AfterEdit observer mutation as the editor result | Foreign candidate bytes can become the accepted prepared output | Edited witness is fixed before the observer and rechecked at later preparation boundaries. Original canary and separate 56-test held-editor gate pass; Narrow Linux FLAC Executor/journal integration is included in #54; other profiles/operations remain open. |
| P2 | Native receipt decoding allocates impossible node collections or undercharges retained vector capacity | Damaged receipts can exceed admission or fail only after a restore starts | Early minimum-size/single-target validation and actual vector-capacity accounting pass 10 independent and eight author codec probes. Narrow FLAC journal authority binding is included in #54; broader exact staging remains open. |
| P2 | NUL-dense stale ID3 frames expand past native writer work admission | Malformed metadata can allocate 161.8 MB under a 134.2 MB reservation before refusing | Shared record parser now checks member count before expansion. Unchanged dense-frame canary peaks at 6.3 MB; maximum-record canary peaks at 16.8 MB after early ambiguity refusal. Separate writer review passes 9 new tests plus 19 author tests. |
| P2 | Stored move-stage context can be changed without binding it to the recorded plan | Restart recovery can relocate payload to an unapproved name and even falsely complete Undo/discard history | Whole-collection validation and explicit Unix U2 roles pass a separate 21-test review, including 61 malformed identity/seal cases and legacy-link controls. Performance and native platform gates remain. |
| P2 | Undo silently selects one of several ownership rows, then fails after earlier removals | Ambiguous later copy proofs can leave an unsuccessful Undo partially applied; a failed reservation can also rewrite history | Whole-batch duplicate refusal now precedes recovery; SQLite reservation is transactional and the memory adapter matches its uniqueness/idempotency rules. Original three independent regressions pass; the fresh retained gate passes 437 tests with five existing optional tests ignored. Native platforms and power loss remain separate. |
| P2 | An interrupted batch can leave a completed copy without persisted ownership evidence | Undo preserves source/copy/history but refuses automatic copy cleanup | Per-copy checkpoint and post-metadata refresh candidate passes 442 retained tests, real process-death/partial-write and SQLite-full/locked probes. The fresh retained 437-test gate passes after duplicate-proof corrections; busy SQLite cancellation can still take about 10–15 seconds. |
| P2 | Output publication trusts a held root after its logical name is replaced | Success can be reported while the requested output is absent at its intended location | Current public root native identity is rechecked. Retained root/nested replacement probes and the separate seven-test output review pass. |
| P2 | Exact reads mix source files or leak an outer source into a nested request | Exact/native and legacy fields can describe different files | Held source plus public-request isolation implemented; original regressions and eight fresh independent probes pass. |
| P2 | Base/derived query ownership sits outside shared resource admission | Long metadata/computed fields can multiply retained memory | Base rows and computed overlays pass independent allocation/concurrency/cancellation reviews. Service snapshot sharing/policy correction passes original regressions; its independent review and remaining DTO/response ownership are pending. |
| P2 | Service snapshots rebuild an unadmitted base index or reuse an obsolete policy | Repeated reads multiply metadata retention or expose obsolete interpreted values | Shared admitted rows, scoped views and generation checks pass three public-service regressions and seven independent scope/concurrency/policy/lifecycle probes. Original DTO/JSON owners remain a separate resource scope. |
| P2 | A revoked in-flight snapshot inserts after client-owned handles were retired | The denied request retains an unreachable resource owner | Same-state-lock authorization before insertion fixes the retained public race canary; broader handle lifecycle implementation is now assigned. |
| P2 | Empty tag-block labels allocate before admission checks | A caller DTO allocates 2.4 MB under a 64 KiB budget before refusal | Count and checked overhead fixed; unchanged allocation regression and 256/257 boundaries pass. |
| P2 | Dead weak artwork entries outlive their charged retention | Retained cache memory greatly exceeds the coordinator's account | Bounded bookkeeping and lifetime accounting implemented; independent retained-allocation probes pass. |
| P2 | Resource pressure suppresses artwork size warnings | UI can mistake an unchecked image for a verified small image | Explicit Busy/Unknown inspection states and plan notes implemented and rechecked. |
| P2 | Conflicting EXIF orientation entries are silently selected | Rotation becomes permanent and contradictory evidence is discarded | Strict bounded parser corrected; fresh endian/format/cancellation/pixel checks pass. |
| P2 | A moved sidecar link cannot Undo after its referent disappears | Restart/retry gets stuck despite retaining the correct link object | No-follow entry containment/existence corrected; all six fresh move-identity probes pass. |
| P2 | Preview parsing/swallowed errors bypass cancellation checkpoints | Cancelled work can return a partial preview | Strict cancellation propagation and monotonic token checkpoints implemented; retained CSV/import/rename/scan probes pass. |
| P3 | A held-parent store candidate accepts an O_PATH directory until its first sync | A refused opening can leave a newly created directory behind | Unsupported descriptor flags now refuse before namespace effects. Original canary and fresh 71-test media gate pass; controlled parent-swap probes preserve the foreign profile. |
| P3 | Partially successful output cleanup reports deleted private directories as retained artifacts | Recovery instructions include paths that cannot be inspected | Explicit private retirement state and definite-absence filtering pass the independent multi-parent cleanup regression. |

The important remaining foundations are the open preservation/publication/context
failures, complete memory ownership, platform verification and performance under
the stronger durability protocol. The private staging implementation's initial
5,000-folder probe took 25.41 s Apply / 30.29 s Undo versus 3.80 s / 3.40 s for the
prior identity-only implementation. A separate 1,000-file proxy profile measured
about 0.83 s per phase inside intent SQL calls, out of approximately 3.1–3.5 s
per complete phase. A subsequent reverse cleanup index removes repeated whole-map scans, with the same durable intent transactions: the author observed 16.97 s / 17.13 s at 5,000 folders. Separate 512/1,000-folder observations and authority canaries pass. Unused SQL batching was removed; this is not a throughput SLA or proof that the remaining time is solely fsync.

Reports, retained canaries and implementation boundaries are linked from
[the programme](../design/backend-parity-programme.md). The focused service authority slice merged in [PR #50](https://github.com/djhappyman/TAGnetic/pull/50)
at `67625add`, with all four exact-head hosted checks passing. This verifies that
PR scope; no whole-roadmap completion or release availability is claimed.

## Final FLAC publication compatibility review

PR #54 (reviewed head `d1998ab2`) includes corrected per-App metadata policy,
rollback after settings changes, valid-generation display/query caching and
bounded compound native admission. The final policy review accepted 17 independent
probes; admission review accepted 54 tests after correcting private-sink reentry
before callbacks/history. Combined publication workspace: 1,638 passed, zero
failed, 34 existing optional/manual ignored. See [publication evidence](native-flac-publication.md)
for exact source hashes and hosted matrix/merge link. These overlapping component
gates are not additive or a complete-backend/platform readiness claim.

### Portable execution follow-up

Windows execution exposed a P2 shared operation-boundary gap: legacy Apply/Undo
continued after invalidation that the native route refused. Portable probes also
caught a P2 native final-completion bypass. Both are corrected; the fresh review
accepted 36 distinct tests. Settings are pinned for the synchronous operation,
owned Apply rollback is sealed, and legacy Undo remains incremental.

Surviving panic now runs owned rollback. Restart scenarios use genuine bounded
child exit 73, retaining original foreign-data/whole-collection assertions. A
separate review passed 43 behavior tests and three dispatchers; same-thread
nested unwind retains its outer-authority assertions and verifies inner cleanup.
Final combined scope: 1,638 passed, zero failed, 34 optional/manual ignored,
workspace Clippy/formatting and two private D-Bus tests passed. Hosted results on
the linked PR head remain the delivery gate. No full-platform/native/RSS/release
claim follows; the runner contract must be reviewed on current main before
broader work resumes.


### Descriptor IO follow-up

macOS exposed a P2 legacy metadata IO fault: descriptor aliases share file
positions and discard logical filename hints. Fresh held-parent sessions now
check the opened object against the held source before IO. Independent review
also corrected a P2 nested-peer root regression and accepted 31 scoped tests.
See [the independent review](backend-descriptor-io-independent-review.md) for
primary source evidence and exact hashes. Linux emulation establishes the
cursor mechanism; actual macOS hosted execution remains required before merge.


### Windows fixture admission follow-up

Windows reached seventeen direct move-staging unit tests whose disposable roots
bypassed the canonical-root preparation used by public Apply/Undo. Both that
fixture and the direct descriptor unit fixture now normalize their owned roots
before constructing paths. Production authority predicates and original fault
assertions are unchanged. [Independent review](backend-windows-authority-independent-review.md)
checked the previously hidden bodies and found no further scoped defect from
source; exact-head Windows execution remains required.


### Created-directory receipt comparison

Windows passed the previous core fixture and command failures, then found one
Executor assertion comparing resolved receipt paths with raw temporary-root
spelling. [Independent review](backend-created-directory-path-independent-review.md)
accepted canonical containment comparison while preserving raw public inputs,
folder counts, Undo removal and pre-existing-folder survival. The old assertion
reproduced under a Linux symlink-TMPDIR; all 48 Executor cases passed under the
same alias after correction. Fork CI now collects every target failure using
`--no-fail-fast`; failed targets still block merge. Backend code is unchanged.


The Unix-only cleanup-index fixture had the same private canonical-root
precondition; its original setup failed under a disposable symlink TMPDIR.
Two test-only setup lines now normalize that root, and both retained index and
retirement cases pass under the alias. All alias-redirection and retained-pin
assertions remain. This is local Unix evidence, not actual macOS execution.
