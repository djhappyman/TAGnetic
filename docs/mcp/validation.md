# MCP validation for 1.4.7

Local verification was performed on Linux from fork main
`0fb56b7fa5416cbaea22f59a438e23f3709f612b`. Hosted exact-head Linux, Windows,
macOS and advisory checks remain the merge gate; release packaging is separate.

## Real clients and shared staging

Actual Codex 6.1 Sol and OpenCode Go DeepSeek Flash clients connected through
the installed executable's production stdio MCP. Music operations used the
shared Rust service rather than the development JavaScript bridge.

DeepSeek staged and wrote changes to two MP3s, verified non-destructive staging
and idempotent retries, and saved filter presets, masks and transformations.
Codex obtained fresh Discogs evidence, composed tags, artwork and comments,
and repaired those files. Independent metadata/image readback and encoded
audio-packet hashes confirmed the writes. Apply, strict Undo and Redo passed
on disposable copies. Existing history and unrelated pending edits survived.

A subsequent natural-language live demonstration staged cleanup, numbering,
filenames, comments and recreated front artwork across 53 tracks in two albums,
then paused before Apply. The agent created reusable filters, masks and cleanup
transformations through MCP. The person could inspect the same working values
in Files, Staged and the Inspector.

The shared stage is authoritative. Handles identify live file/field scopes;
Apply seals their current values into a separate accepted-write receipt.
Human changes remain current, unrelated fields stay editable, and disconnecting
an agent does not discard accepted staging. Saved-file reads remain distinct
from `staging.get` working differences.

The live demonstration exposed and fixed two frontend integration defects:

- A re-entrant automatic refresh could clear the UI preview during shared-stage
  hydration. Native staged work remained intact; rendering now occurs outside
  publication suppression and retains the shared working identity.
- Selection with staged renames rebuilt all 53 visible rows twice. Selection
  with no automatic fields to remove now performs no preview publication or
  table repaint. The live reproduction changed from 106 row replacements to
  zero, retaining all 53 nodes and the Inspector's current values.

Digital recreations were generated from verified official cover references,
allowing reconstructed fine details as requested. The selected 1254-square
images were encoded as optimised JPEGs. This is artwork recreation rather than
a claim of pixel-identical archival restoration.

## Local evidence

- Backend workspace before shared-stage extension: 954 passed, 12 ignored.
- Shared-stage service extension: 97 tests passed, with focused command
  working-value, artwork-pool and legacy-receipt regressions.
- Commands: 304 unit tests and four integration tests passed, followed by a
  focused staged-mask rendering regression.
- Desktop native: 61 passed, three ignored; subsequent shared API compilation
  and the actual desktop launch passed.
- Final JavaScript units: 171 passed. Lint: 88 modules, no problems.
- Browser suite before the final selection fix: 586 passed, 11 opt-in skipped.
  Final focused MCP/selection suite: 19 passed, including retained row identity,
  current Inspector values, shared Apply and preservation of human edits.
- Svelte diagnostics/autofixer, production frontend build, native formatting
  and Clippy passed during integration.
- Installed stdio and official SDK tests covered authentication, revocation,
  private framing, ownership, cancellation, bounded responses and clean EOF.

Synthetic 100,000-row metadata and 2,000 disposable WAV checks exercised paging,
aggregate handle budgets, scoped cancellation and index construction outside the
shared App mutex. They do not establish 100,000-file disk scanning throughput,
provider latency or native animation frame rates.

## Boundaries

Windows/macOS runtime permissions and IPC have not been exercised on those
operating systems locally. Compilation and packaging do not prove native runtime
behavior. Optional MCP Tasks/subscriptions are not required; clients poll stable
native operation IDs. The desktop owns the profile, library and journal; the
connector does not create a separate headless writer.

See [transport](transport.md) for protocol boundaries and
[the upstream ledger](../UPSTREAM.md) for contribution routing.
