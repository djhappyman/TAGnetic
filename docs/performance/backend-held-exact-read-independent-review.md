# Held-file exact reader: independent review

Checkpoint ready for this read-only seam, after one confirmed P2 was fixed by the orchestrator. This is not Executor, journal, write, or whole-backend readiness.

## Scope and route

Reviewed `TagEngine::read_exact_file[_with_cancel]`, shared `read_exact_using`, `exact::prepare_input_file`, `raw::Input::{prepare_file,prepare_source,ensure_unchanged}`, and thread-local compatibility source isolation. The reviewer owns only `crates/core/tests/held_exact_read_independent.rs` and this report; production fixes belong to the orchestrator. No commits, app/runtime changes, live-media writes, or Undo writes were performed.

This is shared backend behavior eligible for a curated `tagrex/tagrex:main` contribution once its required foundation is compatible. Preserve upstream inline journal images and the stable waveform API. Current evidence applies to the uncommitted programme checkout, not a published PR.

Graph verification used Tier 2, project `tagrex-backend-readiness`, generation `2026-11-18T12:48:55Z`, confirmed root `/mnt/Games/Development/tagrex/native-field-foundations`. Parent discovery/trace covered the earlier exact-reader pipeline. Reviewer checked exact-path coverage: model/exact/raw and existing exact tests have changed metadata; source scope and held tests are untracked. Actual current source and scoped tests were read directly. The graph cannot certify the newly added API by itself.

## Ranked finding and correction

**P2, fixed: final cooperative callback ran after publication witnesses.** The last `cancelled()` callback could modify the held file or read policy and return false after the final source/policy checks. A successful read then published the earlier snapshot despite an observed in-request mutation. Independent probes overwrote a same-length FLAC Title (`Before` → `After!`) or changed the multi-value separator specifically at the final callback. Both returned `Ok` before the fix. They are unchanged retained regression tests and now return `ExactReadError::Changed`.

The orchestrator moved the final callback before the final policy and held-source checks. No caller callback runs after those checks. The named path API shares this correction and has a separate final-callback source-mutation regression. No unresolved defect was found within the bounded review scope.

## Executed checks

On Linux, using the existing `dev-env.sh` toolchain/cache:

```sh
cargo test -p tagrex-core --test held_exact_read_independent --test held_exact_read --test exact_read --test exact_read_adversarial --test exact_source_authority_fresh_review --locked
cargo clippy -p tagrex-core --lib --test held_exact_read_independent --all-features --locked -- -D warnings
rustfmt --edition 2021 --check crates/core/tests/held_exact_read_independent.rs
```

Results: **44 passed, 0 failed, 2 existing manual tests ignored** across five integration binaries; scoped all-features Clippy and owned formatting passed. The independent binary contributes **8 tests**, with none ignored. Initial independent run after correcting test-lock poison recovery was **3 passed, 2 failed**; the two failures were the unchanged final-callback probes described above.

Independent coverage includes:

- FLAC, ID3v2.4/MP3, and the repository's real AAC-in-MP4 fixture: exact entries, compatibility tags, format, and tag-block selection match a named baseline when the held descriptor starts near EOF and its logical label is absent or points at foreign FLAC data.
- Unix unlink of the held source followed by foreign replacement at the same name: reads continue using the original held bytes, and replacement bytes remain unchanged.
- Changing the foreign logical label at every callback, including the last, and reading that label through a nested public legacy request: the held reader still returns its own original metadata.
- Nested held requests with the same label, public legacy requests, and another thread: each reads its own authority, restoring the outer request afterward.
- Ninety-six alternating success, malformed-source error, cancellation, and unwind operations: coordinator ownership returns to baseline each iteration and Linux `/proc/self/fd` count is unchanged.
- Final-callback source mutation, policy mutation, and named-reader mutation are refused without undoing or overwriting the external change.

Evidence: `tmp/backend-roadmap-query-art-gate/held-independent.log` (red), `held-independent-final.log` (green), and `held-independent-clippy.log` under `/mnt/Games/Development/tagrex`. Disposable fixture directories were removed by their owning guards, including unwind paths.

## Final reviewed hashes

| File | SHA-256 |
| --- | --- |
| `crates/core/src/model.rs` | `660e30bf41e990415ae3db7e7661d1f75326f1cf61d172a716530ccc9c5790c8` |
| `crates/core/src/model/exact.rs` | `71527bccf5c557b4936d0579bc7fc224315871c4f93e600300c68a9ba1e8faf8` |
| `crates/core/src/model/exact/raw.rs` | `6c134a3df73ba22e671b6d812ba488d73b94d32bb36d228bff4be6e96ef18f3d` |
| `crates/core/src/model/exact/source.rs` | `599a1c270ea49576d90236e0e97778728f759c19246de1a1fd80641d60b4e743` |
| `crates/core/tests/held_exact_read_independent.rs` | `e66b6a7aa889991a3a43b2c415396bf920cf75fe2a11abb2133d0b076a70b97c` |

## Limits

The logical label is not filesystem authorization. The caller must bind its held file to approved roots/path and revalidate before mutation. Cloned file handles can share seek cursors; concurrent access through caller-owned clones remains excluded by the API contract. The returned snapshot has no file handle and does not authorize a write. Stamp checks are observations, not exclusion of another writer after the last check.

These are Linux results; native Windows/macOS execution was not performed here. Unix unlink and Linux descriptor checks are intentionally platform-specific. Existing optional scale/independent decoder tests were not rerun as part of this seam review. Separate exact-reader expansion review owns allocator stress; these checks do not establish a whole-process RSS guarantee. Existing parser/syscall work is synchronous and cannot be interrupted internally. Executor lossless publication, persistent journal integration, cross-process recovery, and the known ordinary metadata-preservation issue remain separate work.
