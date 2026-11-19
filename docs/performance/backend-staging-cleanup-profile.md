# Indexed move-staging cleanup checkpoint

Scope: Unix directory-authority retirement in `plan_paths.rs`, test-only native
phase accounting, and real SQLite profiling fixtures. Reusable standalone backend
route: curated `tagrex/tagrex:main`. Fork storage compatibility and upstream
inline images are preserved. No UI, live app/profile/media, commit or PR changes.
Fresh independent index-authority review is required before integration.

## Finding and resulting change

Every successful private-directory removal scanned **all** remembered directory
identities and canonical aliases with `retain`. A batch spread over N folders
therefore did approximately N full-ledger scans. Undo also holds aliases for old,
now-absent reservations, so the repeated alias scan affected flat Undo too.

Native exclusive timing exposed the cost: before the change, 1,000-file fan-out
cleanup consumed about 0.66–0.81 s, against about 0.31–0.50 s for the flat layout.
SQL intent commits still consumed about 0.8 s. The measurements did not support
assuming that fsync alone explained the remaining time.

The authoritative hash maps and their native identity comparisons are unchanged.
Unix now keeps an ordered secondary directory-path set and a reverse canonical
alias index. Successful retirement visits the selected component subtree and its
aliases, rather than unrelated folders. Directory fallback-pin accounting is
updated for precisely those removed records. Unix `rmdir`, directory syncs,
no-follow traversal, native ownership checks, journal ordering and all admission
limits remain in place. Windows handle retirement is unchanged.

The indexes add O(path-count) metadata under the existing directory/alias caps;
this is not an RSS certification or 100,000-folder readiness claim. Source and
private directory identities still undergo the same validation. The transfer
protocol, reservation naming and persisted journal format are unchanged.

SQL batching was explored as a possible next step, but no unused batching API is
included. Every profiled 1,000-file Apply/Undo still performs 1,000 ordinary intent
transactions. Existing adapters retain their prepare/capture interruption order.
Shared private directories and intent batching remain separate work after review.

## Measurement conditions

All writes used disposable fixtures under the existing task temporary directory.
The public profile uses 1,000 minimal PCM WAV files; native component profiling
uses 1,000 slightly larger PCM WAV fixtures. The large-folder probe uses the
existing 5,000-file FLAC fixture. These workloads must not be compared as though
their absolute times were interchangeable.

`data-advised` syncs each fixture source and requests Linux
`POSIX_FADV_DONTNEED` before timing. It is an advisory data-cache eviction attempt,
**not a guaranteed cold filesystem run**: metadata caches, inline data, OS policy
and concurrent machine activity remain uncontrolled. `warm-repeat` reuses the
same files and journal after full readback. No global caches were dropped and no
runtime or system settings were changed. Warm runs are not assumed faster.

Both profiling paths assert unchanged source bytes, absent owned destinations,
retired history, removed created output folders and fewer than 64 progress-observed
extra descriptors. The native profile additionally checks all 1,000 capture and
1,000 publication scopes executed. There is no throughput pass target.

An intermediate index incorrectly recorded a parent instead of the inserted
child. The index-coherence test failed; that code was corrected before checkpoint.
**Its measurements were discarded**, even though ordinary readback had passed.
Only the corrected `bdd4d844…` index snapshot is accepted below.

## Public persistent-journal profile

Seconds, one paired run per layout/cache label. Intent SQL performs 1,000 calls
in every cell; it remains roughly 0.80–0.99 s before and 0.81–0.83 s after.

| Layout / cache | Apply before | Apply after | Undo before | Undo after |
| --- | ---: | ---: | ---: | ---: |
| Flat / data-advised | 3.264 | 3.179 | 3.631 | 3.109 |
| Flat / warm-repeat | 3.471 | 3.138 | 3.335 | 3.067 |
| Fan-out / data-advised | 3.523 | 3.206 | 3.499 | 3.127 |
| Fan-out / warm-repeat | 3.468 | 3.221 | 4.096 | 3.142 |

Every row observed four extra callback descriptors and restored all 1,000 files.
These synthetic local observations are not production SLOs or a controlled claim
that the total-time percentage will repeat on other hardware or cache states.

## Native component accounting

Test-only RAII scopes record **exclusive** durations: nested proof durations are
subtracted from their parent scope. The release build has no timing collector,
environment hook or observer. This is source-level accounting, not syscall tracing.
Reservation covers mkdir/seal/initial directory sync; capture/publication include
their directory-relative boundaries and syncs; proof covers native identity and
directory-seal checks; cleanup includes retirement, empty-directory removal and
sync, plus existence checks for already-absent reservations. Unclassified time
includes metadata reads, preflight, other authority traversal and remaining work.

Corrected fan-out cleanup compared with the instrumented baseline, milliseconds:

| Cache / operation | Before | After |
| --- | ---: | ---: |
| Data-advised Apply | 658.6 | 328.1 |
| Data-advised Undo | 812.3 | 331.2 |
| Warm Apply | 754.2 | 331.0 |
| Warm Undo | 817.9 | 329.4 |

Full corrected exclusive component accounting, milliseconds:

