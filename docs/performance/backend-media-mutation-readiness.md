# Durable media prewrite primitive

Implementation checkpoint, 18 November 2026, shared
`feat/native-field-foundations` checkout following `9da71249`. This is a
standalone core dependency for the next Executor integration. It does not change
Apply/Undo, journal/schema storage, existing writers, the UI, or the deliberately
failing untouched-value writer canary. Independent review and the parent's
combined gate remain required.

## Implemented interface and behavior

`core::media_mutation` exposes `Store`, `BoundMedia`, `Prepared`, `Published`,
bounded records/fingerprints, a durable `StateSink` acknowledgement contract,
and cooperative `Observer` boundaries. `Store::prepare` accepts a held original
capability, an explicit alphanumeric extension hint, maximum candidate growth,
and a trusted native edit/verify callback. The callback receives a descriptor-
anchored candidate path ending in the supplied extension. A real TagEngine
FLAC edit through that path is tested; original bytes remain unchanged until
publication. This primitive does not itself prove the callback's format-specific
semantic preservation. That remains the selected native writer's responsibility.

`BoundMedia::from_file` validates a regular read/write, non-append capability
against its logical absolute Unicode path and held final parent. It records
device/inode/birth identity and permission/owner attributes. An independently
opened file description obtains an exclusive nonblocking file lock, including
when callers supply cloned handles. This serializes participating writers across
stores and processes. It is not approved-root authorization; Executor must still
construct it inside its existing scoped mutation authority. Uncooperative
external programs can ignore advisory locks.

The state sequence is:

1. Reserve metadata, one job, streaming headroom and both full images on disk.
   Create a private operation directory and a synced `Preparing` manifest.
2. Stream the held original into `preimage`, hashing in 64 KiB blocks, and sync it.
   Verify source change witnesses. Preallocate the candidate plus admitted growth,
   stream the preimage into it, and invoke the native edit/verify callback.
3. Check candidate object identity, length and streaming SHA-256, sync the
   candidate, recheck the original, and durably write/acknowledge `Prepared`.
4. Revalidate both images and the source. Durably write the local `Publishing`
   manifest and obtain the sink's acknowledgement **before any original write**.
   Callback failure or uncertainty retains both images even if no bytes changed.
5. Revalidate again. Reserve required original-file extents, then copy the
   candidate through the held original inode in bounded blocks, set length and
   sync. Immutable candidate/preimage witnesses are bound to their expected
   SHA-256 images before publication callbacks. Every subsequent callback,
   write allowance, write and truncation checks those same witnesses. Per-write
   guards also check original permissions/identity, owned artifacts and the
   original's verified change witness. Verify the resulting
   image and durably write/acknowledge `Published`.
6. `Published::retire` requires explicit confirmation of the caller's durable
   journal finish and fresh content proof. A synced root retirement tombstone
   carries the owned directory/artifact identities while cleanup runs. A crash
   between artifact removals or directory removal can be resumed explicitly with
   `finish_retirement`; it cannot lose the ownership proof midway through cleanup.

Publication keeps the original inode, owner, ordinary permission mode, hardlinks
and xattrs. Tests verify these properties, growth/shrink/empty outputs, and raw
FLAC encoded audio bytes. A temporary file renamed over the original would
change the hardlink/metadata contract and is not used.

## Evidence, recovery and cleanup ownership

The private store requires current-user ownership, mode 0700 directories and
0600 files, no final links, and no multiply linked artifact files. Existing
unrelated directories are never chmodded or adopted. Directory-relative
no-follow opens, held handles, opaque random operation IDs, strict artifact names,
native object identities and an exclusive store lock protect its namespace.
New recovery files and directories carry a bounded reserved user xattr binding
them to their native object identity. Original admission checks the held file
and walks held physical parent directories; textual aliases cannot bypass it.
File markers also reject protected hardlinks outside the store, including through
another store or process after the first store handle closes. Ordinary media
hardlinks remain supported. Marker errors fail closed. An unmarked nonempty
directory from the earlier prototype is unsupported rather than silently adopted;
no automatic migration of that recovery evidence is implemented here.
No service storage code was extracted or weakened. Only the existing rustix
`process` feature was added for effective-UID checks.

Small serialization-agnostic binary manifests are versioned, checksummed and
bounded to 8 KiB. At most seven artifacts belong to one operation. Parsing rejects
unknown/future/corrupt manifests, missing phase predecessors, changed identities,
path traversal, unknown names and reservation inconsistencies. An untrusted
manifest cannot supply an arbitrary cleanup path. No full media BLOB or full
media `Vec` is allocated. A partially written/corrupt manifest causes conservative
store refusal with evidence retained; this checkpoint does not repair arbitrary
damaged manifests or automatically delete their directory.

