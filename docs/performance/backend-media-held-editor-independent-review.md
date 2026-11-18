# Independent held-editor preparation review

**No new confirmed blocker found in the bounded Linux seam.** The corrected
post-editor witness and shared held/path preparation routine pass the independent
interleavings below. This does not approve production Executor, SQL journal,
shared-stage, platform or full-backend integration.

Only `core/tests/media_held_editor_adversarial.rs` and this report were authored.
Production source, existing tests, commits, PRs, live app/profile and user media
were not changed. Reusable core corrections belong in curated upstream
`tagrex/tagrex:main`; fork storage compatibility and upstream inline journal
images/waveform behavior remain separate requirements.

## Independent conditions

The new target contains seven test functions, including one subprocess fixture
entry point. Six substantive probes exercise the actual public Store interfaces:

- **48 callback fault cases.** Both the held-file/typed-sink facade and legacy
  path/dynamic-sink facade encounter an IO error or unwind at each of the twelve
  observed post-editor callback positions. The source and its hardlink retain
  exact bytes and inode identity. Owned prepublication artifacts clean up,
  participating resource charges return to the open-Store baseline, and no
  successful Prepared handle escapes. The expected fixture panics are caught.
- **96 candidate/entry mutations.** At every post-editor position through the
  Prepared acknowledgement, both facades encounter same-length foreign bytes
  with restored mtime, candidate growth, a replaced candidate name, or a moved
  entry directory with a foreign replacement. Preparation refuses. The original
  stays byte-exact; renamed edited candidates and foreign files/directories
  survive. Ambiguous entries retain their capacity reservation. This challenges
  the entire callback tail, beyond the author's individual AfterEdit and sink
  mutation canaries.
- **Same-object reentrancy.** Opening the original through an independent
  hardlink while either editor facade is active refuses without blocking or
  disturbing the outer capability. Dropping the Prepared attempt releases that
  lock and permits a new capability. Original bytes and resource accounting
  remain unchanged.
- **Seven real hard-exit boundaries.** A child edits a disposable FLAC using the
  actual native writer and codec, then exits without destructors in the editor,
  immediately before Prepared acknowledgement, after synced Prepared ack, after
  prepare returns, after synced Publishing ack, after synced Published ack, or
  after publication returns. A newly opened Store validates the retained
  preimage/candidate, recorded phase, source inode/content and receipt evidence.
- **Published receipt after restart.** A child-published operation is reopened,
  confirmed from its synced fixture acknowledgement, and retired. With the old
  full images gone, its decoded exact receipt drives a second held-file protected
  operation that restores the full independently constructed FLAC bytes and
  original inode. Literal separators, case-distinct genres, empty custom values,
  Unicode and the encoded suffix survive. This is a fixture journal/Store test,
  not the application's SQL history protocol.
- **Retirement pressure and retry.** With one admitted job, a second prepared
  operation makes retirement of a completed first operation return typed Busy.
  The first remains Published with inspectable evidence and its source bytes;
  the second source is unchanged. Dropping the second attempt permits reopening
  and retirement using the same typed acknowledgement driver. No capacity or
  participating-resource owner leaks.

The fixture acknowledgement file is created and synced with its parent directory
before preparation; each later acknowledgement writes and syncs the actual phase,
Store ID, before/after fingerprints and optional encoded receipt. Child processes
have a fifteen-second deadline and are reaped. They exercise process exit and
reopen, not a physical power cut.

## Explicit recovery boundaries

Before an editor returns, a crash leaves a Preparing manifest with no committed
output proof or durable receipt. The independent test observes `Current::Ambiguous`
even though the known fixture source remains unchanged. Prepared and Publishing
boundaries with an unchanged source inspect as Preimage; Published boundaries
inspect as Candidate. Only the Published case is resumed/retired automatically
by these tests. This review does not invent authority to discard unresolved
Preparing entries.

A cooperative error after a Prepared acknowledgement can leave caller-owned
acknowledgement evidence while validated prepublication Store artifacts clean up.
The caller must record/reconcile that failed attempt. Prepared is not permission
to infer original publication. Production integration must bind the complete
plan, source identity, phase and selective rollback receipt, and prove a durable
finished file outcome before retiring full images.

## Verification

```text
cargo test -p tagrex-core --test media_held_editor_adversarial --test media_held_file --test media_mutation --test media_mutation_adversarial --test media_witness_alias_recheck --test media_terminal_recheck --locked --quiet
7 independent + 7 held-seam author + 24 media author + 5 image/artifact
+ 7 witness/alias + 6 terminal = 56 passed; 0 failed; 1 existing manual probe ignored
cargo clippy -p tagrex-core --test media_held_editor_adversarial --all-features --no-deps --locked -- -D warnings
passed
rustfmt --edition 2021 --check crates/core/tests/media_held_editor_adversarial.rs
passed
```

The unchanged terminal suite retains its 175 mutation cases, callback-error/panic
and reopen coverage. Its subprocess helper's nested success line is not counted
again as an extra top-level test. No ignored performance probe was rerun or
claimed passing in this review. Logs remain as compact source evidence in
`tmp/media-held-independent-combined-final.log` and
`tmp/media-held-independent-clippy.log` outside the checkout.

Reviewed production SHA-256:
`f00f43a943c74d82b711f168168f8eca4c7171b17f1ca6094d07dfb0ad559814`.
Independent test SHA-256:
`a13a89254cd01e24835621a24e00f9518b8e42297b15f5064cd98d33c08c6137`.

Tier 2 discovery checked `tagrex-backend-readiness`, generation
`2026-11-18T12:48:55Z`, rooted in this checkout. The bounded structural search
found no indexed held-editor symbols. Coverage reports the media/new test/native
writer/codec files untracked, exact/model source changed, and resources metadata
matching with a best-effort caveat. Current source was read for preparation,
post-editor validation, phase acknowledgement, publication/retirement, drop
cleanup, original locks, receipt calls and participating-resource accounting.
No absence or exhaustive architecture conclusion rests on the stale graph.

Native Windows/macOS, filesystems with different metadata/xattr behavior, physical
power loss, hostile same-account writers at arbitrary syscall boundaries, whole
process RSS and production SQL/Executor recovery remain outside approval. This
seam was tested without loading or changing a live library, profile or Undo
history. All owned fixture trees clean up on successful completion.
