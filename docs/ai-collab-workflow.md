# AI Collaboration Workflow

Lightweight workflow for one human plus up to twenty coding agents across Claude and Codex using GitHub issues as the scheduling surface.

## Goal

Keep three things separate:

- spec docs in `docs/` are the design truth
- GitHub issues and labels track execution state
- PRs and issue comments carry implementation and review handoffs

GitHub is the system of record for work pickup. Do not rely on local "oldest actionable" heuristics alone.

## Source of truth

Use these rules consistently:

- `docs/*.md` defines architecture, constraints, and accepted design
- the GitHub milestone defines the initiative boundary
- the initiative issue defines current scope and links child slices
- slice issues define the next concrete implementation steps
- issue labels define workflow state and current owner
- PRs implement one slice or one tightly related set of changes

Do not use `docs/roadmap.md` as the live status tracker for day-to-day progress.
Keep `docs/roadmap.md` high-level only.

## Critical Docs

These docs are mandatory context for every `/issue` or `$issue` run:

- `docs/k8s.md` for Def -> Instance -> Controller architecture and system boundaries
- `docs/authority.md` for data ownership and source-of-truth rules
- `docs/performance.md` for hot-path patterns, anti-patterns, and review expectations

Do not treat them as optional background reading.
If a change conflicts with any of them, stop and reconcile the design before writing code.

## Workspaces

Each agent works in its own independent clone to avoid file-level conflicts with other agents.

- Agent workspace: `C:\code\endless-{agentId}` (e.g., `C:\code\endless-claude-1`)
- Created as a full `git clone` of the GitHub repo
- Each agent has full control of its own workspace -- no coordination needed for uncommitted files
- Any agent can checkout any branch -- no conflicts between agents
- The main repo at `C:\code\endless` is for human use only; agents never work there directly

Workspace setup (run once per agent, handled by `/issue` on first use):

```
git clone https://github.com/abix-/endless.git C:\code\endless-{agentId}
cd C:\code\endless-{agentId}
git checkout dev
```

If the workspace directory already exists, reuse it. Do not recreate or remove existing workspaces.

### Shared build target

All workspaces share one Cargo target directory (`C:\code\endless\rust\target`) via `~/.cargo/config.toml`. Dependencies compile once; only the `endless` crate rebuilds per workspace (~16s vs ~7min cold).

Because concurrent builds to the same target dir can clobber artifacts, agents must use `k3sc cargo-lock` instead of bare `cargo` for all cargo commands:

```
k3sc cargo-lock build --release
k3sc cargo-lock clippy --release -- -D warnings
k3sc cargo-lock check
k3sc cargo-lock test --release -- <filter>
k3sc cargo-lock run --release
k3sc cargo-lock fmt
```

The lock serializes builds -- one agent builds while others wait in line. `test` and `run` automatically build first under lock.

## Branches and PRs

Do not commit implementation work directly to `main` or directly to `dev`.

Use these rules consistently:

- each issue gets its own branch: `issue-{N}` (e.g., `issue-34`)
- branch from `dev` when starting work: `git checkout -b issue-{N} origin/dev`
- if the branch `issue-{N}` already exists (from a previous handoff), check it out and rebase onto `origin/dev`
- push the branch and open a PR targeting `dev`
- before any handoff or review request, verify the branch is on GitHub as `origin/issue-{N}`; never hand off unpushed local commits
- issue comments remain the handoff channel; include the PR link in the handoff comment
- the reviewing agent fetches and checks out the same remote `issue-{N}` branch in their own workspace to review
- merge the PR after reviewer signoff and required tests pass
- do not use `git stash`, `git checkout dev`, or `git clean` to move aside work -- each agent owns their workspace

## Labels

Create and use these labels in GitHub.

Type labels:

- `feature`
- `bug`
- `test`

State labels:

- `ready`
- `needs-review`
- `needs-human`
- `waiting`

Owner labels (the owner label IS the claim -- no separate `claimed` label):

- `claude-1` through `claude-20` (Windows Claude agents via `k3sc launch`)
- `claude-a` through `claude-z` (k3s Claude agents via operator)
- `codex-1` through `codex-20` (Windows Codex agents via `k3sc launch codex`)
- `codex-a` through `codex-z` (k3s Codex agents via operator)

Suggested usage:

