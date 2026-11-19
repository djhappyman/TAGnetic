# Independent native receipt codec recheck

**No further confirmed blocker in the reviewed codec scope after three P2
corrections.** This review covers bounded receipt decoding, native record
constraints, ownership and semantic-policy rebinding on Linux. It does not enable
or approve production Executor/SQLite/stage integration, original-media writes,
native Windows/macOS execution, or the whole backend roadmap.

Only `crates/core/tests/native_receipt_codec_adversarial.rs` and this report were
authored. The parent made the production corrections. No live app, profile, media,
commit or PR was changed. Reusable corrections are candidates for curated
`tagrex/tagrex:main`; fork storage compatibility and upstream inline journal
images remain separate integration requirements.

## Findings and retained corrections

**P2: declared raw-node capacity allocated before sufficient wire evidence.** A
checksum-valid 16,601-byte envelope declared 4096 zero-length raw nodes. The old
4-byte minimum admitted the collection count and allocated the complete Node
vector before refusing its first record. An isolated observational System
allocator measured **360,641 bytes** against **330,448 bytes** of admitted work.
The retained normal canary now measures **193 bytes** before refusal. Counts use
the minimum valid native record, including its wire length; slot counts include
a valid nonempty run. The budget and canary assertions were not raised.

**P2: impossible native target cardinality passed decoding.** Independently
constructed ID3 and MP4 envelopes had matching semantic states, target digests,
contexts and integrity hashes, but encoded an old `[A, B]` target as two TIT2
frames or two identical `©nam` items. Their admitted native profiles require one
frame/item carrying the list. Before correction the codec accepted both; public
selective restore changed the disposable candidate and then returned `Ambiguous`
on readback. This is invalid-history acceptance and a candidate effect, not a
claim of published-original loss. Decoding now refuses non-FLAC multiple runs,
multiple records in a run and multiple after-target records before allocating
those vectors. Both original refusal canaries pass unchanged. FLAC repeated
records and historical absence remain supported.

**P2: decoded outer-vector capacity escaped retained ownership.** A valid FLAC
receipt with 1024 empty after-values decoded successfully. Actual retained
allocation was **125,204 bytes**, but its transferred lease charged only
**59,981 bytes**. In-place collection can reuse the much larger allocation of a
`Vec<Node>` for a `Vec<Vec<u8>>`; counting only inner raw bytes misses it. The
parent now counts actual outer after-record capacity and each old run's node
capacity in both codec and candidate receipt estimates. The same canary retains
125,204 bytes under a **150,165-byte** charge; its peak is 157,988 bytes under a
363,376-byte work reservation. No retention ceiling was increased.

## Additional independent probes

Ten new normal tests include the three correction families and:

- Qualified English COMM versus German COMM/private frames, and insertion of an
  absent Unicode-described TXXX with repeated/trailing-empty values. Encoded
  receipts restore complete original bytes and reverse receipts reproduce the
  exact applied candidate.
- MP4 full freeform namespaces, UTF16 data, nonzero locales, repeated/empty
  values and insertion into an absent independent namespace. Full-byte
  restoration/Redo preserves the other namespace and native item identities.
- Changed semantic separator policy refuses rebinding and restoration without
  candidate effects. Restoring identical semantics under a new generation then
  explicitly rebinding permits exact restoration.
- Fourteen checksum-valid structural refusals exercise context enums/flags,
  oversized counts and invalid/non-owning native records. Six typed Vorbis
  targets cannot enter through the reverse-receipt capability exception.
- Busy job admission and TooLarge work admission retain no partial owner.
  Retained pressure fills a bounded coordinator, then 24 repeated refusals keep
  allocation and charge exactly stable. Releasing one owner permits a retry;
  releasing all owners restores baseline usage.

The System allocator delegates unchanged to the platform allocator and only
counts successful allocation sizes. Allocation probes run alone in child test
processes; functional cases also use isolated process coordinators. Fixture
setup, input envelopes and existing owners precede the measured intervals. This
is evidence about these owned allocations, not an RSS or allocator-overhead
ceiling. All paths are disposable and clean themselves on failure and success.

## Verification

```text
cargo test -p tagrex-core --test native_receipt_codec_adversarial --test native_receipt_codec --locked -- --include-ignored --nocapture
# 10 independent + 8 author = 18 passed; 0 failed; 0 ignored
cargo clippy -p tagrex-core --test native_receipt_codec_adversarial --all-features --no-deps --locked -- -D warnings
# passed
rustfmt --edition 2021 --check crates/core/tests/native_receipt_codec_adversarial.rs
# passed
```

The combined run explicitly executes the author's real FFmpeg MP3/M4A and
FFprobe encoded-packet hash probe. That run also exercises 746 checksum-valid
mutations and a fresh-process policy-generation restore. Independent tests do
not equate display-reader normalization with exact native state. Logs remain in
the task-owned external `tmp/native-codec-independent-{final,allocation,clippy}.log`.
No owned fixture directories remain.

Reviewed SHA-256:

- codec.rs: `7b2d89b3c24c0bc2a1d147e85b16aea0482108b0e64d86ea64ab62f65960d4d2`
- native_patch.rs: `93abbd0a2aeb8cdf2d92dae372e22d915f50789d86c27aa85fe5d92f3217d883`
- exact.rs: `b546bd85d0e223d8af420df7b44edeefd071b5f027887025c64d882cf2155bb1`
- independent test: `23c2e4b5d513f79ea0ebd87c01c266a11a4d6ecae39dfd41a3c75e3be7192bed`

Tier 2 discovery selected `tagrex-backend-readiness`, generation
2026-11-18T12:48:55Z, with the correct checkout root. Search found only the older
PreparedTextPatch struct; its trace had no edges and its snippet bounds were
visibly stale. Coverage marks codec/native candidate/tests untracked and exact
changed. Current source was therefore read for all material decoder, parser,
capability, restore, resource and policy behavior. Fixture binary coverage is
not graph evidence. No absence or exhaustive architecture claim uses this graph.

A checksum does not authenticate deliberately modified journal contents. Trusted
plan/batch/path/native-source binding, whole-collection refusal before earlier
recovery effects, schema compatibility and protected publication are still
required before these decoded receipts can authorize original-media Undo.
