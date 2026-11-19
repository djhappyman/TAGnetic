# Confined output publication hardening

Status: implemented in the foundation checkout; independent review and native
Windows/macOS checks remain required. This is an in-process publication
checkpoint, not a completed crash-recovery protocol or a claim that the whole
backend is ready for interface exposure.

## Problem and resulting behavior

The original production-path regression published the first output, moved that
owned file aside, installed an unrelated replacement, and forced a later output
collision. Rollback deleted the unrelated replacement. Its assertions are
unchanged and now pass.

Publication now holds the original and generated file identities open and checks
their SHA-256 content and change witnesses. It validates the complete batch
before the first publication, repeats each baseline check at publication, and
checks the final public outputs and backups before committing. An overwrite
request does not authorize replacing a file that changed after its baseline.

Public output removal first moves the entry without replacing any destination
into a fresh private capture, then verifies the captured identity/content. A
changed captured object is restored without replacement or retained with an
explicit recovery error. Rollback seals and verifies saved originals again
before restoring them. Cleanup checks backups and staged candidates the same
way; a source name recreated after transport is no longer owned, even if the
replacement is a hard link to the generated file. No production destructor
unlinks or restores files by their names.

Originals remain available on incomplete rollback. When a named backup has been
replaced or removed but its held original is intact, a bounded streaming rescue
copy is attempted. Failure to prove ownership or restore safely returns
`OutputError::RecoveryRequired` with inspectable artifact paths. Best-effort
`recovery.json` records have `automatic_recovery: false`; they are evidence for
the failed live operation, not authorization for a future process to replay,
delete or restore those paths. Cleanup failure after commit keeps the new
outputs and reports retained evidence rather than rolling back a batch whose
earlier backups may already have been retired.

On Linux/macOS the transport uses native no-replace rename. Unsupported native
operations fail closed; there is no checked-existence followed by ordinary
replacing rename fallback. Windows uses no-replace `MoveFileExW`, pinned
ancestors, full native file identities, and checked directory guards held
through path operations. Native Windows execution is still a gate.

## Bounds and callers

Admission happens before staging: at most 128 outputs, 64 MiB of new payload,
256 MiB of total existing originals, 64 MiB per original, 4 KiB per input and
canonical parent path, and 256 KiB of input paths. Input payloads are already
owned by the caller; this reservation covers the publisher's additional
structures and work, not total caller memory or process RSS. Existing originals
are hashed in 64 KiB streaming buffers rather than cloned.

Each operation reserves a shared backend job slot, 2 MiB of transient work, a
conservative path/record allowance, and an estimated descriptor allowance from a
separate process limit of 768. The shared backend currently permits eight jobs.
Contention returns typed `Busy`; oversize batches return typed `TooLarge` before
partial publication. Some batches can hit descriptor admission before the count
limit, especially deep Windows paths with many distinct parents. Operating
system descriptor exhaustion also remains possible and follows the same safe
failure path. A recovery error retains only its returned path/message memory
charge; job, transient and descriptor reservations release when the operation
returns. Dropping the error releases the remaining charge.

`publish_confined_outputs` returns the typed error. Application and service
boundaries distinguish resource pressure/limits, changed files and required
recovery. The service no longer claims every failed output batch was restored.
No interface, branding, journal image representation or waveform behavior changes
are included.

## Adversarial evidence

`cargo test -p tagrex-commands --lib output:: --all-features --locked --quiet`
passes 27 tests on Linux. These include the unchanged original replacement
regression plus actual production-writer interleavings covering:

- A changed later destination during overwrite, same-inode content modification
  with restored modification time, and replaced backup/candidate names.
- Backup replacement at restoration and committed cleanup boundaries; unrelated
  bytes survive and cleanup never rolls back already committed new outputs.
- A recreated former source slot using the exact generated inode, moved parents
  with replacement symlinks, foreign files in the private-directory name, and
  hard-linked original aliases.
- Real kernel short writes and `EFBIG`, using a child process under a file-size
  limit; only the generated successful prefix is cleaned up.
- A process exiting after its first publication; original bytes remain in
  private artifacts. This deliberately does **not** prove restart recovery.
- Eight simultaneously staged publishers, refusal of a ninth before creating
  its output, an overlapping destination race, successful maximum-count
  publication, whole-batch oversize refusal, sync-boundary failure, and resource
  ownership through a retained recovery error.

Unrelated functional fixtures run in child test processes so their admission
does not interfere with one another. The concurrency test exercises eight real
publishers together inside one process; production limits are unchanged.

Commands/service all-target, all-feature Clippy with denied warnings passes on
Linux. The full commands library run passes 401 tests, with 10 existing ignored
tests. Final integrated service/desktop verification is owned by the
orchestrator because other backend changes are in progress concurrently.
Windows cross-check was attempted offline but stopped in dependency
build scripts because `x86_64-w64-mingw32-gcc` is unavailable; it did not verify
this module. The Windows-only directory-pinning canary still needs native
execution.

## Remaining gates and limits

Independent review must challenge capture/restoration ownership, resource
estimates, and actual service error propagation. Native Windows/macOS execution
and their filesystem sharing/rename failure cases are required before claiming
platform readiness. Windows reparse-point files currently refuse publication;
cloud-file compatibility needs native validation.

This protocol protects against ordinary external changes to public names and
detectable changes to retained artifacts. Fresh unpredictable private captures
and Unix mode 0700 limit namespace access; they are not a security boundary
against another privileged or same-account process deliberately modifying the
private namespace at arbitrary instruction boundaries. No filesystem protocol
can promise preservation against an external actor independently destroying all
copies of the data. Do not infer that guarantee from these tests.

There is no durable, authenticated restart discovery/replay, output append
protocol, or power-loss/directory-sync proof here. The crash test proves retained
bytes at one interruption boundary, not atomic visibility of a multi-file batch
or automatic restoration after restart. Those remain separate backend work.

Scope: `commands/src/output.rs` and its new ownership/test modules, the typed
commands/service error boundary, and the cached `getrandom` dependency. Shared
backend fixes are eligible for `tagrex/tagrex:main`; preserve fork service
compatibility and upstream's inline journal images. No commit or PR was made by
this worker.
