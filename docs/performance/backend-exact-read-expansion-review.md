# Exact-reader compatibility expansion review

Status: checkpoint ready for orchestrator integration. All 14 independent
expansion/control tests pass against the corrected source (2026-11-18, Linux).

## Scope and delivery gate

This bounded independent review exercises the public `TagEngine::read_exact`
path on disposable ID3v2.4/MP3, MP4/M4A and FLAC inputs. It owns only
`crates/core/tests/exact_read_expansion_adversarial.rs` and this report. The
orchestrator owns production corrections and integration. No live application,
profile, library, media, commit or PR was changed by this worker.

Completion requires the retained failing fixtures and valid controls to pass
against the final corrected source, source bytes to remain identical, resource
ownership to return to baseline, formatting and scoped Clippy to pass. This is
shared backend work eligible for a curated `tagrex/tagrex:main` contribution;
publication and current-head hosted platform gates belong to the orchestrator.

## Confirmed findings

All sizes below are bytes. The allocator observer delegates unchanged allocation
operations to `System` and measures the maximum simultaneous successful allocation
increase during the public read, after fixture creation. It does not measure RSS,
allocator arena retention, stack memory, native allocator usage or OS page cache.
The process resource budget is observed independently. The original exact reader
reserved 67,108,864 transient bytes plus bounded retained results.

| Input | Before correction | Corrected result |
| --- | --- | --- |
| Six duplicate UTF-8 TIT2 frames, 900,000 NUL-heavy text bytes followed by visible X each; file 5,401,343 | Read refused only after peak 355,206,929 versus budget high-water 67,109,376 | Early refusal, peak 5,400,108 |
| Same six bodies under distinct TIT2/TPE1/TALB/TPE2/TCON/TCOM | Child exceeded 45 seconds and was killed/reaped; observed RSS about 373,612 KiB, not interchangeable with allocation peak | Early refusal, peak 5,400,108; child completes about 0.03 seconds in final run |
| Six UTF-16LE TIT2 bodies, 300,000 NUL code units followed by X; file 3,601,367 | Returned unsupported-native-encoding diagnostic after peak 156,906,609 versus budget high-water 67,113,220 | Early refusal, peak 3,600,132 |
| One MP4 UTF-16BE ©nam/data containing 6 Mi U+0800 code points; file 12,583,714 | Refusal after peak 79,700,752 versus budget high-water 67,109,376 | Early refusal, peak 12,583,154 |
| One ID3 Latin-1 TIT2 with 15 MiB 0xff non-NUL value; file 15,729,928 | Refusal after peak 78,656,875 versus budget high-water 67,109,376 | Early refusal, peak 15,728,693 |

These are P2 reliability/performance defects: eventual native refusal or an
unavailable diagnostic did not bound earlier compatibility-parser allocations.
Distinct-field joining also creates a practical latency hazard. Root cause was
validation after compatibility decoding. The parent added ID3 raw/decoded text
length, list count and aggregate preflight,
plus MP4 text length/count preflight, before Lofty; no transient-budget increase
was needed. Configured code pages use a fixed 512-byte incremental decoder.
UTF-16 ID3 remains diagnostic-only with a conservative decoded expansion cap.

The visible non-NUL suffix is intentional: all-NUL trailing payloads were trimmed
by the compatibility parser and did not reproduce the large UTF-8 expansion.
The retained reproducers were not reduced after failure.

## Controls and bounded negative evidence

- Exact ID3 lists with 1, 1023 and 1024 members preserve empty members and the final
  visible value, including qualified TXXX values. Per-frame 1025 refuses.
- Individually valid TXXX frames totaling 16,383 and 16,384 values succeed; 16,385
  refuses. This tests aggregate admission independently of per-frame admission.
- Aligned UTF-16LE and BE ASCII (many individual zero bytes but no NUL code units)
  and surrogate pairs remain bounded and keep native UnsupportedEncoding
  diagnostics. They are not rejected by counting individual zero bytes.
- Eight supported UTF-8 native fields totaling 8,355,840 value bytes succeed with
  exact field lengths; peak 26,119,913. A 14 MiB opaque PRIV frame alongside a
  readable title succeeds with NonText diagnostics; peak 46,145,900.
- Qualified TXXX NUL-heavy values refused within the budget before the ID3 fix
  (peak 3,979,897); no separate defect was established for that shape. The new
  guard refuses it earlier (peak 1,200,198).
- FLAC with 4096 small CUSTOM comments (~3.75 MB) refuses its oversized native
  state with peak 15,544,769; no expansion defect established by this case.
