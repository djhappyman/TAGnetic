# Independent media mutation review

**Changes required.** Two P1 preservation failures remain in the reviewed Linux
primitive. This review does not approve Executor/journal integration, the selected
metadata writer, non-Linux support or the wider backend programme.

Reviewed 18 November 2026 in the existing dependent foundation checkout following
`9da71249`; the source tree is intentionally dirty with parallel programme work.
Reviewed `crates/core/src/media_mutation.rs` SHA-256:
`d9deb9eb9b9ff4d9e550ef34f518ef52bd0ca1a825eb205424c39bf704153056`.

The reviewer owns only `core/tests/media_mutation_adversarial.rs` and this report.
No production, journal, schema, runtime, profile or real-media changes were made.
Reusable corrections are candidates for curated upstream `tagrex/tagrex:main`;
any fork storage integration and platform validation remain separate gates.

## P1: publication starts after its image proofs have become stale

`copy` emits `CopyStarted(Publish)` before capturing its input witness
(`media_mutation.rs:338`). The publication guard then checks artifact identities
and namespace accounting, but does not bind the verified candidate/preimage
content witnesses to the first original write.

Two retained public-caller canaries change an owned artifact at that observable
boundary, without touching the original or retaining a writable candidate handle:

- `changed_candidate_at_publish_start_refuses_before_overwriting_the_original`
  changes the candidate to a different **same-length** image. Publication copies
  those foreign bytes into the original, then returns an error. Detecting the
  wrong digest after copying is too late to satisfy prewrite refusal.
- `changed_preimage_at_publish_start_refuses_before_destroying_the_last_original_image`
  changes the sole preimage. Publication overwrites the still-valid original
  with the candidate and detects the damaged preimage only afterwards. None of
  the original, preimage or candidate then contains the old original bytes.

Both should refuse before the first original write. Preserve expected candidate
and preimage content/change witnesses from successful verification and check
those same witnesses after public callbacks and before writing. Do not capture a
new input witness after a callback and thereby endorse changed bytes. Continued
streaming checks must retain the original proof instead of silently replacing it.

These are controlled corruption/change probes in a disposable private store,
not a claim to prevent every uncooperative writer or storage-controller failure.
The important distinction is that the implementation has an explicit callback
boundary after proof and before first write that can be revalidated safely.

## P1: a parent alias admits a recovery artifact as fresh media

`Store::prepare` excludes originals using only
`original.logical.starts_with(store.path)` (`media_mutation.rs:1062`). A parent
symlink alias to the same held store is outside that textual prefix.

`a_recovery_artifact_cannot_be_adopted_as_new_media_through_a_parent_alias`
reproduces admission of the first operation's `preimage` through that alias.
The stronger `aliased_artifact_publication_must_not_overwrite_an_unretired_preimage`
then actually publishes a second operation to that adopted file. It overwrites
the only old preimage of the first, deliberately unretired publication. The first
operation's current file remains its new candidate; its old bytes have been lost.

The adoption-only probe initially looked like a P2 isolation gap. Actual
publication demonstrates preservation loss, so the finding is ranked P1.
Reject recovery objects through held physical ancestry/object identity, including
aliases, rather than relying on a logical path prefix. The caller still owns
approved-root authorization, but that does not make this primitive's explicit
recovery-artifact exclusion safe.

## Passing control and verification

`reentrant_state_sink_can_inspect_and_prepare_another_file_without_deadlock_or_leaking_reservations`
passes. It queries public records inside each sink acknowledgement, performs a
nested preparation of another fixture file, and checks reservation/resource
release and unchanged unrelated bytes. This exercises a different path from the
author's hook enumeration.

Commands actually run:

```sh
source /mnt/Games/Development/tagrex/dev-env.sh
cargo test -p tagrex-core --test media_mutation_adversarial --locked -- --nocapture
cargo clippy -p tagrex-core --test media_mutation_adversarial --all-features --locked --no-deps -- -D warnings
rustfmt --edition 2021 --check crates/core/tests/media_mutation_adversarial.rs
```

Final Linux test result: **one passed, four failed**, with the four unchanged
normal regressions retained. Scoped Clippy and owned formatting passed. The
fixture owner removes all test originals and recovery stores during unwinding;
no rejected experiment or fixture tree is an intended deliverable. The test also
contains a meaningful non-Linux opt-out check, but no Windows/macOS execution is
claimed.

The graph project `tagrex-backend-readiness` was checked at generation
`2026-11-18T12:48:55Z`. Coverage reports the module/test as untracked; the entire
bounded production module and the author's test/report were read from current
source. No completeness claim is inferred from the graph.

After implementation, rerun these assertions unchanged, add correction-specific
stress and obtain a fresh review when the ownership/publication design changes.
The author's existing process-kill, permissions, disk/resource and retirement
suites remain necessary; this bounded review does not replace them.
