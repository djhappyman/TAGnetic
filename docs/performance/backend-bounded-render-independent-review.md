# Independent bounded-render and computed-overlay review

Review scope: shared bounded mask rendering and computed query ownership in the
existing foundation checkout. **Scoped correction rechecked; whole-backend and
platform approval are outside this review.** No production changes by reviewer.

## Confirmed finding: P2 prefix-list candidate short-circuit

A public bounded render of
`$stripprefix(%title%,UnreasonablyLongFirstCandidate,The)` with `The Artist`
returned the unchanged title instead of `Artist`. `split_prefix` used
`split_at_checked(word.len())?`, exiting the entire candidate search when an
unusable early candidate was too long or ended inside a UTF-8 character.
Unicode examples similarly skipped a later matching `尾` candidate after `xx`.
The same defect affected `$swapprefix` and legacy rendering; it predated this
work rather than being introduced by the new budget API.

The retained normal regression is
`every_prefix_candidate_is_considered_after_an_unusable_earlier_candidate` in
`crates/core/tests/mask_bound_adversarial.rs`. It failed unchanged before the
parent correction. The parent now continues to the next candidate; the unchanged
three prefix/swap assertions pass. No expected value was weakened.

## Independent stress evidence

- **Core review suite: five passed.** An observational System allocator in
  isolated subprocesses checked 150 deterministic nested/function/input cases
  across empty, Unicode-expanding, control-character and noise-suffix text;
  dynamic count errors; maximum allowed multi-byte padding; replacement,
  insertion, reverse, conditions and nested casing. Observed live allocation
  peaks fit the AST-derived allowance. Retained numeric diagnostics stayed
  bounded. This is finite evidence, not a proof for every possible AST.
- Near-limit wide argument and deeply nested render ASTs, plus a long Unicode
  literal, compiled within the caller's conservative `512 * source + 8192`
  allowance. All explicitly refused extraction. No extraction regex is stored
  by the render-only parser.
- A Unix probe checked invalid OS-path expansion and maximal supplied technical
  values against the allocation allowance without filesystem access. Legacy
  invalid-UTF-8 filename components remain empty; full paths use replacement
  conversion. Filename sanitisation/padding and raw value rendering stayed
  distinct, and admission one byte below the computed bound refused.
- **Public query review suite: three passed.** A nested successful query warmed
  its cache during an outer cancelled cold build; it remained usable with all
  expensive-job slots occupied. Injected transient pressure returned the typed
  desktop resource-busy code, preserved the hot owner, and retried after release.
  Invalidation inside a reentrant public callback cancelled the old generation.
  Final index drops returned all participating charges to baseline.

Commands, using locked dependencies and the existing toolchain/cache:

```text
cargo test -p tagrex-core --test mask_bound_adversarial --all-features --locked
cargo test -p tagrex-commands --test query_overlay_adversarial --all-features --locked
cargo clippy -p tagrex-core --test mask_bound_adversarial -p tagrex-commands --test query_overlay_adversarial --all-features --no-deps --locked -- -D warnings
```

The review uses the public renderer/query methods rather than including private
implementation modules. Only the two new review test files and this report were
written. Tests use in-memory metadata and isolated owned subprocesses, with no
real media, runtime/profile changes or leftover fixture trees.

## Evidence boundary and route

Tier 2 codebase-memory routing used project `tagrex-backend-readiness`, generation
`2026-11-18T12:48:55Z`. Coverage reported changed mask/functions/query files and an
untracked render-budget module; current source was read directly. Graph snippets
were not treated as fresh implementation evidence.

Source hashes after parent correction:

```text
mask.rs           9ae4d5023826d98ddca11e38d44c050bd047f8ef76f071002b7aa7d475d319a9
render_budget.rs  e131ee3517e9e07eb407312087013d81adce5e9481ae3a987908eefd1d2136ac
functions.rs      048930efcf4284e44df4a8828ee051670a0bf3478013ece4607de1c48ee35a38
query.rs          58f1a51f1433b87b974552a52a7d80f114028d6a8c43a53dba352c53a9f96efd
```

No further confirmed allocation underestimate or overlay ownership failure was
found in these exercised paths after correction. This does not establish an RSS
ceiling, budget result DTOs/indices, certify all CPU costs, review the new report
AST/output publisher, or verify Windows/macOS execution. Cross-platform hosted
checks, broader combined workloads and subsequent consumers remain required.
Reusable mask/query fixes are candidates for curated upstream
`tagrex/tagrex:main`; fork service/storage compatibility stays separate. No commit
or PR was created.