- A 15 MiB MP4 UTF-8 text payload refused with peak 47,195,480 before the fix;
  this bounded control does not exonerate UTF-16 expansion.
- Configured windows-1252 euro text and Shift-JIS multibyte text decoding to
  1,048,575 bytes succeed across the incremental decoder buffer boundary;
  windows-1252 decoded overflow and a trailing malformed Shift-JIS lead byte
  refuse. Exact decoded characters and byte lengths are independently checked.
- Latin-1 512 KiB of 0xff decodes to exactly 1 MiB U+00FF and succeeds; one
  additional encoded byte refuses the native decoded-field limit.
- Native MP4 UTF-8 exactly 1 MiB succeeds, 1 MiB + 1 refuses; UTF-16 U+0800 data
  decoding to 1,048,575 bytes succeeds and 1,048,578 bytes refuses. Successful
  results retain the exact one-value length.

Each measured call checks exact fixture-byte equality and complete process-budget
release after result drop. Unsupported native formats may produce an explicit
unavailable diagnostic, never a claim that those text entries are editable.
Refusal cases require an error rather than a partial editable snapshot.

## Isolation and measurement limits

Each test runs in its own subprocess with a 45-second deadline. On Linux the child
lowers its own address-space soft limit to at most 1 GiB. Timeout children are
killed and reaped, and the parent owns fixture cleanup even after child failure.
All fixtures are finite, within the reader's 16 MiB metadata cap, and synthesized
under the configured task temporary directory. No unbounded fuzzing was run.

The tests additionally require actual observed allocation peaks to stay below
64 MiB, stricter than transient plus retained admission for these cases. The
budget high-water counter is monotonic across subcases; it is printed as evidence
but cannot enlarge this fixed ceiling based on an earlier retained result.
This suite does not prove a universal reader RSS ceiling, adversarial behavior
for every container, cross-platform OS behavior or whole-backend readiness.

## Discovery and source evidence

Graph project `tagrex-backend-readiness`, generation 2026-11-18T12:48:55Z, matched
the checkout. Exact-read public entry points, `read_selected` and frame-envelope
validation were found and traced. Coverage reported `model.rs`, `exact.rs` and
`exact/raw.rs` changed, and `exact/source.rs` untracked. Stale snippet boundaries
were observed. Current direct source reads, rather than graph absence, established
the compatibility-decoder ordering and preflight behavior. The parent is also
factoring the path and held-File entry points through this same input/parser path.

## Final verification

Commands used the existing toolchain/environment from
`/mnt/Games/Development/tagrex/dev-env.sh`:

```text
cargo test -p tagrex-core --test exact_read_expansion_adversarial --locked -- --nocapture
# 14 passed; 0 failed; 0 ignored; 1.55 seconds
cargo clippy -p tagrex-core --test exact_read_expansion_adversarial --all-features --no-deps --locked -- -D warnings
# exit 0
rustfmt --edition 2021 --check crates/core/tests/exact_read_expansion_adversarial.rs
# exit 0
git diff --check -- crates/core/tests/exact_read_expansion_adversarial.rs docs/performance/backend-exact-read-expansion-review.md
# exit 0
```

The final test log is `/mnt/Games/Development/tagrex/tmp/exact-read-expansion-final.log`;
Clippy evidence is in the adjacent `exact-read-expansion-clippy.log`. Superseded
worker logs and owned disposable fixture trees were removed after the final report
was verified. Existing toolchains, caches, active work and runtime state remain.
No remaining blocker was found within this bounded Linux reader scope.

SHA-256 of the source tested:

| File | SHA-256 |
| --- | --- |
| `crates/core/src/model.rs` | `3503b4dd1c51024d8955cc8c51b74fb2b967562f2d844b999eb4c4da37196169` |
| `crates/core/src/model/exact.rs` | `71527bccf5c557b4936d0579bc7fc224315871c4f93e600300c68a9ba1e8faf8` |
| `crates/core/src/model/exact/raw.rs` | `6c134a3df73ba22e671b6d812ba488d73b94d32bb36d228bff4be6e96ef18f3d` |
| `crates/core/src/model/exact/source.rs` | `599a1c270ea49576d90236e0e97778728f759c19246de1a1fd80641d60b4e743` |
| `crates/core/tests/exact_read_expansion_adversarial.rs` | `6581cd76b257f490a0760e59b6485e1befadcff52f0f9dcfe5cbf28b38a36d2f` |

These hashes identify the reviewed/tested candidate despite concurrent unrelated
work in the shared checkout. A later change to these paths needs impact-based
reverification. The orchestrator still owns combined gates, publication and native
platform evidence; this report does not claim those are complete.
