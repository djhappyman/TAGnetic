# Independent created-directory path oracle review

Status: the scoped oracle/private-fixture corrections and three-line CI diagnostic
change are accepted for integration. No production change or Windows runtime acceptance
is claimed; all required exact-head hosted checks remain necessary before merge.

## Diagnosis and scope

Windows job `135151219241` progressed beyond the earlier fixture failures and
passed the original App policy canary. Its Executor target then reported 47 pass,
one fail at `executor.rs:829` in
`rename_into_new_folders_creates_them_and_undo_removes_them`. The failure was the
lexical assertion that each returned `created_dirs` path starts with the raw
temporary-directory spelling. Evidence is retained in
`/mnt/Games/Development/tagrex/tmp/flac-ci-docs-windows-a9987bb.log`.

**P2 test-oracle portability defect:** the assertion compared caller spelling
with resolved receipt paths. The public Executor canonicalizes allowed roots;
held `create_all` resolves the requested parent and records directories actually
created by the authority walk. On Windows, Rust's
[canonicalize API](https://doc.rust-lang.org/std/fs/fn.canonicalize.html) produces
extended-length paths. Equivalent existing directories need not have the same
lexical prefix as the original input.

The `AppliedBatch::created_dirs` documentation describes actual created cleanup
paths, in shallow-to-deep order. It does not promise the caller's original
spelling. The bounded consumer audit found:

- Undo removes recorded directories deepest first after containment validation.
- Failed Apply rollback uses the recorded directories for owned cleanup.
- SQLite stores and reloads these paths as outcomes.
- The command-layer `BatchDto` does not expose `created_dirs` to the history UI.

No production receipt-contract defect was established by this review. Changing
production path authority to satisfy this lexical assertion would be unjustified.

## Corrected oracle

Only this test's containment assertion changes: it canonicalizes the expected
root and each recorded existing directory before comparing containment. This
also requires every receipt entry to resolve successfully.

The shared `TempDir`, raw public roots, plan paths and API calls remain unchanged.
The test still requires exactly two created directories, both moved files at
their destinations, restoration of the original file, removal of the newly
created tree on Undo and preservation of the pre-existing folder. The neighboring
root-escape and rollback tests are unchanged. No test ignore or platform gate was
added, and no new test file was needed.

The author's Linux symlink-TMPDIR probe reproduces the original lexical assertion
failure without changing public inputs, then passes after the oracle correction.
Its retained evidence is `flac-executor-containment-{red,green}.log`.
Independently, all 48 Executor cases also pass under a fresh, reviewer-owned
symlink TMPDIR. That verifies path-name resolution behavior on Linux; it is not
Windows execution evidence.

## Unix private-scope fixture

The direct-scope inventory also identified the Unix-only
`cleanup_index_tests::retirement_keeps_unrelated_alias_authority_and_fallback_pins`
fixture. Unlike public Executor admission, it entered the private scope with a
raw temporary root. Under the author's controlled symlink TMPDIR, the unchanged
fixture failed at private Scope entry with `NotADirectory`; its retained red is
`flac-cleanup-fixture-alias-red.log`.

The correction adds only a comment and root canonicalization immediately after
creating that owned directory, before deriving test paths. It does not change a
production method or the public raw-input fixture above. Every index-consistency,
retained-handle, exact fallback-pin count, unrelated alias, same-inode redirection
refusal and preserved-file assertion remains. In particular, the fixture still
replaces `aa` with a symlink after scope capture and requires both entry access
and final validation to refuse it. Independently, both cleanup-index unit tests
pass under another reviewer-owned symlink TMPDIR after this two-line change.

## CI diagnostic change

The exact `.github/workflows/ci.yml` diff appends `--no-fail-fast` to the existing
`cargo test --workspace` command in the Linux, Windows and macOS Test steps. Those
are the only three workflow lines changed. Package/target selection, test
concurrency, required jobs and failure handling remain unchanged.

The [Cargo command documentation](https://doc.rust-lang.org/cargo/commands/cargo-test.html)
explains that the flag continues to later test executables after an executable
fails; it does not turn a failure into success. Cargo's failing exit status
remains nonzero. Each workflow step still invokes Cargo directly, with no
`continue-on-error`, status-swallowing pipeline or weakened assertion. This
provides the complete failure inventory in a CI run rather than exposing one
failed target per run.

This workflow adjustment is fork CI execution policy, not a backend feature.
Shared backend work retains its separate upstream route to `tagrex/tagrex:main`,
waiting on the compatible foundation and preserving upstream inline images and
its stable waveform API.

## Independent verification

The graph project `tagrex-backend-readiness`, generation
`2026-11-18T12:48:55Z`, indexes the parked source checkout. Publication Executor,
authority, journal and command paths report `outside_project`; review uses actual
source and diffs. The reviewer owns only this report and made no test, production
or workflow edits.

Frozen SHA-256 values, rechecked after verification:

| File | SHA-256 |
| --- | --- |
| `crates/core/tests/executor.rs` | `e3f5ef30b41ac19a6354d604246e75bc9b3c32b84ac2c2fdf31793623cf097df` |
| `.github/workflows/ci.yml` | `39cad2963c282a010e0eed8aaf75b54b4eb7a8c9ce38bf4379631a3ec73952cd` |
| `crates/core/src/plan_paths.rs` (Unix test setup only) | `d1b79a611ba99fe69bce8ac80cc7159f887478a7e4e2c1be3c04b5497665eb92` |
| Unchanged `crates/core/src/plan.rs` | `8cc196f1202bceb2564cce0000da5cae06de8f102617defe2ede8cdfe0eca5be` |
| Unchanged `crates/core/src/journal.rs` | `c29348b001a1b80ba4809a5307da8f36cff012cfcb48a3f78aebb63d0c206fce` |

Using `source /mnt/Games/Development/tagrex/dev-env.sh` on Linux:

| Check | Result |
| --- | --- |
| `cargo test -p tagrex-core --test executor` | 48 passed, 0 failed/ignored, default concurrency |
| Same 48 Executor tests under reviewer-owned symlink TMPDIR | 48 passed, 0 failed/ignored |
| `cargo test -p tagrex-core --lib cleanup_index_tests` under reviewer-owned symlink TMPDIR | 2 passed, 0 failed/ignored |
| `cargo clippy -p tagrex-core --test executor --all-features -- -D warnings` | Passed |
| `cargo clippy -p tagrex-core --lib --all-features -- -D warnings` | Passed |
| `rustfmt --check --edition 2021` on Executor/authority source | Passed |
| `git diff --check` on both test-bearing files and the workflow | Passed |

These are 50 distinct test cases; rerunning the 48 Executor cases under an alias
is a second environment, not 48 additional distinct tests.

Independent logs are `flac-created-directory-independent-tests.log` and
`flac-created-directory-independent-clippy.log` under
`/mnt/Games/Development/tagrex/tmp`, plus
`flac-created-directory-independent-{alias,cleanup-alias,cleanup-clippy}.log`.
No Executor or reviewer-owned alias fixture directory remained after
verification. All writes were disposable; no live library, application profile,
installed runtime or user Undo history was touched.

The workflow change was source-reviewed against Cargo's documented semantics;
this review did not deliberately fail a hosted run. Linux checks and the alias
probe do not replace Windows, macOS, Linux and applicable audit results on the
exact pushed head, nor the combined workspace gate. No broader roadmap work or
new production behavior is accepted by this narrow review.
