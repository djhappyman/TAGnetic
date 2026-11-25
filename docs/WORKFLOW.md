# Contribution workflow

Agents and orchestrators must read this file before planning, delegation, commit
proposals, PR work or branch retirement. Explicit user instructions take priority.
This is an agent working agreement; it does not install an automatic GitHub bridge.

## 1. Scope the change

Record the intended behavior, the files owned, the completion condition, required
verification and publication target in the task/PR description. One branch
should deliver one independently reviewable concern. Workers implement their
assigned scope; the orchestrator reviews, resolves integration issues and verifies
the combined result. Delegation requires authorization from the current task.

For interface styling, read [Interface rules](design/interface-rules.md) before
changing surfaces, borders, inputs or focus treatments.

For new app work, follow Mike's choice of checkout. If he has not specified one,
ask whether to keep the changes on `main` in `personal-app` or use a separate
worktree. For a new branch, check the remote URL and use the latest required
fork checkpoint as its base. Preserve other threads' edits.

Keep at most three active personal feature branches by default. Historical
contribution branches are recovery material, not an active upstream work queue.
Complete or explicitly park an existing personal task before starting a fourth.
A parked task must have a recovery ref and a short resumption note.

## 2. Reach a feature checkpoint

Before pushing a candidate, review the complete diff against its requested
behavior and inspect each affected platform branch. Resolve requested independent
review findings and run the permitted lightweight formatting, source/configuration
and workflow/script syntax checks. For a substantive defect, add a regression
that detects its specific old failure and proves the corrected behavior,
including relevant cancellation, contention and recovery boundaries.

Respect the owner's workload restrictions. When runtime verification is
hosted-only, push a reviewed candidate for focused preflight and label it
**CI pending**. Pass the focused regression/lint/pressure gate before starting
the expensive full platform and installer matrix. Publication uses that exact
verified commit; changed source requires fresh gates. A rerun alone is not proof
that a defect was corrected. Installed-package smoke checks are a separate gate
when available; build/upload success proves only its checked scope.

Offer a commit message only when the scoped feature or code change is complete:

- The intended behavior works and the diff contains only that concern.
- Appropriate checks have actually run on the candidate and their results are read.
- Review findings are resolved, or accepted limitations are explicitly recorded.
- User-visible changes preserve the required features and interaction boundaries.
  Ask for user judgment only where the task requires it or the intended behavior
  cannot reasonably be determined. Existing authorization remains valid.

At that point report `Checkpoint ready`, the completed behavior, verification
evidence and one feature-focused commit subject. Commit if the user has already
authorized commits; otherwise present the proposal. Keep incomplete iterations
in the working branch and describe what remains without proposing a completion
commit. An explicitly requested recovery/WIP commit is labeled as such and is
not a completed-feature checkpoint.

A large feature can have several checkpoints when each one is coherent, tested
and independently reviewable. Commit around behavior, not elapsed time or every
agent handoff. Keep private audit/history and generated build output out of PRs.

## 3. Review and publish

Open a focused PR into `djhappyman/TAGnetic:main`. Explain the problem, resulting
behavior, verification and material risks. A local passing result permits
publication; merge readiness requires hosted results on the exact pushed head.

Require Linux `check`, `windows`, `macos` and all applicable audit/check results to
pass. Pending, missing, cancelled, skipped and approval-required results remain
unverified. Inspect failed logs, fix the cause and verify the new pushed head.
Use the workspace's `tools/check-pr-ci.py` gate when available; otherwise inspect
the complete current-head matrix directly.

Where the host provides event-driven PR watches, retain one while checks/reviews
are pending. Read old comments before watching. Link newly created/pushed PRs as
required by the host. A new push invalidates an old head's green result.

## 4. Fork-only work and upstream intake

Features, fixes and releases target `djhappyman/TAGnetic`. Upstream contributions
are discontinued: ordinary fork work must not prepare, open, update or submit
upstream PRs.

Bring changes from upstream only when the user explicitly requests that intake.
Inspect the requested refs, curate compatible changes into a fork branch, and
apply the same review, verification and release gates. Preserve this fork's
identity, storage compatibility, settings and Undo history.

Historical upstream notes and PR links in [UPSTREAM.md](UPSTREAM.md) record
provenance; they do not authorize further contribution work.

## 5. Merge and retire

Before merging, check current base/head, review disposition, full hosted gate and
remaining task-specific acceptance criteria. Merge within existing authorization.
For a stack, preserve ancestry, retarget the next layer and verify its new base
and checks. Remove only branches whose final heads have been accounted for.

Retire a merged personal branch after confirming its tip is included in the
merged history. For squash/adapted changes, retain a recovery bundle/ref and
verify the integration before retirement. Preserve historical contribution refs
as recovery material unless their removal is explicitly requested.
Check for uncommitted work and dependent branches/worktrees before local cleanup;
never force-delete active work. Record retired branch names, SHAs and recovery
location. Delete remote refs with their expected SHA to avoid removing new work.

Update branch/PR status when work merges. Review the active personal branch list
after each merge; avoid creating a new checkpoint branch for every minor edit.

## 6. Verify delivery

A green PR proves its checked scope. A merged `main` commit also needs its
[Build and release](BUILDING.md) validation and package matrix checked before
installers are described as available or a release milestone is complete.

Use independent numeric app versions and `personal-v<VERSION>` tags. When Mike
requests a build, prepare a personal release by default. Check whether the
configured version is already released: if so, prepare a patch-version bump in
the app config, workspace manifest/lockfile and changelog as a release checkpoint,
then apply the normal review/CI/merge gates. Dispatch **Build and release** on the
merged fork `main` with `publish_release` enabled. This validates and publishes
the full platform matrix. Use artifact-only mode only when requested. A normal
main push continues producing development artifacts without publishing a release.
Never overwrite an existing version's source tag or published assets.
Keep existing portable releases and state intact. After outputs are verified,
remove task-owned disposable build/probe output and retain compact evidence.

## Handoff

Report the completed behavior, exact commit/PR, verified checks, publication target
and remaining blockers. Use `Implementing`, `Checkpoint ready`, `CI pending`,
`Merged` or `Packages verified` accurately; each describes a different gate.
