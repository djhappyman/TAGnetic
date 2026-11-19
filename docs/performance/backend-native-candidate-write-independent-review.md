# Independent selective candidate-writer review

**No new blocker found after two admission corrections.** This scope is the pathless disposable candidate writer and
its exact intent validation, on Linux. It is not approval of production Apply,
persistent Undo, the parent's new receipt codec, or the complete backend roadmap.
Only `core/tests/native_candidate_writer_adversarial_review.rs` and this report
are owned by this review. No production, live app/profile/media, commit or PR
changes were made here. Reusable fixes belong in curated upstream
`tagrex/tagrex:main`; fork storage and upstream inline journal compatibility remain
separate gates.

## Confirmed P2: ID3 value expansion precedes admission-shape validation

A prepared ordinary UTF8 Title intent was followed by a stale candidate carrying
six duplicate Title frames, each with 900,000 empty values. The metadata is under
5.5 MiB. The actual public candidate parser expanded their String vectors before
rejecting target duplication. An isolated System allocator observed a
**161,795,816-byte** peak delta against the admitted **134,217,728-byte** transient
reservation. The candidate was refused unchanged and all charges released; the
confirmed consequence is resource-contract overshoot, not byte loss.

The parent added a bounded 1025-item counting pass before ID3 list collection.
The unchanged canary then passed with a **6,300,102-byte** peak delta. Inclusive
1023/1024-member Apply and exact restore controls pass; a stale 1025-member frame
refuses without changing bytes.

A further retained boundary canary combines 4095 duplicate targets with 1024
empty members each and an opaque private frame, under the 16 MiB metadata and
4096-record ceilings. It still measured **134,553,612 bytes** before refusing,
335,884 bytes (about 328 KiB) above the reservation. The parent now refuses a
second selected ID3 target immediately while scanning, before retaining all the
parsed targets. This unchanged canary passes at **16,828,798 bytes**. Neither the
reservation nor the allocation assertion was raised. Both retained allocation
canaries and inclusive list boundaries pass on the corrected candidate.

## Independent preservation and failure probes

- 160 deterministic randomized FLAC interleavings cover literal separators,
  empty/repeated/Unicode values, multiple selected raw runs, growth/shrink and a
  98,321-byte suffix. A separate raw-record oracle checks Apply. Selective Undo
  restores the entire original byte sequence and reverse-receipt Redo restores
  the complete applied sequence.
- A target at the edge of an unrelated repeated native group restores while the
  unrelated group expands and changes values. A changed interior group anchor
  refuses before writing, preserving the entire current candidate.
- All 30 observed successful-path event positions are independently replaced
  with an IO error and with an unwind: 60 interruptions. Every case releases its
  native resource owners and preserves a separate original. Events before any
  resize/write leave the candidate unchanged. Later failures deliberately may
  leave a torn disposable candidate: it must be discarded, never published.
- A real child process inherits `RLIMIT_FSIZE` through its shell. The actual OS
  returns a positive short write followed by EFBIG while shrinking metadata and
  moving the media suffix. The separate original and resource accounting remain
  intact; the failed disposable candidate actually contains a written prefix.
- A callback changes the candidate suffix and restores mtime immediately before
  mutation. Native change-time validation refuses it without adopting the new
  bytes or making a subsequent write.
- A manual external-reader probe generates real MP3 and M4A audio with FFmpeg,
  seeds qualified ID3 and full-namespace MP4 metadata with Mutagen, and changes
  ordered lists containing leading/trailing empty, Unicode and duplicate
  members. An unrelated UTF16 ID3 album, UFID bytes and a same-name freeform field
  in a different MP4 namespace survive. FFprobe encoded packet SHA256 sequences
  remain identical. Selective restore returns the complete original file bytes.

Mutagen 1.48.1 trims a final empty ID3 list member from its read projection. The
probe therefore also independently parses the complete raw TXXX payload and
requires its trailing delimiter and full ordered values. It explicitly checks
the trimmed Mutagen view rather than treating it as evidence of native loss.
MP4 empty data values are checked directly through Mutagen's native byte lists.
FFmpeg/FFprobe runtime version: n9.0.2.

## Verification and remaining gates

```text
cargo test -p tagrex-core --test native_candidate_patch --test native_candidate_writer_adversarial_review --all-features --locked -- --include-ignored --test-threads=1 --nocapture
# author 19 passed (both manual probes executed); independent 9 passed
cargo clippy -p tagrex-core --test native_candidate_writer_adversarial_review --all-features --no-deps --locked -- -D warnings
# passed
rustfmt --edition 2021 --check crates/core/tests/native_candidate_writer_adversarial_review.rs
# passed
```

The independent suite normally has eight tests and one explicit manual external-
reader test. All nine actually ran above. The outer combined harness was serial
so manual media jobs did not create incidental admission contention; existing
author concurrency cases retain their actual concurrent work. This does not
raise production limits or turn a resource-pressure assertion into an unbounded
happy path. Logs stay outside source under task-owned
`tmp/native-candidate-independent-*.log`. Disposable fixture trees clean themselves
on success and assertion failure.

Reviewed production SHA256:

- native_patch.rs: `f9720b0162234b715ca60a7c13f3f329ce0a49e32fc96dcb2c759412c221d0ce`
- exact.rs: `b546bd85d0e223d8af420df7b44edeefd071b5f027887025c64d882cf2155bb1`
- independent test: `6ec6b52e3cbdb32a36881c964879b579977faa82ef144486681c264c29ecc946`

The native_patch hash includes the parent's child `pub mod codec` declaration.
Codec implementation and its new durable entry points are outside this review;
this hash does not imply their approval.

Tier 2 graph discovery used project `tagrex-backend-readiness`, generation
`2026-11-18T12:48:55Z`, rooted in the correct checkout. Search found the old
PreparedTextPatch symbol; its trace had no edges and the snippet body was visibly
stale. Coverage marks native_patch and tests untracked and exact.rs changed.
Current source was therefore read for scanners, pure native record parsing,
container layout, receipt anchors, mutation/readback, witnesses and capability
validation. No completeness or absence claim rests on the stale graph.

The primitive assumes exclusively controlled disposable Files. Concurrent hostile
writers, native Windows/macOS runtime behavior, network/coarse filesystems,
physical power loss and process-restart receipt authorization are unverified.
Unsupported ID3 encodings/flags and MP4 relocation profiles remain explicit
refusals. The original Executor metadata-preservation canary remains a separate
production integration blocker. The new child codec is outside this review.