- `feature`: new functionality, behavior changes, or doc-driven implementation slices
- `bug`: defect fixes and regression work
- `test`: test-only or verification follow-up work
- `ready`: unclaimed and eligible for auto-pick
- owner label present (e.g. `claude-a`): actively being worked by that agent
- `needs-review`: implementation done, waiting for any different agent to review
- `needs-human`: agent work is done, waiting for human action (merge, close, design decision)
- `waiting`: blocked on an external decision or prerequisite, never auto-picked

Closed issues represent done. Do not add a `done` label.

## State Machine

Use this strict issue-state model:

- `ready`: issue is eligible for auto-pick
- owner label present: one specific agent is actively working it
- `needs-review`: implementation done, waiting for any different agent to review
- `waiting`: blocked on an external decision or prerequisite

Required invariants:

- the k3sc operator is the ONLY entity that adds or removes workflow labels and owner labels
- agents do NOT touch labels -- the operator handles all transitions
- auto-pick considers open issues labeled `needs-review` first, then `ready`
- each owner label should appear on at most one open issue
- reviewers never review an issue they most recently worked or implemented
- an issue has exactly one workflow state: `ready`, owner label, `needs-review`, `needs-human`, or `waiting`

## Agent Identity

Agent identity is derived from the workspace directory path. No registration scripts or settings files.

- k3s agents: `/workspaces/endless-claude-a` -> `claude-a` (letters, assigned by operator)
- Windows Claude agents: `C:\code\claude-1` -> `claude-1` (numbers, assigned by `k3sc launch`)
- Windows Codex agents: `C:\code\codex-1` -> `codex-1` (numbers, assigned by `k3sc launch codex`)

The k3sc operator assigns agent identities and slots. Agents do not self-register.

## Initiative Issue

The initiative issue should contain:

- the milestone name
- the canonical spec doc path
- the goal in one short paragraph
- the agreed slice list
- links to child issues
- a short "current focus" section

Pin the initiative issue in the repo if GitHub pin slots are available.

Initiative issues are trackers, not implementation-complete by default.

- do not close an initiative issue just because its body or spec links were corrected
- close an initiative issue only when its own acceptance criteria are actually satisfied
- before closing an initiative issue, verify the linked child slices are closed, superseded, or explicitly no longer needed
- if initiative housekeeping is done but implementation slices remain open, keep the initiative open and move it to `waiting` unless another immediate initiative-level action is explicitly assigned

Example structure:

```md
## Canonical Spec
- [docs/npc-activity-controller.md](../blob/main/docs/npc-activity-controller.md)

## Goal
Refactor NPC behavior into `Activity.kind + Activity.phase + Activity.target`
with one authoritative decision system.

## Slice Issues
- #123 Slice 1: Rest + Heal
- #124 Slice 2: Patrol + SquadAttack
- #125 Slice 3: Work + Mine + ReturnLoot + Raid
- #126 Tests and BRP follow-up

## Current Focus
- Human: review and direction
- codex-1: review and test follow-up
- claude-1: implementation
```

## Slice Issue Rules

Each slice issue should be small enough to finish with one focused PR or one short PR stack.

Each slice issue should include:

- milestone
- labels
- linked spec doc
- exact scope
- explicit acceptance criteria
- test requirements
- handoff notes section

Default state for a newly actionable slice is `ready`.

Do not mix open-ended design churn and implementation in the same issue unless the scope is tiny.
If the design is still moving, refine the spec doc first, then keep or move the implementation issue to `ready` once the work is concrete.

## Feature Spec Requirement

Every `feature` issue must have a spec doc before implementation begins.

### Spec doc rules

1. **Create before working**: when an agent creates a new feature issue (from a user request or epic breakdown), it must also write a spec doc in `docs/` and link it from the issue body.
2. **Spec doc contents**: the spec defines exactly how the feature works -- behavior, edge cases, data model, UI, and interactions with existing systems. It is the single source of truth for "done."
3. **Acceptance = spec compliance**: closing (or approving) a feature issue means the implementation matches the spec doc 100%. If the spec says X and the code does Y, that is a blocker.
4. **Spec-first, not code-first**: if the agent discovers during implementation that the spec needs changing, update the spec doc first, then adjust the code to match. Do not silently deviate from the spec.
5. **Reviewers verify against spec**: reviewers must read the spec doc and confirm the PR matches it. Approval without spec verification is invalid.
6. **Bug and test issues are exempt**: only `feature` label requires a spec doc. `bug` and `test` issues may use the issue body as the spec.
7. **Self-contained features**: if the feature is small enough that the issue body fully specifies behavior (no ambiguity, no design decisions left open), the issue body IS the spec. Add "Spec: self-contained in issue body" to the issue. Do not create an empty or redundant doc.

