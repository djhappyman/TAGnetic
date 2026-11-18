# Independent base query admission review

18 November 2026. Scoped review completed after one correction. No new confirmed
issue remains in the reviewed correction; this is not whole-backend approval.

## Scope and source evidence

Reviewed `QueryRow` saved/raw/folded/search ownership, its fallible constructors,
row updates and custom-index retention, and native App list/refresh/snapshot
publication. Read desktop and service result propagation. The Tier 2 graph
`tagrex-backend-readiness` (generation `2026-11-18T12:48:55Z`) identified the
existing structure/callers; coverage reported modified metadata for query,
command, service and shell files, so current source was read directly. Resource
coordinator coverage matched. The new review test is outside that index snapshot.

Native session paths construct admitted rows; the ordinary `from_track` and
`from_tracks` helpers remain unmetered caller-managed entry points. App's
`from_rows` receives rows produced by its fallible constructor. Tests do not
certify arbitrary future consumers of the unmetered helpers.

Final reviewed `crates/commands/src/query.rs` SHA-256:
`3494da87aa4cfd41acec9315e822eaf3ce3e63e10f6e98ed72d08bcf82a6942d`.
This is an uncommitted working-source checkpoint, not a hosted exact-head gate.

## Finding and correction

**P2 — tag-block count bypassed allocation admission.** The public
`try_from_track_in` estimate included label bytes but omitted the number of
labels, temporary label-reference vector and join delimiters. A caller-created
DTO containing 100,000 empty-label blocks, given a 64 KiB budget, allocated
**2,397,810 bytes** before returning `TooLarge`; the shared retained high-water
was only **13,296 bytes**. The independent observational System allocator
excludes caller-owned input and runs this probe in its own process.

This was a public constructor invariant failure. Native parsers currently expose
only a small known block set; the review did not establish a malicious native
file that produces 100,000 `TagBlockDto`s.

The parent corrected production admission by refusing more than 256 blocks
before construction and adding checked 128-byte overhead per admitted block.
The original allocation canary now passes unchanged. An additional 256/257
boundary test verifies empty-label joining still works at the admitted boundary,
while the next member refuses without leaked ownership. No expected result was
relaxed to accommodate the defect.

## Executed independent checks

`crates/commands/tests/query_base_admission_review.rs` has seven normal tests:

- Actual native WAV snapshots retain documented technical mask results after
  technical values move to the small overlay.
- 96 rows with 128 Unicode/custom fields each retain shared saved ownership
  through an update, old snapshot retention and repeated custom-index use.
- Cancellation part-way through native listing publishes no partial generation,
  leaves the previous snapshot usable and releases partial row charges.
- A policy change during native listing refuses mixed interpretation generations;
  a later complete snapshot uses the new generation.
- The original 100,000-empty-label allocation canary refuses before excessive
  allocation after correction.
- The 256/257 label boundary observes admission and release correctly.
- 32 simultaneous row constructors contend for one 1 MiB retained budget; some
  succeed, some refuse, the budget is never overbooked and all charges release.

Commands actually run using the existing `dev-env.sh` toolchain/cache:

```text
cargo test -p tagrex-commands --test query_base_admission_review --all-features --locked -- --nocapture
# 7 passed, 0 failed, 0 ignored
cargo clippy -p tagrex-commands --test query_base_admission_review --all-features --no-deps --locked -- -D warnings
# passed
rustfmt --edition 2021 --check crates/commands/tests/query_base_admission_review.rs
git diff --check -- crates/commands/tests/query_base_admission_review.rs docs/performance/backend-query-base-independent-review.md
```

An early test draft used unsupported query-version/operator tokens and a bare
technical placeholder; those harness mistakes were corrected to the documented
query version 2, `equals` operator and `_` technical placeholders before reporting
behavior. They were not implementation findings.

## Boundaries and route

This scope accounts query base ownership, not every response DTO, compiled query,
computed overlay, library parser, process allocation or RSS. Existing low-level
constructors remain intentionally unmetered. Native device/filesystem platform
validation and complete integrated regression/hosted gates remain outstanding.
The unrelated scalar native-value preservation and recovery ownership blockers
are not waived by these results.

Only this report and the new independent test were authored by the reviewer.
No production edits, commits, PRs, live app/profile changes or real media writes.
Fixtures are disposable and removed by scoped cleanup. Reusable query fixes are
candidates for upstream `tagrex/tagrex:main`; fork service differences retain
separate compatibility review.
