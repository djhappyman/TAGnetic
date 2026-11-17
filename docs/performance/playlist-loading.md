# Native playlist/text checkpoint

Date: 17 November 2026. Base: cache checkpoint `0fe97c60` on
`feat/native-field-foundations`. Scope: native local playlist resolution and a
shared bounded strict text codec. Owned source: `core/src/text_codec.rs`, its
module registration, `commands/src/playlist_load.rs`, disposable playlist tests,
and the existing bounded text-file decoding call site. No frontend source or
running application is changed.

## Verified behavior

The core codec tests assert exact UTF-16LE/BE BOM and surrogate bytes; Unicode
and newline roundtrips; malformed Unicode/conflicting BOM refusal; strict
Windows-1252 encoding with unmappable character errors; Shift-JIS and stateful
ISO-2022-JP conversion across chunk boundaries; output-expansion limits; and
cancellation without a partial result. The implementation uses the locked
encoding_rs 0.8.44 without adding a dependency or changing the lockfile.

Six command tests use disposable FLAC files and local playlists. They cover
Unicode order and duplicate canonical paths; missing, outside-scope, remote,
file-URI and nested-playlist diagnostics; explicit M3U codepages/UTF-16; strict
UTF-8 M3U8; significant spaces and escaped comment filenames; HLS/NUL/size/root
errors; cancellation and valid empty playlists; symlink and escaping-parent
refusal. Preview is checked against unchanged media bytes and performs no live
session or staged-plan replacement.

The existing text-import and text-sidecar tests also run with the shared codec.
Their prior BOM detection, input limit, malformed-text refusal and NUL policy
remain in the command layer. Playlist references use the existing scanner's
extension policy and the pinned confined-reader helper.

The full core/commands/service test command passed **911 tests**, with 15
manual/fixture-dependent tests ignored. Clippy passed with warnings denied.
Formatting and whitespace checks passed. This is Linux evidence; Windows/macOS
filesystem execution remains unverified.

```sh
source /mnt/Games/Development/tagrex/dev-env.sh
cargo fmt --all --check
rustfmt --edition 2021 --check crates/commands/src/playlist_load_tests.rs
cargo test -p tagrex-core -p tagrex-commands -p tagrex-service
cargo clippy -p tagrex-core -p tagrex-commands -p tagrex-service --all-targets --all-features -- -D warnings
```

## Review and completion boundary

Review checked explicit root ownership, lexical checks before target lookup,
regular/no-follow handles, retained-path copies and diagnostic accounting,
decoded expansion bounds, codepage output substitution, UTF-16 byte order,
cancellation and preserved live state. The resolver intentionally rejects
remote/HLS/nested documents and unsupported path syntax, and reports errors
instead of widening permissions. It does not claim media-container validation
from an extension match.

The native load preview and text codec are complete within the
[documented scope](../guide/playlist-loading.md). UI playlist controls, expression
grouping, report encoding selection, append/template integration, and the
larger storage/Undo migration are still pending. No full-parity, installer or
enterprise certification is claimed by this checkpoint.

Reusable core and command components are eligible for curated upstream `main`
work after compatibility review from a clean base; future UI goes to `next`.
This dependent local checkpoint does not publish, merge or claim hosted checks.
Temporary test outputs are removed after the committed evidence is verified.
