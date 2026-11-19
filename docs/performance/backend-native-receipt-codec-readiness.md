# Durable native text receipt codec

Locally implemented and independently rechecked in the bounded codec scope. This prepares the
selective writer's evidence for journal integration. It does not enable exact
Apply, migrate the shared stage, or fix the unchanged Executor preservation
regression. Reusable backend route: curated `tagrex/tagrex:main`; fork storage
compatibility and upstream inline journal images remain separate requirements.

`Receipt::encode_for_journal` produces an owned, leased byte envelope.
`decode_for_journal` accepts bounded historical evidence, not caller write
authorization. Both have explicit-budget variants. The envelope has a versioned
magic, exact payload length and SHA-256 integrity. Decoding rejects trailing data,
unknown enum tags, invalid UTF-8/identity, malformed contexts, unreasonable
counts, inconsistent states and incorrect raw-target digests. Counts also fit
the remaining minimum encoded collection size before vector allocation.

Only raw target records and native identity anchors are persisted. The shared
native parser derives each record's identity, selection and values. Every record
must actually select the declared target, and their aggregate values/digests must
match the receipt. Serialized Node flags/keys/values are never accepted. The
shared text capability gate must admit at least one direction, because a reverse
receipt may restore an old native fourcc that a fresh user intent cannot create.
Typed/read-only targets remain refused in both directions.

`rebind_current_policy` samples the current process generation only after checking
unchanged semantic policy. This allows a historical receipt to resume after a
restart even when identical settings have a different revision number. A changed
semantic policy still refuses. Actual restore independently checks the native
candidate, exact current target bytes/state and source witnesses. The Executor
must additionally bind decoded evidence to its entire trusted plan, source native
identity, roots, batch and file position before any live mutation.

The payload is limited to 16 MiB, 4096 raw records/runs, 1024 values per state,
1 MiB of text per state and bounded native names/policy. Decode reserves transient
work before ownership expansion, then transfers to a retained lease after strict
validation; the retained receipt limit is 16 MiB. Busy/oversized requests refuse
without leaking owners. These are participating-resource limits, not RSS proof.
A checksum detects damage; it does not authenticate modified journal contents.

## Local evidence

```text
cargo test -p tagrex-core --test native_receipt_codec --locked -- --include-ignored --nocapture
8 passed; 0 failed; 0 ignored
cargo clippy -p tagrex-core --lib --test native_receipt_codec --all-features --no-deps --locked -- -D warnings
passed
```

The probes cover complete raw FLAC restore and reverse-receipt Redo, every
truncation and single-byte corruption, 710 checksum-valid mutations with no panic
or retained charge, mismatched semantic/raw authority, trailing bytes, resource
pressure and successful retry. Structurally valid mutations remain historical
evidence; their acceptance is not permission to restore an arbitrary file.

A real child process reads a saved receipt after reconstructing identical policy
with a different generation and restores the disposable candidate. This is codec
persistence evidence, not SQLite/Executor restart recovery. FFmpeg-generated MP3
and M4A exercise native ID3/fourcc envelopes, empty/repeated values, full-byte
Undo and reverse-receipt Redo. FFprobe packet SHA-256 output stays identical
through edits and restoration. The external-reader probe was explicitly run.

Functional fixture cases serialize their use of the process coordinator; they do
not assume unlimited jobs from the test harness. Pressure tests use a separate
bounded coordinator and check refusal/retry/release. Native writer concurrency
and allocation are challenged separately by its independent review.

A separate [ten-test review](backend-native-receipt-codec-independent-review.md)
found and corrected three P2 classes: allocation from impossible declared record
counts; duplicate ID3/MP4 target collections that failed only after candidate
mutation; and missing retained vector-capacity ownership. Count checks now use
minimum valid native encodings, non-Vorbis historical target collections enforce
single-record cardinality, and both candidate and decoded receipt accounting
include outer raw/Node vector capacities. The retained maximum-empty-vector
probe measures 125,204 bytes against a 150,165-byte charge. All ten independent
and eight author probes pass, including qualified ID3, namespace/UTF16/locale MP4,
changed-policy refusal, generation rebinding and pressure/release/retry cases.

Pending: protected journal/Executor integration, old/future schema barriers and
native platform execution. No live media, app/profile, commit or PR changed.
