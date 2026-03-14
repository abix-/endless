# AI Collaboration Workflow

Lightweight workflow for running design and implementation with one human plus two coding agents without turning `docs/roadmap.md` into a live status board.

## Goal

Keep three things separate:

- spec docs in `docs/` are the design truth
- GitHub issues and milestones track execution state
- PRs and issue comments carry agent handoffs

This is intentionally lighter than GitHub Projects. Use Projects only if this process stops being enough.

## Default setup

For each major initiative:

1. Create one milestone.
   Example: `NPC Activity Controller`
2. Create one initiative issue.
   This is the top-level tracker for the milestone.
3. Create a small set of slice issues.
   Target size: `4` to `8` issues total for the initiative.
4. Link the canonical spec doc from the initiative issue.
5. Use issue comments for Codex and Claude handoffs.

## Source of truth

Use these rules consistently:

- `docs/*.md` defines architecture, constraints, and accepted design
- the GitHub milestone defines the initiative boundary
- the initiative issue defines current scope and links all slice issues
- slice issues define the next concrete implementation steps
- PRs implement one slice or one tightly related set of changes

Do not use `docs/roadmap.md` as the live status tracker for day-to-day progress.
Keep `docs/roadmap.md` high-level only.

## Branches and PRs

Do not commit implementation work directly to `main`.

Use these rules consistently:

- one slice issue = one branch
- open a PR for any code or accepted-doc change before asking the other agent to review
- issue comments remain the handoff channel; PRs are the code review surface
- merge only after reviewer signoff and required tests pass

## Labels

Create these lightweight labels in GitHub:

- `design`
- `code`
- `testing`
- `waiting`
- `codex`
- `claude`
- `shared`
- `needs-review`

Suggested usage:

- `design`: doc refinement, design decisions, architecture changes
- `code`: code changes for a slice
- `testing`: test-only or test-follow-up work
- `waiting`: waiting on a decision or prerequisite
- `codex`: current primary agent is Codex
- `claude`: current primary agent is Claude
- `shared`: both agents are expected to touch it
- `needs-review`: ready for review or needs a review pass

## Initiative issue

The initiative issue should contain:

- the milestone name
- the canonical spec doc path
- the goal in one short paragraph
- the agreed slice list
- links to child issues
- a short "current focus" section

Pin the initiative issue in the repo if GitHub pin slots are available.

Example structure:

```md
## Canonical Spec
- [docs/npc-activity-controller.md](../blob/main/docs/npc-activity-controller.md)

## Goal
Refactor NPC behavior into `Activity.kind + Activity.phase + Activity.target`
with one authoritative decision system.

## Slices
- #123 Slice 1: Rest + Heal
- #124 Slice 2: Patrol + SquadAttack
- #125 Slice 3: Work + Mine + ReturnLoot + Raid
- #126 Tests and BRP follow-up

## Current Focus
- Claude: Slice 1 implementation
- Codex: review + spec tightening
```

## Slice issue rules

Each slice issue should be small enough to finish with one focused PR or one short PR stack.

Each slice issue should include:

- milestone
- labels
- linked spec doc
- exact scope
- explicit acceptance criteria
- test requirements
- handoff notes section

Do not mix speculative design and implementation in the same issue unless the scope is tiny.
If design is still moving, use a `design` issue first, then open or update the `code` issue once the doc is settled.

## Agent handoffs

Issue comments are the handoff channel between Codex and Claude.

Use this exact comment shape:

```md
## Handoff
- Changed: short factual summary
- Tests: commands run and result
- Open: blockers, risks, or unresolved questions
- Next: smallest sensible next step
```

Rules:

- leave a handoff comment whenever stopping with unfinished work
- leave a handoff comment after any meaningful spec refinement
- leave a handoff comment after review, even if no code changed
- keep handoff comments short and factual

## Design workflow

Use this flow when the architecture is still moving:

1. Update the relevant spec doc in `docs/`
2. Comment on the initiative or slice issue with the design delta
3. Get the design to a stable enough point that implementation can target it
4. Only then move the implementation issue to active work

If Codex and Claude both touch the same spec:

- one agent edits the doc
- the other agent reviews the doc and leaves findings
- the doc is the accepted result, not the issue comment

## Implementation workflow

Use this flow for each slice:

1. Read the spec doc and the slice issue
2. Create or update the slice branch
3. Implement the smallest complete step
4. Run the required tests
5. Update docs if the implementation changed accepted behavior
6. Open or update the PR
7. Leave a handoff comment on the issue with the PR link or branch name

Preferred ownership pattern:

- one active slice per agent at a time
- avoid both agents editing the same implementation slice unless one is explicitly reviewing

## Review workflow

Default review split:

- Claude or Codex implements
- the other agent reviews the PR before merge
- the implementing agent may mark the issue `needs-review`, but may not clear that label or self-approve closure
- only the non-implementing agent may say the slice is ready to close or can lose `needs-review`
- a slice issue closes only after reviewer signoff and required tests pass

Review should focus on:

- behavior regressions
- authority violations
- missing tests
- spec drift

If review finds new design problems:

- update the spec doc first
- then fix the code against the updated spec

## Shared skill

Use the shared workflow skill when you want either agent to pick up one issue and carry it through the current workflow without extra prompting:

- Claude: `/ai-collab 3`
- Codex: `$ai-collab 3`

Expected behavior:

- read this workflow doc, the target issue, the canonical spec, and the latest handoff comments
- decide whether the current agent should implement or review
- work on the issue branch or create one if implementation starts from issue-only state
- perform the smallest complete next step
- run the required tests
- open or update the PR before handing off implementation work
- leave a GitHub handoff comment
- respect the reviewer gate above instead of self-approving closure

## Progress tracking

Use these signals, in order:

1. milestone completion
2. open vs closed slice issues
3. initiative issue current-focus section
4. latest handoff comments

This is enough for a one-person project with two agents unless coordination starts breaking down.

## When to move to GitHub Projects

Adopt a Project board only if one or more of these start happening:

- progress is hard to read from the milestone and issues
- Codex and Claude step on each other regularly
- blockers vanish inside issue comments
- `docs/roadmap.md` starts turning into a live tracker again
- the initiative has too many concurrent slices to follow comfortably

Until then, stay with milestone + issues + docs.
