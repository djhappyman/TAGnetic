# Library query measurements

Measured 15 November 2026 on Linux x64, Intel i7-12700K, Node 22.23.3 and the
running Tauri/WebKit development build. The Rust commands crate uses the normal
development profile with `opt-level=2`; this is not a release-build claim.

The baseline is the actual JavaScript evaluator at frontend checkpoint `213f06c`.
Both engines receive identical deterministic 10k, 50k and 100k synthetic rows and
equivalent queries. Every result count and deterministic index checksum matches.
Each warm measurement uses 40 samples after warmup. Exact values, cold timings,
p95, machine details and methods are in the adjacent JSON files.

## Pure matching

Warm median milliseconds, excluding IPC and rendering:

| Tracks | Query | Previous JS | Rust | Ratio |
| ---: | --- | ---: | ---: | ---: |
| 10,000 | Free text | 0.455 | 0.138 | 3.30× |
| 50,000 | Free text | 2.721 | 1.236 | 2.20× |
| 100,000 | Free text | 6.022 | 3.489 | 1.73× |
| 100,000 | Genre + missing artwork | 4.313 | 0.099 | 43.42× |
| 100,000 | Artist equality + artwork | 2.896 | 0.048 | 60.89× |
| 100,000 | Alternative conditions + text | 7.912 | 1.190 | 6.65× |
| 100,000 | Bounded regex (internal API) | 2.502 | 0.019 | 131.90× |

Index construction took 26 / 126 / 253 ms at 10k / 50k / 100k. It happens once per
snapshot, outside the UI thread. This fixture has repeated metadata values;
regex and facet-heavy gains depend on cardinality and query shape. Broad free
text still scans indexed text, so it improves less than structured conditions.

## Actual WebKit and Tauri IPC

The isolated debug-only native benchmark keeps the real library and journal
untouched. It measures the previous evaluator directly inside WebKit and the
full native request/response, including result-index serialization. WebKit's
clock reports integer milliseconds here; sub-millisecond differences cannot be
inferred from this run.

| 100,000 tracks | Previous JS median | Rust + IPC median | Rust + IPC p95 |
| --- | ---: | ---: | ---: |
| Free text | 6 ms | 7 ms | 9 ms |
| Genre + missing artwork | 6 ms | 2 ms | 3 ms |
| Artist equality + artwork | 6 ms | 1 ms | 3 ms |
| Alternative conditions + text | 15 ms | 8 ms | 13 ms |
| Bounded regex (internal API) | 4 ms | 1 ms | 2 ms |

The largest observed 4 ms heartbeat gap during these warm native query samples
was 24 ms; none exceeded 50 ms. The corresponding previous-JS maximum was 27 ms.
These workloads do not establish that the old UI always froze, or that every
native query is faster. Free-text round-trip latency is slightly higher, while
matching executes outside the UI thread.

Returning all 100,000 indices took 11 ms median / 17 ms p95, with a maximum
heartbeat gap of 10 ms. Synthetic fixture upload plus index setup took 1,318 ms
at 100k (198 ms in index construction). Production builds its initial index
from the native scan rows, without this synthetic frontend upload.

Regex is retained only in the engine compatibility/benchmark API; its app
controls and stored-mode effects were removed at Mike's request. The comparison
chart shows the four workloads relevant to the current app.

The user-facing 160 ms suggestion delay and existing search debounce are
intentional interaction costs and are excluded. Table rendering, disk scanning
and duplicate audio analysis are also excluded. Browser virtualization checks
are separate from native engine/IPC measurements.

The real runtime still held the same 53 tracks, one selected file, zero pending
edits/preview and 37 Undo batches before and after these measurements.

## Compact field suggestions and table scale

The final one-rule-per-field suggestion mode was measured separately on fresh
10k/50k/100k snapshots with three terms (`Artist 17`, `Hardcore`, `17`). At 100k,
warm native medians were 1.64–3.06 ms and p95 2.12–5.06 ms. First suggestions
took 165–231 ms because their distinct-value facets were still cold. This cost
runs outside the UI thread and is separate from the 160 ms input delay. Each
returned field was unique and its count was checked independently against rows.
These are native-only suggestion timings, excluding IPC; no speedup ratio is
claimed for the changed suggestion semantics.

Separate Chromium mock tests selected all 20k/50k/100k files and scrolled 30
positions. All three passed: at most 50 mounted track rows, all selected paths
retained, final row reached, and zero full-selection iterations during scrolling.
Their timing samples include two deliberately awaited animation frames and are
not query latency or native throughput measurements.

## Progressive duplicate table

A separate serial Chromium fixture pairs every file into duplicate groups and
grows from 40% to all published groups while scrolled into the table. At 20k,
50k and 100k files (10k/25k/50k groups), the final implementation kept 47 rows
mounted, retained the visible row object and moved its scroll anchor by 0 px.
The 100k case deliberately stresses a much larger number of groups than an
ordinary library is likely to contain.

Profiling identified redundant full-library filtering/folder ordering and
temporary index arrays during each duplicate publication. The final model skips
that unused work and fills navigation indexes as rows are added. One before/after
sample per size is retained as diagnostic evidence, not a stable speedup claim.
After cleanup, publication-to-paint took about 301/316/357 ms. Those samples
include the 180 ms status poll, 120 ms query debounce, mock JavaScript matching
and two animation frames. The largest heartbeat gaps were 29/48/89 ms: the
extreme 100k-file/50k-group burst can still interrupt a frame. Native query
latencies above measure a different boundary and must not be substituted for
these whole-table updates. Real duplicate audio-analysis throughput remains
unmeasured.

Reproduce with `TAGREX_DUPLICATE_BENCH=1 npx playwright test
tests/ui/query-command-bar.spec.js --grep 'large progressive' --workers=1` after
building the UI fixture. See [progressive table evidence](library-query-progress-results.json).

## Reproduce

Reuse the existing Rust toolchain, dependencies and build cache.

```sh
cargo build -p tagrex-commands --example query_bench
node tools/bench-library-query.mjs
node tools/bench-library-suggestions.mjs
```

In the running development WebView, import
`/@fs/<checkout>/tools/bench-library-query-webview.mjs` and call `run()`. The
function disposes each synthetic index in `finally` and compares live runtime
state before returning. On bridges with a short evaluation timeout, retain the
Promise result on a temporary window property and read it after completion.
These benchmark commands are absent from release builds.

Sources: [pure measurements](library-query-results.json),
[WebKit measurements](library-query-webview-results.json),
[suggestion measurements](library-suggestion-results.json),
[table scale checks](library-query-table-results.json),
[comparison](library-query-comparison.html),
[shared fixtures](../../tools/library-query-fixtures.mjs).
