# Agent entry point

Before planning a change, delegating work, proposing a commit, opening or merging
a PR, or retiring a branch, read [docs/WORKFLOW.md](docs/WORKFLOW.md).
Orchestrators must include its scope, completion gate and publication target in each
worker brief and check the worker's evidence before integration.

Publish work to this fork. Upstream contributions are discontinued; bring in
upstream changes only on an explicit user request, following the workflow.

Use the installed `codebase-memory` skill for structural code discovery, checking
coverage and reading source where coverage is incomplete. Use `tagrex-ui-iteration`
for interface changes and `tagrex-safe-audio-testing` for tag/artwork/rename/Undo
verification. Preserve active user work, portable builds, runtime state and Undo
history; test writes only against disposable fixtures.

## Branches and the development runtime

For a new feature branch, reuse the existing dependencies, toolchain, build
cache and runtime profile. Branch from the latest required checkpoint, including
another active branch when the user requests it, and leave that agent's checkout
and running work intact. Creating a branch is not a reason to reinstall
dependencies, recreate the development environment or replace launch tooling.

When asked to connect the running app to a checkout, change only the source
attachment needed for iteration. Preserve pending edits, the loaded library and
Undo history. Restart only the affected process when necessary; verify its source
checkout and live updates before reporting completion. If isolation or a rebuild
is actually required, establish the concrete reason and explain it before acting.
