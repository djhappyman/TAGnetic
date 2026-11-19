# Held-parent recovery store: independent review

Checkpoint ready for this bounded Linux opening seam. No new production defect
was found in the reviewed candidate. This is not Executor, persistent journal,
whole-backend or native-platform approval.

## Scope and authority

Reviewed `Store::open_in` and its shared `open_with_parent` initializer, initial
parent/descriptor checks, descriptor-relative child initialization, final parent
witness and existing store validation. Production source stayed unchanged.
Reviewer-owned deliverables are `crates/core/tests/media_store_parent_independent.rs`
and this report. The review reused the existing toolchain, build cache and
resource coordinator. No commits, active profiles, running app, user media or
existing Undo were changed.

This reusable backend behavior is eligible for a curated `tagrex/tagrex:main`
contribution once its foundation is compatible. Upstream inline journal images
and the stable waveform API remain separate contracts. This evidence applies to
the programme checkout, not a published or merged PR.

Graph Tier 2 used `tagrex-backend-readiness`, generation
`2026-11-18T12:48:55Z`, confirmed programme root. Store discovery returned no
matching node. Exact coverage reported the media module and author tests as
untracked; current source was read directly. Resource coordinator coverage
matched metadata. No complete graph or repository-wide audit is claimed.

## Independent probes

The new binary has **seven passing test functions**, including one child entry
and six parent probes:

- Real fixture children exit without destructors after Prepared and Published.
  Held-parent reopening recovers the exact operation ID/phase and reports the
  actual current image. The complete namespace snapshot stays unchanged during
  opening/inspection. Published recovery can then retire normally.
- Retirement stops with a real observer error after the first owned unlink.
  Closing and held-parent reopening retains the terminal proof; finishing
  retirement clears the entry and preserves the published bytes and media inode.
- A held parent is renamed before entry while the replacement profile contains
  its own published recovery history. Opening with the stale handle refuses
  with no byte, inode, mode, link, ctime, protection-marker or history change in
  either tree. Opening the foreign profile with its own correct handle still
  reads its original operation.
- An externally hardlinked lock makes opening refuse 48 times. Every attempt
  releases resource ownership and descriptors without changing any recovery
  bytes or namespace. Removing only the fixture-owned external link permits the
  same history to reopen.
- Unicode and non-UTF-8 store basenames create and reopen through the same held
  parent without changing unrelated original bytes or retaining failed owners.
- On Linux x86_64, an owned-child syscall tracer deterministically replaces the
  logical parent at two exact boundaries: immediately before `mkdirat`, and
  immediately before the final parent revalidation open. Both return Changed.
  Any initialized storage remains under the displaced held directory. The new
  foreign profile retains only its unchanged sentinel; its directory inode,
  mode, ctime and xattrs are unchanged, and no foreign store appears.

The timing probe uses Python's standard-library ctypes interface to ptrace only
its own fixture child and its libtest worker thread. It has a 20-second deadline,
EXITKILL and finally-based kill/reap cleanup. It changes no production hooks,
configuration or environment behavior. The tracer's initial setup required
correcting its thread tracking and syscall selection; those were test-harness
issues, not production findings. The final successful assertions require that
the target boundary actually occurred, so a missed interception cannot pass.

The earlier author-caught O_PATH side-effect defect remains a passing retained
canary. This review found no additional unresolved scoped defect.

## Verification

Using the existing dev environment on Linux x86_64:

```sh
source /mnt/Games/Development/tagrex/dev-env.sh
cargo test -p tagrex-core --test media_store_parent_independent --test media_store_parent --test media_mutation --test media_mutation_adversarial --test media_terminal_recheck --test media_witness_alias_recheck --test media_held_file --test media_held_editor_adversarial --locked
cargo clippy -p tagrex-core --lib --test media_store_parent_independent --all-features --locked -- -D warnings
cargo fmt --check -p tagrex-core
git diff --check -- crates/core/tests/media_store_parent_independent.rs docs/performance/backend-media-parent-independent-review.md
```

Eight-binary retained gate: **71 passed, 0 failed, one existing manual test
ignored**. Subprocess summaries are not added a second time. Scoped all-features
Clippy and core formatting passed. The ignored media streaming profile was not
rerun in this opening-seam review.

Compact evidence is under `/mnt/Games/Development/tagrex/tmp/`:
`media-parent-independent-retained.log`, `media-parent-independent-clippy.log`
and `media-parent-independent-fmt.log`. All owned fixture directories were
removed by their guards, including test-harness failure paths. Owned child
processes were reaped.

## Reviewed hashes

```text
b46fe5bb6f462d1d356a7b44ccb10dd800eb44cbbb051873137b48935181a8fc  crates/core/src/media_mutation.rs
44628fb432f9f85a3d9cfe5a3f5db83cde9b27b91e8d16ef1e52a85e47fdf4b2  crates/core/tests/media_store_parent.rs
34b67e891c04bb32f126bebe1b71d74636cfb5b1c3f31520b28ea30fa2d1844c  crates/core/tests/media_store_parent_independent.rs
```

## Limits

The caller must authorize the parent/profile when acquiring the directory
handle. This opener does not itself bind a journal to the profile or grant new
roots. Final witnesses do not prevent another process changing the namespace
afterward. A detected during-initialization change can leave initialized
recovery objects under the held parent; they are retained evidence, not silently
removed. The controlled timing probes cover two specific boundaries rather than
all possible concurrent mutations.

Independent added fixtures use opaque synthetic bytes because this seam opens
and recovers storage rather than parsing an audio codec. Retained media tests
cover the existing protected publication behavior. No expanded audio-format
preservation claim follows from the added opener probes. Native Windows/macOS,
other Linux architectures' syscall tracing, unsupported filesystems, physical
power loss, whole-process RSS and production Executor/journal integration remain
separate verification gates.
