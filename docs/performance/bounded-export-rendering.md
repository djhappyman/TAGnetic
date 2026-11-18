# Bounded export rendering

All native desktop and connected-agent CSV, HTML, XML, M3U, CUE and text report
exports now build through a byte-limited text sink. The limit is 64 MiB per
export request, including the aggregate of grouped playlists. Headers, newlines,
UTF-8 and escaping count before appending. CSV/HTML/XML/CUE escaping streams
unchanged slices and replacement tokens without building expanded cell copies.
Nothing is returned or published on a rendering failure; guarded output
publication and rollback remain the existing implementation.

Report preparation clones only mask-referenced fields and implicit side/position
inputs. Unrelated large tags are excluded from that temporary view. Pattern
appends check their existing 1 MiB limit before growing the result; referenced
oversized values are rejected before sanitizing/copying them. Existing report
behavior still skips mask-invalid rows. Mask functions retain their own bounded
inputs/results and may use temporary buffers; this is not an entire-process RSS
guarantee. Source metadata and playlist entry collection also remain in memory.
No export-encoding feature or report-template parity is claimed here.

Verification: 921 core/commands/service tests passed (15 ignored); all-target,
all-feature Clippy and formatting passed. Tests cover exact/one-byte-short/zero
budgets for all formats, multibyte text, formula quoting, escaping expansion,
bounded scratch capacity and immediate sink failure. A disposable comparison
against the prior builders passed for all six formats and their existing tests
(11 tests); its source copy was removed after verification. Existing confined
publication tests passed. No installed runtime or user media was changed.

Route: core/commands changes are reusable upstream `main` candidates; service
integration requires fork-aware curation. No hosted or cross-platform validation
is claimed by this local checkpoint.
