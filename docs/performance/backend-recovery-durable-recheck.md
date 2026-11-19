# Fresh recovery review: interrupted, changes required

18 November 2026, integration base `9da71249` plus the current backend programme.
The independent reviewer produced `core/tests/recovery_durable_recheck.rs`, then
its turn stopped with an automated risk-filter error. There is no completed
review approval. This parent-maintained note preserves the reported evidence
and acceptance blockers; subsequent fixes need a fresh review.

Two reported passing conditions: a future schema is refused without changing
its unknown payload bytes; ordinary copied-file Undo works after journal reopen.

Two reported failing conditions: during Undo's public progress callback, rename
an approved target's parent directory and replace its old name with a link to
a different directory within the same disposable fixture tree but outside that
operation's approved root. Copy Undo deletes that unrelated fixture file when
its bytes/timestamp match the copy evidence. Move Undo transfers the unrelated
fixture file. Both operations returned success. The fixture's actual original
target remains in the renamed parent. These are P1 preservation/authority gaps,
even when content hashes agree.

The retained canaries check that unrelated fixture bytes remain untouched, the
operation refuses changed ancestry, the original target remains intact and
recovery history is retained. No real user directory, media or profile was used.

Required correction: keep directory authority bound through mutation with
pinned/no-follow directory-relative operations and reject changed ancestry.
Another preflight-only pathname check is insufficient. Apply/rollback/metadata
write paths need the same assessment. Durable removal reservations must remain
recoverable if an error or process interruption occurs during these transitions.

The original implementation worker is addressing these findings. Passing
earlier digest/no-clobber tests does not waive them. Windows/macOS, physical
power loss and network-filesystem validation remain separate gates.