Before an intent is attempted, Drop can remove only the still-matching artifacts
created by that operation. A replaced artifact, changed parent or uncertain
cleanup is retained, with its disk reservation. After intent, Drop never cleans
the images, including during panic, cancellation, write error, failed sync or
failed journal acknowledgement. Reservations survive reopening by scanning the
bounded manifests. Successful explicit retirement releases them only after the
owned files/directories have been removed and synced.

`inspect` verifies images and classifies current bytes as preimage, candidate,
ambiguous or different identity. It never restores or deletes the current file.
An interrupted in-place copy can leave mixed bytes; such bytes are not proof
that no foreign edit occurred. Retain the images and current file and require a
separate explicit recovery decision. `resume_published` only reconciles a proven
already-published image; it does not replay a write. Normal completed Undo must
remain selective and use its own protected mutation, not restore a whole-file
backup over unrelated later edits.

## Limits and platform boundary

Defaults admit 128 entries, 64 GiB of reserved store bytes and 8 GiB per media
image. Each operation reserves original size + maximum candidate size + 40 KiB
manifest overhead. The store reserves 8 KiB of participating retained metadata
per configured entry; active streaming reserves 136 KiB and one shared resource
job. All admission is nonblocking. Saturation and arithmetic/size excess refuse
before native editing or original writes. Disk allocation uses Linux
`fallocate(KEEP_SIZE)` rather than assuming sparse `set_len` reserves space.

These bounds cover this primitive. The trusted native callback must budget its
own allocations and obey admitted candidate growth; a callback exceeding it is
refused before publication. They are not a kernel filesystem quota or whole-
process RSS ceiling. Source and recovery storage may be on different filesystems;
allocation or sync failure on either side is retained conservatively.

This checkpoint is opt-in **Linux only**, requiring usable `/proc/self/fd`
directory paths, birth identities, file locks, directory sync and extent
allocation, plus native-identity protection xattrs on recovery storage. Missing
xattr support returns Unsupported; it cannot fall back to prefix-only exclusion.
Non-Linux `Store::open` returns Unsupported and `SUPPORTED` is false;
the Linux capability/publish API is not exported there. Windows/macOS adapters,
their private ACL creation and runtime recovery checks remain pending. Existing
non-Linux workflows do not route through this module.

## Verification

Disposable fixtures use the reused dev environment TMPDIR under Tagrex. No real
media, live app, user profile or Undo history was accessed. Fixture owners remove
their files after assertions. Commands:

```sh
source /mnt/Games/Development/tagrex/dev-env.sh
cargo test -p tagrex-core --test media_mutation -- --nocapture
cargo test -p tagrex-core --test media_mutation bounded_streaming_profile -- --ignored --nocapture
cargo clippy -p tagrex-core --all-targets --all-features -- -D warnings
cargo check -p tagrex-core --target x86_64-pc-windows-gnu
cargo test -p tagrex-core --lib --quiet
rustfmt --edition 2021 --check crates/core/src/media_mutation.rs crates/core/tests/media_mutation.rs
git diff --check -- crates/core/src/media_mutation.rs crates/core/tests/media_mutation.rs crates/core/Cargo.toml crates/core/src/lib.rs docs/performance/backend-media-mutation-readiness.md
```

The corrected module run passed 24 cases, with one manual profile ignored by
default. It launches and SIGKILLs a real child process at **87** observed
copy/state/sync/retirement boundaries, bypassing destructors. Every pre-intent
original stays unchanged; interrupted publishing keeps complete images, and
retirement resumes from its tombstone. This tests process death, not physical
power-loss/storage-controller behavior.

Other probes cover concurrent disk and job saturation, clone-handle lock
exclusion, initial private permissions, read-only/append handles, permission
changes, oversized/corrupt inputs, foreign source/parent/artifact replacement,
manifest corruption and recomputed-checksum traversal, candidate callback
panic/error, post-intent sink panic, acknowledgement failure, injected ENOSPC and WriteZero, positive
short write allowances, and failed sync boundaries. These are controlled I/O
fault injections; they do not fill the user's disk or pretend to simulate every
kernel/filesystem fault. The FLAC candidate test uses independent raw metadata
and encoded-payload readback.

## Independent preservation corrections

The independent review retained five public-caller probes. Its initial result
was one pass and four failures, in two P1 families:

- The publish-start callback could change the same-length candidate or sole
  preimage after their hashes were checked. `copy` captured a new input witness
  after that callback. It then overwrote the original before detecting bad
  evidence, and the preimage case lost the last original image.
- A parent alias bypassed a textual original-path prefix check. A second
  publication then overwrote the first unretired operation's only preimage.

Those four assertions are unchanged and now pass, together with the reviewer's
reentrant sink control. The first family is corrected by SHA-bound immutable
image seals: callback validation cannot substitute a new metadata baseline for
the expected image. The second is corrected by held physical ancestry checks
and persistent native-identity object markers, with no unsafe exclusion fallback.

