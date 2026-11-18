# Fresh EXIF correction recheck

18 November 2026. Separate review of the parent's correction following the P2
contradictory-Orientation finding. Scope: public artwork pixel processing and
`core/src/cover/pipeline.rs::exif_orientation`, not tag writing, stage integration,
Undo or overall backend readiness. Owned files: this report and
`core/tests/artwork_exif_fresh_recheck.rs`. No production edits were made.

## Disposition

**No new confirmed defect in this scoped recheck.** The prior contradictory-entry
regression now passes unchanged, and the correction is acceptable for parent
integration within its documented IFD0 orientation scope.

Reviewed pipeline SHA-256:
`4ee763c3d780fb963da86c377b267120ad6e79fb43a26a91bee4c8fe858cd07d`.
This is a working-tree source identifier, not a commit or hosted-CI result.

The parser examines all IFD0 entries, validates the orientation SHORT/count/value
shape, refuses conflicting duplicates, and checks cancellation every 128 entries.
Unrelated metadata offsets are not followed. A thumbnail's IFD1 orientation is
correctly kept distinct from the main image's orientation. These are bounded
semantics, not general EXIF validation or preservation.

## Independent conditions tested

The fresh suite uses public `Processor` calls and independently constructed TIFF
and carrier envelopes with valid PNG CRCs / JPEG APP1 lengths:

- All eight orientations, both TIFF byte orders, both PNG and JPEG carriers,
  with consistent duplicate declarations: 32 successful processing conditions.
  Twelve contradictory pairs are refused across those byte orders/carriers.
- 192 malformed TIFF conditions: every truncation of the base fixture, invalid
  byte order/revision, zero/header/out-of-range IFD offsets, oversized entry count,
  unsupported orientation types/counts, and zero/out-of-range orientation values.
  Every refusal releases transient/job resources and retains the existing hot
  cache entry; the next valid call still shares that cached image.
- Four thumbnail conditions with main-image orientation 1 and thumbnail
  orientation 6: no main-image rotation or false conflict.
- Maximum 65,535-entry PNG IFD, with the orientation as its last entry. Six
  cancellation positions (including late scanning) return `Cancelled` without
  cache publication or leaked resources. A subsequent uncancelled call succeeds.
- Explicit FFmpeg readback of little-endian duplicate declarations for all eight
  orientations in PNG and JPEG: all coordinate permutations match. PNG samples
  are byte-exact. JPEG allows at most three 8-bit levels for differences between
  independent decoder YCbCr rounding; distinct sample coordinates cannot be
  hidden by that tolerance.

The earlier independent suite was also rerun explicitly with its normally ignored
FFmpeg oracle: all eight EXIF orientations at both 8-bit and 16-bit RGBA precision
match their independently specified coordinates exactly. Its metadata-policy,
reentrancy, cancellation, lease-lifetime and original conflict canaries also pass.

## Checks actually run

```sh
cargo test -p tagrex-core --test artwork_exif_fresh_recheck -- --include-ignored --nocapture
cargo test -p tagrex-core --test artwork_pipeline_adversarial -- --include-ignored --nocapture
cargo clippy -p tagrex-core --lib --test artwork_exif_fresh_recheck --test artwork_pipeline_adversarial --all-features --no-deps -- -D warnings
rustfmt --edition 2021 --check crates/core/tests/artwork_exif_fresh_recheck.rs
git diff --check -- crates/core/tests/artwork_exif_fresh_recheck.rs docs/performance/backend-artwork-exif-recheck.md
```

Fresh suite: **5 passed, zero failed**, including its normally ignored external
oracle. Prior independent suite: **4 passed, zero failed**, including its external
oracle. Scoped Clippy with denied warnings, owned formatting and whitespace
checks pass. Shared dependency/build cache was reused. Whole-core tests were not
repeated while unrelated writer/recovery canaries are under correction.

Graph Tier 2 discovery used `tagrex-backend-readiness`, generation
`2026-11-18T12:48:55Z`, and completed both pages of the processor's outbound trace.
Coverage reported pipeline `metadata_changed`, prior tests `not_tracked` and
resources `metadata_match`. Current source and test files were read directly;
the graph was not treated as current source proof.

No temporary directories, live media, application profiles, runtime or Undo
history were touched. Encoded fixtures and external-tool pixels stayed in memory
and pipes; child processes were waited to completion. No commits or PRs were
created. Reusable core changes remain eligible for a curated backend contribution
to `tagrex/tagrex:main` after the parent's combined gates. This recheck does not
establish ICC/colour-managed filtering, general metadata retention, whole-process
RSS bounds, platform runtime support or complete artwork integration.
