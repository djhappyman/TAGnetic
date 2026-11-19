# Independent metadata policy review

Status: both confirmed finding groups corrected; retained independent probes
pass, with no remaining defect found in this bounded review. This is a Linux review of the per-App
metadata policy prerequisite for native FLAC transactions, not approval of the
whole backend or UI exposure.

## Scope and method

Review began at `origin/main` checkpoint `67625add` with the curated FLAC
changes uncommitted. The publication base later advanced to `c42a6cea` through
unrelated UI changes; reviewed Rust source hashes were unchanged by that move. Read `docs/WORKFLOW.md` and the Rust,
codebase-memory, safe-audio-testing and verification skills. The graph project
`tagrex-backend-readiness`, generation `2026-11-18T12:48:55Z`, indexes a different
checkout. Coverage checks for every relied-on publication source returned
`outside_project`; conclusions use the actual publication source and executable
public API probes, not a graph completeness claim.

Initial policy source SHA-256 values:

| File | SHA-256 |
| --- | --- |
| `crates/core/src/model.rs` | `0a4bf8ef9d2382884a873fdd77ba44fd1f5de34796259be4e464328808dfada6` |
| `crates/core/src/model/exact.rs` | `58fa6647ef65809438848a1693c26141f8f6cec0b62f2d0aab3af4690cdc3ff9` |
| `crates/core/src/model/policy.rs` | `57ae478134667ec78dcafc635534753e820dab9fd54005130a649bea9655f936` |
| `crates/commands/src/lib.rs` | `a4932f52f694342ca59eaa16176ee697316f5639a81359da8144ab0ce2d1739f` |

The review owns only two new integration test files and this report. Production
corrections belong to the implementation author. No live media, application
runtime, user profile or existing Undo history was used. Fixtures remove their
own temporary directories. Standalone global-setter probes run in child test
processes; existing tests are not serialized or weakened.

## Ranked findings

1. **P2: policy invalidation prevents automatic rollback after a completed owned
   FLAC write.** Public `App::apply_with` with two title edits, then a standalone
   separator change and cancellation at the first completed write, returned
   `RecoveryRequired` while leaving the first file edited. A second public
   `Executor` probe using `ModelPolicy::configure` instead of a global setter
   reproduced it. No media was changed externally. Invalidated current policy
   blocked reading/restoring the operation's own durable prefix. The correction
   must retain current-policy refusal for historical Undo and all source,
   profile, journal, exact-state and root checks.

2. **P2: reads under an invalidated owner populate caches with its pinned old
   interpretation.** Inside `App::with_write_scope`, configure a new separator,
   then call `read_track` or `query_snapshot`. After the outer operation exits,
   a fresh call still returns the old separator. `apply_settings` cleared the
   caches before the pinned old read repopulated them. Separate display-cache,
   query-cache and error-exit probes fail deterministically. Valid-generation
   cache identity/admission must account for the policy, including retained query
   snapshots.

Red evidence is retained outside the source tree in the task log directory:
`/mnt/Games/Development/tagrex/tmp/flac-independent-policy-first.log`,
`flac-independent-policy-expanded.log`, `flac-independent-policy-cache-red.log`,
`flac-independent-policy-query-red.log`, and
`flac-independent-policy-error-red.log`. A separate retained-query validity red
is in `flac-independent-policy-retained-query-red.log`.

The author corrected rollback using a private operation-local interpretation
captured before effects. Only automatic recovery enters its sealed scope;
current-owner invalidation remains sticky afterwards. Public Executor Apply/Undo
refuse a recovery ancestor before any callbacks or journal access, including
legacy adapters and a nested peer policy. Historical Undo continues to require
compatible current interpretation. Display-cache lookup/admission now requires
a matching valid policy generation. Query snapshots built under an invalidated
generation are immediately invalid for workers and are rebuilt at the next
valid operation.

## Retained independent probes

`crates/core/tests/model_policy_independent.rs` covers reentrant A-B-A owners,
non-LIFO final-guard removal, pinned snapshots, no-op normalization, generation
change-and-restore, concurrent owner changes and unwind restoration.

`crates/commands/tests/model_policy_app_independent.rs` covers:

- Two concurrent real App owners with divergent separators, cached reads and
  repeated Apply/Undo/Redo; independent raw Vorbis-comment inspection verifies
  separate values and preservation of a Unicode custom field. Byte comparison
  proves preview does not write and Undo restores the fixture.
- Fresh App/journal reopening under equal semantics but a new numeric revision;
  changed semantics refuse Undo without mutation or history loss, and restoring
  compatible settings permits Undo.
