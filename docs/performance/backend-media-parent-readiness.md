# Protected media store: held-parent opening

Checkpoint ready for a fresh independent review. This bounded Linux seam is
implemented in the existing programme checkout; it is not connected to Executor,
Apply, persistent Undo, or a live profile.

## Scope and route

Owned changes: `crates/core/src/media_mutation.rs`, new
`crates/core/tests/media_store_parent.rs`, and this report. The starting media
source hash was `f00f43a943c74d82b711f168168f8eca4c7171b17f1ca6094d07dfb0ad559814`.
Other agents' source and active runtime work were preserved. No commits or PRs
were made. This is reusable backend behavior eligible for a curated
`tagrex/tagrex:main` contribution, preserving upstream inline journal images and
its stable waveform API.

`Store::open_in(parent: File, logical_path, limits, budget)` consumes a
caller-held, readable parent directory. Before any mkdir, protection marker,
lock or recovery initialization, it verifies that the descriptor is a directory
and still equals the logical store path's current parent. It then creates or
opens the single final component relative to that held parent. It does not
reopen a different parent to choose where initialization writes.

The original `Store::open` and new entry point share one private initializer.
The original path/limits validation, resource admission, private ownership,
native object identity, xattr protection, locking, inventory limits and recovery
validation remain in that initializer. At its end the parent binding is checked
again and the existing store validation verifies the logical child, protection
marker and held lock before returning. No reservation or store limit was raised.
Non-Linux `open_in` explicitly returns Unsupported, with no path fallback.

The caller owns profile/root authorization when acquiring the parent. This API
does not grant roots, bind a journal to a profile, or isolate the namespace from
an unrelated process after the last witness. If initialization observes a later
parent change, it refuses and keeps evidence rather than deleting initialized
objects. A reopened store still uses its existing logical-path validation on
operations.

## Finding caught during implementation

**P3, corrected in the new candidate:** an `O_PATH` directory descriptor passes
metadata/identity checks but cannot flush its directory. The first implementation
created the store and then failed its parent sync. An unchanged dedicated probe
failed because the namespace had already changed. The initializer now checks
its descriptor flags before namespace work and rejects this unsupported parent
capability. The retained probe passes with no changed paths, modes, native
entries or protection markers.

A trailing-dot path is also retained as an authority refusal control: it cannot
change which parent the supplied handle authorizes. This is not reported as a
confirmed pre-existing defect.

## Verification

Only disposable fixtures under the shared dev environment's TMPDIR were used.
There were no live-media, existing Undo, runtime, application or profile writes.
The eight new tests check:

- Held-parent creation, typed held candidate editing, publication, close/reopen,
  persistent record lookup and retirement; the original media inode is retained.
  A final legacy `Store::open` reopens the same clean store.
- A renamed/replaced parent, with both an empty foreign profile and a populated
  private foreign store. The call refuses before touching either the held or
  foreign namespace. Recursive snapshots compare bytes, names, native inodes,
  modes, links, ctime and protection xattrs.
- Wrong parent/kind, relative/parent-traversing/root/NUL paths, parent or child
  symlinks, trailing-dot authority and a replaced child after a successful open.
- Resource-pressure refusal before filesystem effects, successful retry, and
  96 lock-refused attempts with unchanged descriptor count and fully released
  failed-attempt reservations.
- The original `O_PATH` earlier-side-effect canary, unchanged after its fix.

Final retained gate: **64 passed, 0 failed, 1 existing manual test ignored** across
seven media integration binaries. Subprocess test summaries are not counted a
second time. Scoped all-features Clippy and core formatting passed.

```sh
source /mnt/Games/Development/tagrex/dev-env.sh
cargo test -p tagrex-core --test media_store_parent --test media_mutation --test media_mutation_adversarial --test media_terminal_recheck --test media_witness_alias_recheck --test media_held_file --test media_held_editor_adversarial --locked
cargo clippy -p tagrex-core --lib --test media_store_parent --all-features --locked -- -D warnings
cargo fmt --check -p tagrex-core
```

Compact evidence under `/mnt/Games/Development/tagrex/tmp/`:
`media-parent-retained.log`, `media-parent-clippy.log`, `media-parent-fmt.log`,
`media-parent-path-only-red.log` and `media-parent-path-only-green.log`.

Graph Tier 2 project `tagrex-backend-readiness`, generation
`2026-11-18T12:48:55Z`, has the correct programme root. Parent graph search found
no Store node for this new module. Reviewer coverage checks report media source
and its relevant tests untracked, so actual current source was read directly;
resource coordinator coverage matched metadata. No exhaustive graph claim is made.

Frozen SHA-256:

```text
b46fe5bb6f462d1d356a7b44ccb10dd800eb44cbbb051873137b48935181a8fc  crates/core/src/media_mutation.rs
44628fb432f9f85a3d9cfe5a3f5db83cde9b27b91e8d16ef1e52a85e47fdf4b2  crates/core/tests/media_store_parent.rs
```

A new independent review remains required. Linux fixture results do not verify
native Windows/macOS, all filesystems, physical power loss, arbitrary concurrent
namespace mutations or production journal integration. The during-initialization
parent substitution window is guarded by before/after witnesses in source; this
author gate does not claim a deterministic external-process timing test for that
specific window. Existing media callback and process-death canaries were retained.


## Independent review completed

The [fresh independent review](backend-media-parent-independent-review.md) found
no additional scoped defect. Its final eight-binary gate passed **71 tests**,
with one existing manual profile ignored; scoped all-features Clippy and core
formatting passed on the unchanged frozen source hash above.

The reviewer added controlled owned-child Linux x86_64 syscall probes at two
specific initialization boundaries: before `mkdirat` and before the final
parent revalidation open. Each observed replacement returns Changed, leaves
initialized evidence anchored under the displaced held parent, and preserves
the foreign profile's inode, mode, ctime, xattrs and sentinel. Prepared/Published
process exits, partial-retirement reopening, foreign published history,
hardlinked-lock refusals and descriptor/resource cleanup also pass. These
probes address the earlier missing deterministic timing evidence; they do not
prove every concurrent mutation, other architectures or other native platforms.

The opener checkpoint is ready within that Linux scope. Production
Executor/journal binding is now the next integration slice, not a completed
behavior of this module.
