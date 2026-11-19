# Durable move ownership: fresh independent review

18 November 2026. **Link-entry correction independently rechecked; no new confirmed
defect in this scoped correction.** This
review does not approve all recovery, platform behavior or the backend programme.

## Scope and source evidence

Reviewed the persisted native object proof in `core/src/journal.rs` and its
initial capture, Undo, rollback and path authority consumers in `core/src/plan.rs`
and `plan_paths.rs`. Graph Tier 2 project `tagrex-backend-readiness`, generation
`2026-11-18T12:48:55Z`, points to this checkout. Exact-path coverage reported
metadata changes for all three production files and the existing stress tests;
current source was read directly. The graph's old `ensure_untransfer_safe`
relationships were not treated as the new implementation's complete call chain.

Ownership of this review is limited to the new integration test
`crates/core/tests/durable_move_identity_fresh_review.rs` and this report. No
production code, existing tests, journal schema, user media or running profile
was changed. Disposable fixture trees clean up through `Drop`; the real process
exit child uses the parent fixture's directory, which the parent later removes.

## Confirmed finding

### P2: owned sidecar links cannot recover when their referent disappears

The new move proof correctly identifies the link **entry**, but Undo preflight
still canonicalizes `to` using `ensure_within_roots`. Canonicalization follows
the referent. If the target file disappears while the app is closed, Undo returns
`Io(NotFound)` even though the same stamped link entry remains inside the
approved directory and no target contents need to be read or modified.

The retry predicate has the same semantic mismatch: `is_untransferred` calls
`Path::exists`, which follows links. A dangling original link does not count as
restored. Therefore a restarted Undo cannot finish when the original audio and
link objects were already restored before interruption and only the referent
has subsequently disappeared.

Two independent actual-Executor regressions initially reproduced the failure:

- `stamped_sidecar_link_can_be_undone_after_its_referent_disappears`
- `already_restored_dangling_link_is_recognized_during_restart_retry`

Both use an ordinary valid WAV and a relative link created by the fixture. After
Apply and journal close, the fixture renames the referent to another owned name,
leaving its bytes intact. No link identity or payload is replaced. Expected
behavior is to contain/verify the stamped entry, move only that entry, and finish
or recognize completion without following the referent. The current refusal
preserves data and history, but strands an otherwise recoverable operation.

Suggested correction: use parent containment plus no-follow entry existence for
stamped sidecar link transfers and already-restored detection. Keep media access
regular-file-only, preserve unknown/replaced entry refusal, and retain the
existing ancestor capability and destination no-clobber checks. Do not fix this
by following the destination or treating any dangling entry as owned.

## Parent correction under final recheck

The parent corrected sidecar-move preflight to check the containing parent and
no-follow entry existence, while retaining native move-proof verification. It
also changed completion detection to use no-follow entry metadata and count only
NotFound as absence. Source review confirms that regular media and copy gates
were not changed to follow unknown links.

After the unrelated new module compilation work settled, the reviewer reran all
six unchanged canaries: **6 passed, 0 failed, 0 ignored**. Scoped all-feature
Clippy with denied warnings passed. Both initially failing link recovery and
already-restored retry tests now pass without weakening their expectations.
The corrected plan source SHA-256 at the verified checkpoint is
`e48009e29a83369975f45d438cb71f3969ae8ac828f591ae9a7c4b41e0a93b39`.
This verifies the correction in the tested Linux scope; it does not close the
separately documented final-source-check/rename syscall race.

## Other adversarial evidence

Four normal integration probes passed:

1. A replacement with equal encoded bytes and restored modification time is
   refused across restart. Restoring the actual object makes retry succeed;
   subsequent in-place sample changes on the same object survive move Undo.
2. Empty, truncated, wrong-version and arbitrary native identity blobs cannot
   authorize movement or remove history. A moved/copy-kind mismatch is refused;
   restoring the original valid proof makes Undo succeed.
3. A child exits with code 73 immediately after the first real move of two.
   Reopened history contains both write-ahead identities, remains interrupted,
   and Undo reverses only the reached transfer without damaging the untouched
   second file. This is process death, not a mock journal or callback exception.
4. A corrupt move proof in a mixed copy/move batch is refused before deleting
   the earlier valid copy; source, copied output, moved output and history stay
   intact.

## Verification

Initial red gate and final correction gate were both executed.

Run against the current dirty foundation checkout, using the existing development
Rust/toolchain/build cache:

```text
cargo test -p tagrex-core --test durable_move_identity_fresh_review --all-features --locked -- --nocapture
initial: 4 passed; 2 failed; 0 ignored
final corrected source, unchanged tests: 6 passed; 0 failed; 0 ignored

cargo clippy -p tagrex-core --test durable_move_identity_fresh_review --all-features --locked --no-deps -- -D warnings
passed

rustfmt --edition 2021 --check crates/core/tests/durable_move_identity_fresh_review.rs
passed

git diff --check -- crates/core/tests/durable_move_identity_fresh_review.rs
passed
```

The test initially had a wrong progress callback type and was corrected before
execution; the two runtime failures above are production behavior, not compilation
failures. No assertion was weakened to make a production failure green.

## Limits and route

Linux fixture execution only. Native Windows/macOS identity and filesystem checks
remain unverified here. The already-documented source-name race between final
identity check and the rename syscall remains open; durable private move staging
is the next planned scope and is not claimed implemented by this review. The
unrelated selective metadata writer preservation blocker is outside this review.

Reusable core corrections are candidates for curated upstream
`tagrex/tagrex:main`. Fork journal compatibility, old weak-history refusal and
upstream inline image behavior still require integration review. No commit, PR
or publication was performed.