- Standalone setter and owned configuration changes after the first completed
  write, plus cancellation and exact restoration requirements.
- External modification of an already-written title during cancellation: foreign
  bytes remain untouched and unresolved recovery history remains present.
- Same-owner settings changes inside a write scope, fresh-operation resumption,
  display/query cache admission, error/unwind exits and change-and-restore
  invalidation. Retained query Arcs from the invalid operation stay invalid.
- Explicit standalone-versus-App defaults. Existing desktop library/drop open
  paths apply saved settings immediately after opening the App, and provider
  settings remain separate. `App::apply_settings` intentionally does not configure
  unrelated standalone `TagEngine` operations.
- A foreign NativeJournal callback during actual automatic recovery attempts
  public Apply and Undo using both VecJournal and a legacy journal whose methods
  panic if touched, directly and under a nested peer ModelPolicy. All eight
  attempts refuse before callbacks or legacy journal access. A third disposable
  file stays byte-identical, the original owned prefix is restored, recovery
  history retires, and the caller's policy remains invalidated afterwards.

## Verification and limits

Before correction: four core tests pass; rollback, display/query cache,
retained-query validity and error-exit canaries fail as described. Original red
assertions were retained. Final corrected-source verification:

| Check | Result |
| --- | --- |
| `cargo test -p tagrex-core --test model_policy_independent` | 4 passed, 0 failed/ignored |
| `cargo test -p tagrex-commands --test model_policy_app_independent sealed_recovery` | 1 passed, 12 filtered |
| Full freshly compiled App integration binary, default test concurrency | 13 passed, 0 failed/ignored; 0.61 seconds |
| `cargo clippy -p tagrex-core --test model_policy_independent -- -D warnings` | Passed |
| `cargo clippy -p tagrex-commands --test model_policy_app_independent -- -D warnings` | Passed |
| `rustfmt --check --edition 2021` for both owned test files | Passed |

The full App binary command was
`/mnt/Games/Development/tagrex/build-cache/debug/deps/model_policy_app_independent-758568d3b0aed853`,
immediately following Cargo compilation of the final source and unchanged tests.
This avoided competing with the orchestrator's workspace Cargo lock. Final logs
are `flac-independent-policy-final-app.log`,
`flac-independent-policy-final-core.log`, and corresponding `final-*-clippy.log`
under the same task log directory. No admission limits or existing test
concurrency were changed. Tests use shared development dependencies, caches and
toolchain via `source /mnt/Games/Development/tagrex/dev-env.sh`.

Final reviewed production SHA-256 values, rechecked after verification:

| File | SHA-256 |
| --- | --- |
| `crates/core/src/model.rs` | `1e864a3d1940a15e8a7e0f683bcad58927759f27f617e747e95c105857f559ab` |
| `crates/core/src/model/exact.rs` | `dd2130b64a501b5631143cf5cd08cf0f1776e44cec08aaffcba0423806dbaa00` |
| `crates/core/src/model/policy.rs` | `5f0ff1c02b1f2467e40b7c62f1e711a072eb1e1b1258ac83093d39f7d172f11e` |
| `crates/commands/src/lib.rs` | `802f0577d77f7e72ab39f5f40d6b5364e3b7a21a4a4535f0c5627ad78d7a1e89` |
| `crates/commands/src/read_cache.rs` | `8a17a575e8fbd158694844f54cb920962f2b752bc0509d3e8a1ad541fc7f7f45` |
| `crates/core/src/plan.rs` | `930c9843ee0148e70f56122549cc228333785c616ac62c57e937149b2c36a2f1` |
| `crates/core/src/plan/native_flac.rs` | `9b099cff59c406d4c80606e33e0ce48fb8c9df711ca527cb2b7d9d7757a11c63` |

The separate admission/reentry report records earlier native private-I/O
reentry and nested-budget findings. They are not additional findings established
by this policy review.

This review does not establish Windows/macOS behavior, physical power-loss
durability, a process RSS ceiling, production scale latency, every file format or
the complete service worker topology. ModelPolicy TLS requires explicit entry on
metadata-reading worker threads; a scope does not grant file-write authority.
The owned fixtures contain synthetic FLAC frames, so this report makes no new
decoder or audible-playback quality claim. Full workspace and hosted gates
remain the orchestrator's integration responsibility. Eligible shared backend
changes follow the upstream curation route in `docs/WORKFLOW.md`, preserving
upstream inline journal images and its stable waveform API.
