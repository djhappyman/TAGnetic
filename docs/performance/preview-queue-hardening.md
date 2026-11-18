# Preview queue hardening

Resource queues admit at most 64 active/waiting jobs before spawning blocking
workers. Excess requests return a retryable busy error; input buffers are not
silently discarded. This is a conservative job-count cap, not a measured optimum
or a process-wide byte limit. Existing mutation admission, recovery and FIFO
ordering remain separate from read-only preview supersession.

The native field-operation preview uses an explicit latest-request lane. A new
admitted request cancels its predecessor, wakes obsolete queue waiters and feeds
the existing per-file cancellation callback. A superseded result is refused
again before returning. Other preview lanes and normal jobs are unaffected.
Only this native preview command opts in; the primitive does not silently
coalesce unrelated requests. Cancellation remains cooperative during a running
filesystem read or parser call; those operations cannot be forcibly interrupted.

Blocking workers now own the serialization permit and admission guard. Dropping
an invoking future cannot let a second worker enter the same resource while the
first one is still running. Errors and panics release both guards. Cancelled
waiters release their task wakers on drop.

Verification: all 69 desktop tests passed (3 ignored), including 8 queue tests;
all-target/all-feature desktop Clippy and workspace formatting passed. Tests
exercise admission pressure, dropped queued/running callers, 1,000 superseded
waiting previews with only the final computation executed, running cancellation,
independent lanes, FIFO normal jobs and panic/error cleanup. No queue-wait or
wall-clock speedup claim is made; production queue/compute profiling remains
useful for tuning admission. The running development app was not restarted for
this checkpoint and still uses the earlier verified handoff build.

Route: reusable desktop Rust fix for curated upstream `main`.
