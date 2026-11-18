# Derived query memory

Computed filter rows share immutable saved-value maps and search strings with
their source rows. They hold only their computed overlay and row context rather
than another complete metadata copy. Template inputs are prepared only for
referenced fields; unrelated large lyrics are not cloned. Derived row/value
accounting has a 64 MiB admission budget. No partial index is cached after a
failure or cancellation.

Facet and regex caches now evict least-recently-used entries rather than clearing
everything. Each has a 32 MiB retained-data admission budget as well as the prior
entry caps. Facet estimates include raw/folded strings, row vector capacities,
member-credit storage and overhead. Regex admission conservatively charges both
2 MiB compilation/DFA limits plus pattern storage. These are estimates, not RSS
limits; regex engine scratch and concurrently held snapshots remain separate.

Facet construction also checks a 64 MiB working budget before cloning new values
or growing row vectors. Member parsing admits unique source credits and estimates
normalization scratch first. Oversized facets report an explicit query error;
they never return a partial or broadened match. Base metadata itself is not
limited by these derived-cache budgets. Retained snapshots and concurrent query
workers each have their own lifetime; no process-wide allocation cap is claimed.

Verification: 925 core/commands/service tests passed (15 ignored), warning-free
all-target/all-feature Clippy and formatting passed. Tests prove hot facet
retention, byte/count eviction, disabled/oversized admission, accounting on
replacement and clock rollover, refusal before value cloning and cancellation.
A 2 MiB unrelated lyrics fixture proves the computed overlay shares the exact
saved map/search allocations and leaves source values unchanged. Existing query
operator, computed-field, suggestion and large service projection tests passed.
No benchmark speedup or OS-specific peak-memory result is claimed.

Route: reusable commands-layer fix, eligible for curated upstream `main`.