### Spec doc format

Path: `docs/{feature-name}.md`

Contents:

- **Goal**: one paragraph
- **Behavior**: what happens, step by step
- **Data model**: new components, constants, registry entries
- **Edge cases**: what happens when X fails, when Y is empty, etc.
- **UI**: what the player sees and interacts with
- **Integration**: how this feature interacts with existing systems
- **Acceptance criteria**: checklist (mirrors the issue checkboxes)

## Dispatch

The k3sc operator handles ALL issue assignment and label management:

1. Operator scans GitHub for eligible issues (`ready` and `needs-review`)
2. Operator assigns an agent slot and adds the owner label
3. Operator creates a k8s Job that launches the agent pod
4. Agent works the issue -- writes code, creates PRs, pushes branches
5. Agent pod exits
6. Operator detects completion, posts result comment, transitions labels:
   - Success from `ready` -> `needs-review`
   - Success from `needs-review` -> `needs-human`
   - Failure from `ready` -> `ready` (retry, max 3)
   - Failure from `needs-review` -> `needs-human` (escalate to human, don't loop)

**Agents do NOT touch labels, claim issues, or register themselves.** The operator does all of this.

The agent's identity and assigned issue are passed via environment variables (`AGENT_SLOT`, `ISSUE_NUMBER`, `REPO_URL`).

## Comment Formats

Handoff comment (posted by agent before pod exits):

```md
## <AgentName>
- Changed: short factual summary
- Tests: commands run and result
- Acceptance: N/N criteria verified and checked | no checkboxes in issue body
- Open: blockers, risks, or unresolved questions
- Next: smallest sensible next step
```

The `Acceptance` line is mandatory. Before any handoff, the agent must read the issue body and verify each `- [ ]` checkbox against the code. Unchecked boxes = unverified work = invalid handoff.

**Label transitions are handled by the k3sc operator, not the agent.** Agents do NOT run `gh issue edit` to add/remove labels. The operator detects pod completion and transitions labels automatically.

Replace `<AgentName>` with `Codex` or `Claude`.

## Design Workflow

Use this flow when architecture is still moving:

1. Read the issue-linked spec plus `docs/k8s.md`, `docs/authority.md`, and `docs/performance.md`
2. Update the relevant spec doc in `docs/`
3. Comment on the initiative or slice issue with the design delta
4. Implementation starts when the operator dispatches it -- agents do not change issue state

If two agents both touch the same spec:

- one agent edits the doc
- the other agent reviews the doc and leaves findings
- the doc is the accepted result, not the issue comment

## Implementation Workflow

Use this flow for each slice:

1. Read the slice issue, the linked spec doc, and the critical docs: `docs/k8s.md`, `docs/authority.md`, `docs/performance.md`
2. The operator has already claimed the issue and assigned you. Start working.
3. In your agent workspace, create or checkout the issue branch:
   - new issue: `git fetch origin && git checkout -b issue-{N} origin/dev`
   - continuing work: `git checkout issue-{N} && git pull --rebase origin dev`
4. Implement the smallest complete step
5. **Write regression tests** -- every code change must have tests that would FAIL if the change were reverted. Bug fixes must reproduce the bug scenario. New features must cover core acceptance criteria. Refactors must verify behavior matches. Updating existing tests to compile with new names does NOT count. This is mandatory for ALL code changes -- no exceptions, no "will add later", no "too simple to test".
6. Run `cargo fmt` before committing
7. Run the required tests
8. Update docs if accepted behavior changed
9. **Verify ALL acceptance criteria** in the issue body are met before handing off. Check every checkbox against actual code. If any criterion is unmet, either implement it or document it as a blocker -- do not hand off claiming "done" with unmet criteria.
10. Push the branch and open or update the PR targeting `dev`:
    - `git push -u origin issue-{N}`
    - `git fetch origin && git rev-parse --verify origin/issue-{N}`
    - `gh pr create --base dev --head issue-{N}` (or update existing PR)
11. Leave the handoff comment with the PR link only after the remote branch verification passes

The operator handles the label transition when the pod completes. Do NOT add/remove labels.

This same handoff flow applies when a reviewing agent makes the fix instead of bouncing the issue back unchanged.

If work is genuinely blocked:

- leave the handoff comment explaining the blocker
- the operator will handle label cleanup when the pod exits

## Review Workflow

Default review split:

- the operator transitions the issue to `needs-review` after successful implementation
- the operator dispatches a different agent for review
- reviewers never review an issue they most recently implemented
- agents do NOT claim issues or change labels -- the operator handles assignment

To review, the reviewer checks out the `issue-{N}` branch in their own workspace:

```
git fetch origin
git checkout issue-{N}
```

Review must be against the remote handoff branch. Do not review from another agent's workspace or from commits that were not pushed to `origin/issue-{N}`.

If making fix-forward changes, push to the same `issue-{N}` branch. The PR updates automatically.

Review is fix-forward by default:

- if the reviewer finds a concrete in-scope problem, it should make the smallest complete fix in the same turn
- after making code changes during review, that agent becomes the implementing side -- the operator will transition labels when the pod exits
- do not spend a full turn on findings-only review when the fix is clear, local, and safely within scope
- use a findings-only handoff only when blocked, out of scope, design-ambiguous, or explicitly asked to review without changing code

Review should focus on:

- behavior regressions
- violations of `docs/k8s.md`
- authority violations
- violations of `docs/authority.md`
- **regression tests** -- every code change must have tests that fail if reverted. Bug fixes reproduce the bug. Features cover acceptance criteria. Refactors verify behavior. Existing tests updated to compile do NOT count. Missing regression tests = blocker.
- spec drift
- performance regressions and violations of `docs/performance.md`

If there are no findings and tests pass:

- verify ALL acceptance criteria in the issue body are met -- every checkbox, no exceptions
- if any acceptance criterion is unmet, that is a blocker regardless of whether the code compiles and tests pass
- an issue with 11/12 criteria met is NOT ready for merge -- 100% or nothing
- include a pass/fail table in the handoff comment showing each acceptance criterion and its status
- only after all criteria pass: leave the handoff comment noting approval
- the operator handles label transitions and the human merges the PR

Initiative issue exception:

- for initiative or epic tracker issues, "no findings" on the issue body is not enough to close
- only close an initiative when the acceptance is satisfied and downstream slice work is complete
- if the initiative body is now correct but the acceptance is still unmet, leave the handoff comment with the remaining items in `Open`

If review finds a blocker:

- leave the handoff comment documenting the blocker
- the operator handles label transitions when the pod exits

## Shared Skill

Use the shared workflow skill when you want either agent family to pick up one issue and carry it through the current workflow without extra prompting:

- Claude: `/issue 3`
- Codex: `$issue 3`
- No argument: the k3sc operator dispatches the next eligible issue automatically

Expected behavior:

- read this workflow doc, the target issue, the canonical spec, the latest handoff comments, and the critical docs (`docs/k8s.md`, `docs/authority.md`, `docs/performance.md`)
- respect the state-machine and ownership rules above
- work in the agent's own workspace
- create or checkout `issue-{N}` branch from `dev`
- perform the smallest complete next step
- run the required tests
- push the branch, verify `origin/issue-{N}`, and open or update the PR targeting `dev`
- leave a GitHub handoff comment with the PR link
- do NOT touch labels -- the operator handles all transitions when the pod exits

## Progress Tracking

Use these signals, in order:

1. milestone completion
2. open vs closed slice issues
3. issue state labels (`ready`, `needs-review`, `waiting`) and owner labels (`claude-*`)
4. latest handoff comments

This is enough for a one-person project with up to twenty agents unless coordination starts breaking down.

## When to Move to GitHub Projects

Adopt a Project board only if one or more of these start happening:

- progress is hard to read from milestones and issue labels
- multiple agents still step on each other despite the operator's slot assignment
- blockers vanish inside issue comments
- `docs/roadmap.md` starts turning into a live tracker again
- the initiative has too many concurrent slices to follow comfortably

Until then, stay with milestone + issues + labels + docs.
