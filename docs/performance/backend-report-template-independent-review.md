# Structured report renderer: independent adversarial review

Scoped review of the new pure report renderer, 18 November 2026. The reviewer
changed only this report and `crates/core/tests/report_template_adversarial.rs`.
No production, UI, media, runtime, journal, commit or publication changes.

## Finding and correction

**P2 — XML text changed carriage returns on consumer readback.**
`Escape::XmlText` originally emitted raw CR characters. Python's independent XML
consumer read a supplied `"\r"` as `"\n"`; CRLF values also lost their CR. This
contradicted preserving supplied field whitespace. The normal retained canary
`xml_text_does_not_emit_raw_carriage_returns_that_xml_readers_normalize` and an
explicit UTF-8/UTF-16 XML consumer probe both failed before correction.

The parent changed markup data escaping to encode CR as `&#13;`. The original
assertions remain unchanged and now pass. A further control verifies that HTML
and XML data use references while template literals and quoted CSV cells retain
literal CR. The correction was inspected at the actual shared escaping branch.

## Independent evidence

Final public-interface suite: **7 passed, 0 failed**, explicitly including the
two normally ignored independent-consumer tests:

- 128 deterministic generated datasets compare nested lexical/encounter groups,
  track scope, original member order, repeated entries, group keys, known-zero
  durations and unknown-duration counts against a separate selection oracle.
- Python CSV readback covers empty values, CR/CRLF/LF, tabs, quotes, delimiters,
  Unicode and NUL across UTF-8, UTF-16LE and UTF-16BE.
- Python XML readback covers CR, CRLF/LF, tabs, markup punctuation and Unicode
  across those three Unicode encodings.
- An isolated observational System allocator checks compile/render peak work
  against participating admission. The same test performs successful nested
  rendering from an outer cancellation callback, then cancels the outer render:
  the inner result stays valid and charged until its owner drops.
- Cancellation during a large escaping pass and during group construction
  preserves a previously retained result and releases all new working/job
  charges. Dropping final owners returns the injected budget to zero.

Executed on the current Linux checkout using the shared locked toolchain/cache:

```text
cargo test -p tagrex-core --test report_template_adversarial --all-features --locked -- --include-ignored
cargo clippy -p tagrex-core --test report_template_adversarial --all-features --locked --no-deps -- -D warnings
rustfmt --edition 2021 --check crates/core/tests/report_template_adversarial.rs
```

All passed after correction. The initial expanded suite was 4 passed / 2 failed;
the final seventh case is an additional correction control. No assertion was
weakened or failing probe removed. No disposable media or filesystem fixture
was created; subprocesses consumed report bytes through pipes.

Checked production SHA-256:
`b42552cd7e39a5103df3d48ea50fd74edbd8a4f8096d63fb8b95d8ea427e95ff`
(`crates/core/src/report_template.rs`). Review-test SHA-256:
`93841d8e78daaafac6f6bf7de210ebef2e02f4877c0f6a2709c313aec4454455`.

## Evidence boundary and route

Tier 2 graph project `tagrex-backend-readiness`, generation
`2026-11-18T12:48:55Z`, reports the new renderer and tests as untracked; the full
current renderer was read directly. Coverage was also checked for the consumed
bounded-mask and text-codec paths. No claim is made that the old graph indexes
these new interfaces or that this review proves all shared-mask internals.

No further confirmed defect was found in this finite renderer scope. This does
not approve command transport, staged snapshot/seal integration, guarded output
publication, append/restart behavior, exact repeated-value support, the complete
backend roadmap or Windows/macOS runtime behavior. Compiled-mask accounting also
has a separate review. Reusable core changes are candidates for curated upstream
`tagrex/tagrex:main` after combined gates; no PR is created by this review.
