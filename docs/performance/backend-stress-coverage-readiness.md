# Backend stress runner: coverage and evidence contract

Checkpoint ready for a fresh independent review of the runner. This is not a
passing result for the whole backend, hosted CI, native platform functionality or
release readiness. Work reused the existing programme checkout and toolchain.
No Rust production/test source, user media, app runtime, profile or Undo history
was changed in this scope.

Owned deliverables are `tools/backend-stress.py`, its new
`tools/tests/test_backend_stress.py`, `.github/workflows/backend-stress.yml` and
this report. Reusable backend verification is eligible for curated
`tagrex/tagrex:main` contribution once its foundation is compatible. Upstream
inline journal images and the stable waveform interface are unchanged.

## Findings and resulting behavior

**P2, corrected: a positive summary could hide a zero-test requested binary.**
The previous runner accepted any positive `test result: ok` line. A real disposable
Cargo workspace with two requested integration binaries and a filter present
only in the second returns exit zero, with results `[0, 1]`. The old predicate
reports that run green. The retained harness reproduction verifies actual Cargo
output and requires the new parser to refuse it. Final outer summaries are used
per Cargo target, so child-process successes are not counted again or used as a
substitute for an empty parent. Every requested target must have a matching
header, a final successful summary, and at least one passing test.

**P2, corrected: individual case fingerprints could mix source candidates.**
Previously a source change between two individually stable cases was compatible
with success. Every case must now match the initial run fingerprint, and a final
whole-run comparison also runs. Source changes invalidate coverage rather than
merely recording different hashes beside green results. This detects observed
changes; it is not an exclusive source snapshot or an environment/RSS guarantee.

**Coverage gap, corrected:** Unix-only and Linux-only targets need explicit
applicability, and recently reviewed targets were missing from the registry.
The old zero-test guard correctly refused empty binaries, but this made a
Windows default run fail on its entirely Unix source-isolation target without
identifying the actual applicability limit. No failed backend assertion was
converted to a skip. In particular, `untouched-metadata-preservation` remains
functional and applicable on Linux, macOS and Windows. It still fails the runner
until its actual production behavior is corrected.

## Structured coverage contract

`results.json` is schema version 2 and is authoritative for the selected run:

- A case is `verified`, `failed` or `not_applicable`. Nonapplicable cases have
  `passed: false`, `executed_tests: false`, a reason, and no command or log.
- Every applicable case records expected/observed Cargo target counts, outer
  passed/failed/ignored counts, explicit evidence type, referenced log, source
  hashes, and whether it matches the run's initial source.
- Refusal-only tests have evidence type `unsupported_contract`; their successful
  count is separate from `functional` cases. They verify refusal, not protected
  publication on an unsupported platform.
- A selection containing only nonapplicable cases returns nonzero with
  `no_applicable_cases`. A successful mixed run reports
  `applicable_cases_verified`; it does not report whole-backend readiness.
- The global `native_platform_feature_support_verified`, physical-device and
  power-loss fields remain false. A passing applicable target can still contain
  narrower per-test cfgs; the output explicitly states this limit.
- Missing required tools and Mutagen for the selected independent media cases
  remain hard failures. Active cases retain the 600-second subprocess deadline,
  owned subtree termination, per-case logs and source fingerprints. No quota or
  deadline was raised to produce success.

The registry now has **48 cases**: the 33 existing cases and 15 additional groups.
New groups cover held reads, compatibility expansion, native candidate writers,
receipt codecs, copy checkpoints, move staging/cleanup/profile, Unix U2 recovery,
protected media/editor/parent operations and explicit unsupported-platform
contracts. Manual native writer/receipt/streaming and move profile probes are
included deliberately rather than silently left ignored. Existing portable
cases retain their original status and commands.

Applicability based on current source:

| Host | Applicable cases | Explicitly nonapplicable cases |
| --- | ---: | --- |
| Linux | 46 | Two non-Linux protected-store refusal contracts |
| macOS | 45 | Three Linux protected-media/editor/parent functional groups |
| Windows | 42 | Those three Linux groups plus exact source-isolation, Unix cleanup-index and Unix U2 groups |

The non-Linux contracts execute the actual
`non_linux_store_remains_an_explicit_opt_out` and
`unsupported_platform_refuses_the_opt_in_store_without_publication` tests.
Linux held-parent syscall tracing remains additionally x86_64-specific. This
static registration table is not evidence that macOS or Windows tests ran here.

## Executed verification

Using the reused dev environment on Linux:

```sh
source /mnt/Games/Development/tagrex/dev-env.sh
PYTHONDONTWRITEBYTECODE=1 python3 -m unittest discover -s tools/tests -p 'test_backend_stress.py' -v
```

**16 tests passed, 0 failed, 0 skipped.** Coverage includes:

- Actual Cargo `[0,1]` grouped-target reproduction in a disposable dependency-free
  workspace, then a real runner CLI invocation of both populated fixture targets
  with unchanged source and verified `[1,1]` counts. This is a runner fixture,
  not a TagRex metadata or playback result.
- A separate actual CLI/fake-Cargo fixture that checks the written result schema.
- Empty targets, absent summaries/targets, child summaries, Windows-style Cargo
  paths and ANSI color, failed processes, required-tool refusal, source changes
  during/between cases, nonapplicable selections and separate refusal counters.
- Actual Linux parent/descendant timeout: the process group is killed, the owned
  adopted descendant is reaped, and the previous subreaper setting is restored.
  Windows tree termination and macOS process behavior remain native gates.

Python AST parsing, YAML parsing/structure assertions and scoped
`git diff --check` also passed. The workflow runs the runner contract tests after
Rust toolchain setup and before the backend cases; its path filter includes the
new tests. No hosted workflow was run or claimed passing by this worker.

One actual programme invocation was attempted:

```sh
python3 tools/backend-stress.py --case structured-report-independent --output /mnt/Games/Development/tagrex/tmp/stress-contract-real-report
```

It correctly recorded **failed/unverified, zero executed targets, exit 101**:
the concurrent integration worker's temporary `decode_intent` implementation had
not yet converted an `ExactReadError` into `native_patch::Error`. No Rust file
was changed to make this harness run pass, and this log is not a new independent
backend defect claim. The full current-programme runner remains unverified
while that work is active. No full default suite or native macOS/Windows run was
attempted here.

Compact evidence is retained under `/mnt/Games/Development/tagrex/tmp/`:
`stress-contract-tests.log`, `stress-contract-real-run.log`, and
`stress-contract-real-report/{results.json,structured-report-independent.log}`.
Disposable CLI/Cargo directories, their build targets and owned process fixtures
were removed by their guards. No installed tools, shared dependencies or build
cache were removed.

Graph Tier 2 checked project `tagrex-backend-readiness`, generation
`2026-11-18T12:48:55Z`, correct programme root. Exact-path checks covered the runner,
workflow and 50 Rust test-source paths used for case applicability. The tools
subtree is excluded; most newer test files are untracked and other sources have
changed metadata. Actual current file/cfg source was read instead of treating
that graph as current coverage. No exhaustive structural-code claim is made.

## Frozen source hashes

```text
41c31737dd0328cc143e9caffb74b0ea7ddeaf32e53c5f4c8e2cb4ee30496d85  tools/backend-stress.py
a5d6d43c5912b4c294eb942c7b8f4c63470155b40d798c35b992c7cb6237e8f3  tools/tests/test_backend_stress.py
9e67748dc088c687a39e51e024d05c359c68c0fc86e06473a9f4700ee70a5d59  .github/workflows/backend-stress.yml
```

New independent review is pending. Future Cargo output formats fail closed when
headers/summaries cannot be established. The registry provides explicit selected
coverage, not automatic discovery of every backend test or proof of all native
filesystems, large-library RSS, physical devices or power loss.