Publication observers and durable state sinks are checked after successful
callbacks, before more original bytes can change. The same image/source proof
discipline covers original sync, Published/Retired acknowledgements and retirement
manifest callbacks. Cleanup can stop checking an image only after that exact
owned artifact has deliberately been removed; all remaining image witnesses stay
fixed. It cannot promote changed evidence after a callback.

Additional author tests corrupt each image at every observed publication event,
including later streaming chunks; at the write allowance with its original mtime
restored; in Publishing/Published/Retired sinks; in both retirement-manifest sync
stages; and during cleanup. They assert exact original bytes at the reached
boundary and prove no further original writes or false success. Cross-store
physical aliases/hardlinks are also tested in a fresh process after closing the
first store. The original permissions/resource/disk/error/SIGKILL suite still
passes. A separate fresh review remains required after these corrections.

This is observed-change protection, not mandatory isolation from an arbitrary
external writer. The supported change-witness assumption is native identity plus
size, mtime and ctime with adequate filesystem sensitivity. Rehashing whole media
after every 64 KiB block would make the algorithm quadratic; streaming instead
uses fixed witnesses and bounded small-manifest validation. Full SHA-256 proof
is taken at image establishment and explicit content-proof boundaries. An actor
that destroys private evidence after original writes have begun can still destroy
recoverability; the module refuses further progress and retains current bytes and
remaining evidence. It does not claim to reconstruct data deleted or corrupted
by another actor, or automatically restore an ambiguous original.

Correction gates:

```sh
cargo test -p tagrex-core --test media_mutation --test media_mutation_adversarial --locked --quiet
# 24 author cases passed / 1 manual ignored; 5 independent cases passed unchanged
cargo test -p tagrex-core --test media_mutation bounded_streaming_profile --locked -- --ignored --nocapture
# explicitly passed
cargo clippy -p tagrex-core --all-targets --all-features --locked -- -D warnings
# passed
```

One intermediate locked Clippy attempt correctly stopped because another
authorized component was updating the shared lockfile. The parent completed that
update, and the locked gate above was then rerun successfully. No dependency or
lockfile changes were made by this correction.

One explicitly run streamed profile produced:

| Media size | Prepare ms | Publish ms | Retire ms | Maximum write | Observed process RSS before/after |
| --- | ---: | ---: | ---: | ---: | --- |
| 1 MiB | 4.985 | 9.320 | 6.324 | 65,536 bytes | 6,608 / 7,076 KiB |
| 16 MiB | 45.842 | 95.980 | 70.325 | 65,536 bytes | 7,084 / 7,092 KiB |
| 32 MiB | 88.575 | 186.357 | 136.920 | 65,536 bytes | 7,096 / 7,108 KiB |

Every output byte was independently streamed and checked; current charges and
reserved disk returned to baseline. These are one warm local debug-build run,
including multiple SHA-256 passes and sync calls. They establish observed
bounded buffering and the cost of this local protocol, not cold-disk or enterprise
throughput guarantees or a speedup claim over earlier runs. This measurement
includes the corrected publication/retirement guards. The callback only patches seven fixture bytes here;
actual native container edits need their own measurements.

The scoped core all-target/all-feature Clippy gate passed. The corrected core
library gate passed 266 tests, with one ignored. Owned source formatting and
whitespace checks passed. Windows cross-compilation was attempted using the
installed Rust target, but bundled SQLite could not find
`x86_64-w64-mingw32-gcc`; it remains unverified. No compiler/toolchain was installed
and no Windows runtime support is implied.

## Integration still required

Executor must own the store and state sink, persist the operation reference and
intent before invoking publication, and confirm actual durable journal finish
before retirement. Wrap a whole file's tag/cover/block edit once, before its
transfer; protect edits of copy destinations separately without touching their
sources. Preserve the scoped parent/file authority and original parser hint.
The current module does not modify plan.rs, journal.rs or their schema.

The selected native writer must preserve untouched namespaces, order, qualifiers,
private data and exact repeated/empty values, with format-specific candidate
verification before returning success. The existing Title-only scalar preservation
canary remains a separate blocker. Exact stage atoms/digests, persistent field
outcomes, selective restart Undo, idempotency and the supported platform matrix
must then be integrated and independently checked before UI exposure.

Reusable core behavior is eligible for a curated `tagrex/tagrex:main` change.
Fork-specific storage/manifest routing and journal compatibility remain separate;
upstream inline artwork history is unchanged. No commit/publication, full safe-
writer completion or overall backend/UI-readiness claim is made.

Structural discovery used codebase-memory Tier 2 for existing resources, journal,
private storage and mutation seams in `tagrex-backend-readiness`, generation
2026-11-18T12:48:55Z. New module/test paths are not yet tracked, and lib.rs/Cargo
and active mutation sources have changed; their actual source was read. Coverage
for resources/private_storage matched that snapshot. No exhaustive graph claim
is inferred.
