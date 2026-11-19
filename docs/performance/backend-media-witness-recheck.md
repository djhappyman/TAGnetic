# Fresh media image-witness and alias recheck

**Changes required:** one new P2 retirement-proof validation gap. The corrected
publication/physical-alias behavior passed the fresh bounded probes. This is not
approval of Executor integration, selective metadata writing, the wider backend,
Windows/macOS execution, or power-loss behavior.

Reviewed 18 November 2026 in the existing dependent foundation checkout after
`9da71249`, with other programme work active. Current production source reviewed:
`crates/core/src/media_mutation.rs`, SHA-256
`a120496bd9505f14cf1a2ee0ad4f99dff7686aba4a0c0b18d2febb1708c99d52`.
The reviewer changed only `crates/core/tests/media_witness_alias_recheck.rs` and
this report. Production and prior review tests were untouched. Reusable backend
corrections are candidates for curated upstream `tagrex/tagrex:main`; the fork's
storage integration remains separate.

## P2: cleanup can start after its durable fallback record becomes corrupt

Retained normal regression:
`corrupted_retirement_tombstone_is_revalidated_before_any_cleanup_mutation`.

The public retirement observer changes one checksum byte in the root `t-ID`
tombstone at the second `BeforeSync(Manifest(Retired))` event. This is after the
fallback record was written and before any artifact cleanup. The observer later
injects an error at the first `ArtifactRemoved` event.

The implementation proceeds and deletes `0-preparing` before detecting the
injected failure. `CleanupObserver` revalidates the original and remaining image
witnesses, but does not bind the root tombstone's identity/content to those
checks. The final check compares only the tombstone's inode before unlinking it;
it occurs after entry cleanup.

An interruption at this point leaves a partial entry manifest chain and an
already-invalid root fallback. Safe reopen/retirement reconciliation is blocked.
In this controlled canary the current file still contains the candidate and the
old preimage still exists: **no P1 media-byte loss is claimed**. The defect is
avoidable use of already-invalid durable recovery context before cleanup.

Bind the terminal record to an immutable content/change witness before exposing
callbacks. Revalidate that witness and its named object after callbacks and
before cleanup mutations, retaining unknown material on refusal. Do not recapture
a new baseline after a callback or weaken the unchanged assertion.

## Passing correction-specific probes

The fresh suite exercises public APIs under different conditions from the first
review and the author's event sweep:

- Malformed protection markers (empty, future-like header, wrong native ID and
  oversized value) on an externally named hardlink refuse before editing. A
  copied marker on a different inode also refuses. Source and recovery images
  remain unchanged and no second-store reservation leaks.
- Moving the store's ancestor makes the old handle refuse stale logical access.
  After closing it, reopening through an intermediate parent alias still rejects
  adoption of protected media. Published inspection, resumption and confirmed
  retirement remain usable through the relocated store.
- Publishing acknowledgement errors and panics leave the unwritten original and
  both images intact. Reopen preserves `Publishing` and classifies the original
  as the preimage; it cannot be resumed as a completed publication.
- A same-length source edit with restored mtime inside a failing acknowledgement
  remains the foreign edit. Its prior preimage is retained and inspection is
  `Ambiguous`, rather than endorsing the edit as a published candidate.
- Real child processes cannot acquire the held original/store locks. Dropping
  only the outer store handle does not release a published operation's leases;
  dropping the final operation permits a new process to reopen and inspect it.

These checks do not promise isolation from an external actor that deliberately
removes every protection/evidence mechanism, or from uncooperative writes inside
a final syscall window. The caller still owns candidate-editor semantic checks
and durable journal acknowledgement. Non-Linux code has a meaningful explicit
unsupported-store check, but only Linux was executed.

## Verification and cleanup

Actually run with the existing development environment and locked dependencies:

```text
cargo test -p tagrex-core --test media_witness_alias_recheck --locked -- --nocapture
cargo clippy -p tagrex-core --test media_witness_alias_recheck --all-features --locked --no-deps -- -D warnings
rustfmt --edition 2021 --check crates/core/tests/media_witness_alias_recheck.rs
git diff --check -- crates/core/tests/media_witness_alias_recheck.rs
```

Final test result: **6 passed, 1 failed**. The passing count includes one child
helper which is exercised with both locked/released states by its parent test.
Scoped Clippy, owned formatting and whitespace checks passed. The initial marker
fixture tried a 4 KiB xattr value unsupported by this filesystem's xattr storage;
that fixture-only setup was corrected to 256 bytes, still exceeding the reader's
64-byte bound. No production behavior was changed to accommodate the fixture.

All fixture trees are task-owned and removed by their guards on success or panic;
no live media, profile, runtime or recovery history was touched. No commit or PR.

Graph project `tagrex-backend-readiness`, generation `2026-11-18T12:48:55Z`, was
checked. Coverage reports the new module and tests as untracked, so review used
current source fallback, including the complete publication, marker, manifest,
retirement and locking paths. No graph completeness claim is made.