| Layout/cache/operation | Intent SQL | Other SQL | Reservation | Capture | Publication | Proof | Cleanup | Unclassified |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| Flat/data/Apply | 841.3 | 13.9 | 408.9 | 501.3 | 464.5 | 272.5 | 325.3 | 426.3 |
| Flat/data/Undo | 855.4 | 23.1 | 413.6 | 507.5 | 476.8 | 254.8 | 331.2 | 327.6 |
| Flat/warm/Apply | 836.9 | 13.8 | 415.9 | 496.4 | 471.2 | 271.3 | 325.6 | 372.3 |
| Flat/warm/Undo | 808.6 | 22.9 | 394.0 | 488.4 | 456.2 | 246.4 | 325.0 | 325.1 |
| Fan-out/data/Apply | 824.4 | 14.5 | 410.7 | 487.4 | 463.2 | 271.2 | 328.1 | 436.8 |
| Fan-out/data/Undo | 826.4 | 23.1 | 398.3 | 484.7 | 454.0 | 251.9 | 331.2 | 363.2 |
| Fan-out/warm/Apply | 834.2 | 14.1 | 415.9 | 500.4 | 465.6 | 278.9 | 331.0 | 412.7 |
| Fan-out/warm/Undo | 816.4 | 22.9 | 398.2 | 482.9 | 453.4 | 248.7 | 329.4 | 357.6 |

The baseline instrumented source was plan `aaf989f5…`, journal `65ea0702…` and
unchanged prior paths `153c839b…`. Corrected measurements bind the full hashes
below. The small journal difference restores the original single-intent method
format after discarding the unconsumed batching experiment; transaction semantics
and intent counts are unchanged. Instrumented baseline and corrected profiles
both use the same phase collector and unchanged payload operations.

## Large-folder result and preservation

The corrected 5,000-folder/5,000-file FLAC probe passed at **16.97 s Apply /
17.13 s Undo**, versus the prior U2 run's **26.00 s / 29.95 s**. All original bytes,
owned target absence, output-folder cleanup and history assertions passed; four
extra callback descriptors were observed. This demonstrates removal of the
measured scaling cost, not completion of the overall throughput goal. SQL and
filesystem durability costs remain material; native platform and network
filesystem performance remain unverified.

Two new invariant tests verify that ordered component-prefix ranges equal a full
scan over tricky and Unicode paths, and that both secondary indexes equal the
authoritative maps before and after retirement. The filesystem test preserves an
unrelated directory's retained pin and rejects its later symlink redirection even
when the file identity is the same through a hardlink. This makes authority
preservation observable independently of ordinary byte readback.

All retained U2/context/staging/authority/adversarial tests are unchanged. The
scoped gate also runs the real process-death and cancellation/storage-fault
probes. The separately confirmed unfinished-copy case still preserves source,
copy and history and refuses `Stale` without a durable hash; this optimization
neither invents copy ownership nor claims that broader gap is fixed. Metadata
writer preservation, actual power loss and native Windows/macOS remain separate
gates.

## Fresh verification and source snapshot

```sh
cargo test -p tagrex-core --lib --test executor --test sqlite_journal --test recovery_stress --test recovery_authority_final_review --test recovery_durable_recheck --test recovery_adversarial --test move_staging_adversarial --test move_context_recheck --test unix_u2_recovery_recheck --quiet
cargo clippy -p tagrex-core --tests -- -D warnings
cargo test -p tagrex-core --lib profile_staging_native_components -- --ignored --nocapture
cargo test -p tagrex-core --test move_staging_profile -- --ignored --nocapture
cargo test -p tagrex-core --test recovery_stress mutation_authority_five_thousand -- --ignored --nocapture
cargo test -p tagrex-core --test recovery_stress ffprobe_verifies -- --ignored --nocapture
cargo test -p tagrex-commands --lib each_files_sidecars_come_from_its_own_folder --quiet
cargo check -p tagrex-service
cargo fmt --check -p tagrex-core
git diff --check
```

The scoped unit and nine integration suites passed **416 tests, zero failed,
four ignored** (275 + 48 + 8 + 6 + 6 + 6 + 6 + 25 + 29 + 7). Manual native/public
profiles, the large-folder probe and independent FFprobe metadata/encoded-packet
probe explicitly passed. The unchanged command sidecar workflow passed 1/1.
Core all-test Clippy denied warnings, core formatting, whitespace and service
compilation passed. No full-workspace release gate or new independent review
approval is claimed.

| File | SHA-256 |
| --- | --- |
| `crates/core/src/plan.rs` | `aaf989f50917f937ca6aaacc9a51838a88d7ce1db7bffd49e0db8266a3791516` |
| `crates/core/src/journal.rs` | `41e78d4a7b8a322e1eb549687b9c6035bc61d008c0bb4334586c269c2c943056` |
| `crates/core/src/plan_paths.rs` | `bdd4d844b6c3f53a5638aa23c82eaa8f4e9648cd0a9f5ccc658f3956a6976544` |
| `crates/core/src/plan_move_metrics.rs` | `cc76d9a567297aa455f6dbcba59b3ef1970e6f3373facd402959099a1dc6dc65` |
| `crates/core/src/plan_move_staging_tests.rs` | `9babc302b4f3e100099950d8a34ac64c08f6d82f661f86b9baf4e8c77bf75fd9` |
| `crates/core/tests/move_staging_profile.rs` | `0ddd16c97637041044fdd51229b924b1dbbe0a7b79fafae8d5e4c471b10a8aaa` |

Tier 2 coverage remains project `tagrex-backend-readiness`, generation
2026-11-18T12:48:55Z; the edited paths are stale and profile sources untracked.
Current source, not stale snippets or a no-match graph result, supplied the
retirement/validation/SQL evidence. Task-owned fixture trees were cleaned by the
tests. No strace or other tool was installed, and no machine-wide cache action
was performed.
