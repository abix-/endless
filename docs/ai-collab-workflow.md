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

## Branches and PRs

Do not commit implementation work directly to `main`.

Use these rules consistently:

- `ai-collab` works on the shared `dev` branch
- do not create or switch to issue-specific branches or worktrees during normal `ai-collab` runs
- if the current branch is not `dev`, stop and resolve that before making code changes
- push reviewed changes to `dev`
- issue comments remain the handoff channel; use an existing `dev` PR as the review surface when one exists
- merge only after reviewer signoff and required tests pass

## Labels

Create and use these labels in GitHub.

Type labels:

- `feature`
- `bug`
- `test`

State labels:

- `ready`
- `claimed`
- `needs-claude`
- `needs-codex`
- `waiting`

Owner labels:

- `claude-1` through `claude-10`
- `codex-1` through `codex-10`

Suggested usage:

- `feature`: new functionality, behavior changes, or doc-driven implementation slices
- `bug`: defect fixes and regression work
- `test`: test-only or verification follow-up work
- `ready`: unclaimed and eligible for auto-pick
- `claimed`: actively being worked by exactly one agent identity, including active review
- `needs-claude`: waiting for the Claude family to take the next active step
- `needs-codex`: waiting for the Codex family to take the next active step
- `waiting`: blocked, never auto-picked

Closed issues represent done. Do not add a `done` label.

## State Machine

Use this strict issue-state model:

- `ready`: issue is eligible for auto-pick
- `claimed`: one specific agent identity is actively working it
- `needs-claude`: the next active step belongs to the Claude family
- `needs-codex`: the next active step belongs to the Codex family
- `waiting`: blocked on an external decision or prerequisite

Required invariants:

- each open issue carries exactly one state label from this list
- auto-pick for Claude considers open issues labeled `needs-claude` first, then `ready`
- auto-pick for Codex considers open issues labeled `needs-codex` first, then `ready`
- no-argument `ai-collab` must first look for open issues already labeled `claimed` with the current owner label and resume the oldest one instead of claiming a new issue
- auto-pick must ignore any issue labeled `waiting` or `claimed`
- `claimed` requires exactly one owner label
- each owner label should appear on at most one open issue; if an owner already has multiple open claimed issues, resume the oldest and do not claim another
- `needs-claude` must remove `claimed`, `ready`, `needs-codex`, and all owner labels
- `needs-codex` must remove `claimed`, `ready`, `needs-claude`, and all owner labels
- `waiting` must remove `claimed` and all owner labels
- agents must convert `needs-claude` or `needs-codex` to `claimed` before starting review or follow-up work
- reviewers never review an issue they most recently claimed or implemented

## Agent Identity

The shared `ai-collab` workflow requires a stable disk-backed registry at `C:/Users/Abix/.claude/ai-collab/settings.json`.

Expected shape:

```json
{
  "version": 2,
  "slots": {
    "claude": ["claude-1", "claude-2", "claude-3", "claude-4", "claude-5", "claude-6", "claude-7", "claude-8", "claude-9", "claude-10"],
    "codex": ["codex-1", "codex-2", "codex-3", "codex-4", "codex-5", "codex-6", "codex-7", "codex-8", "codex-9", "codex-10"]
  },
  "claims": {
    "claude-1": {
      "family": "claude",
      "pid": 40216,
      "process_name": "claude",
      "session_id": "abc123",
      "workspace": "C:/code/endless",
      "claimed_at": "2026-03-14T17:00:00.0000000Z"
    }
  }
}
```

Claim rules:

- each Claude or Codex instance claims one configured slot by PID
- a claim is valid only while the PID still exists and the process name still matches
- stale claims are removed before allocation
- a live process reuses its existing claimed slot if one already exists
- otherwise it takes the first free configured slot for its family

MVP behavior:

- registration happens only when `/ai-collab` or `$ai-collab` runs
- Claude registers itself by running `C:/Users/Abix/.claude/ai-collab/Register-AiCollabAgent.ps1 -Family claude`
- Codex registers itself by running `C:/Users/Abix/.claude/ai-collab/Register-AiCollabAgent.ps1 -Family codex`
- the returned `agentId` is the owner label to use for the rest of that skill run

If the file is missing, malformed, has no valid slots for the current family, or no live claim can be obtained, the skill must fail fast instead of guessing.

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

## Claim Protocol

Running `/ai-collab` or `$ai-collab` with no issue number means "claim the next eligible issue for this agent family."

No-argument claim algorithm:

1. Read this workflow doc.
2. Register the current process with `C:/Users/Abix/.claude/ai-collab/Register-AiCollabAgent.ps1` and use the returned `agentId`.
3. Derive the family handoff label from the current agent family:
   - Claude -> `needs-claude`
   - Codex -> `needs-codex`
4. List open issues ordered oldest-first.
5. Look first for the oldest open issue already labeled `claimed` with the current owner label.
6. If one exists, resume that issue and do not claim a new one.
7. If none exists, look for the oldest issue labeled with the current family handoff label and not labeled `waiting` or `claimed`.
8. If no family handoff issue exists, look for the oldest issue labeled `ready` and not labeled `waiting`, `claimed`, `needs-claude`, or `needs-codex`.
9. Attempt to claim the first new candidate by:
   - removing `ready` or the matching family handoff label
   - adding `claimed`
   - adding exactly one owner label for the current agent identity
   - posting the claim comment format below
10. Re-read the issue and confirm:
   - `claimed` is present
   - `ready`, `needs-claude`, and `needs-codex` are absent
   - exactly one owner label is present
   - the owner label matches the current agent identity
11. If claim confirmation fails, continue to the next candidate or exit cleanly if none remain.

Claims do not expire automatically.
A claim stays active until that agent finishes the current workflow step and changes labels as part of handoff.

## Explicit Issue Selection

If an issue number is provided:

- if the issue is `ready`, claim it before starting work
- if the issue is `needs-claude`, only Claude may claim it, and Claude must convert it to `claimed` before starting work
- if the issue is `needs-codex`, only Codex may claim it, and Codex must convert it to `claimed` before starting work
- if the issue is `claimed` by another owner label, do not act on it
- if the issue is `waiting`, do not proceed without first resolving the blocker

## Comment Formats

Claim comment:

```md
## <AgentName>
- State: <previous-state> -> claimed
- Owner: <agent-id>
- Intent: implement | review
- Next: smallest immediate step
```

Use the actual previous state: `ready`, `needs-claude`, or `needs-codex`.

Implementation or review handoff:

```md
## <AgentName>
- Changed: short factual summary
- Tests: commands run and result
- Open: blockers, risks, or unresolved questions
- State: claimed -> needs-claude | claimed -> needs-codex | claimed -> waiting | claimed -> close
- Next: smallest sensible next step
```

Replace `<AgentName>` with `Codex` or `Claude`.
Choose `needs-claude` or `needs-codex` for whichever family owns the next step.

## Design Workflow

Use this flow when architecture is still moving:

1. Update the relevant spec doc in `docs/`
2. Comment on the initiative or slice issue with the design delta
3. Keep or move the issue to `ready` only when implementation can start cleanly

If two agents both touch the same spec:

- one agent edits the doc
- the other agent reviews the doc and leaves findings
- the doc is the accepted result, not the issue comment

## Implementation Workflow

Use this flow for each slice:

1. Read the spec doc and the slice issue
2. Claim the issue if it is `ready`
3. Create or update the slice branch
4. Implement the smallest complete step
5. Run the required tests
6. Update docs if accepted behavior changed
7. Open or update the PR
8. Leave the handoff comment
9. Remove `claimed` and the owner label, then add the opposite family handoff label:
   - Codex implementation -> `needs-claude`
   - Claude implementation -> `needs-codex`

This same handoff flow applies when a reviewing agent makes the fix instead of bouncing the issue back unchanged.

If work is genuinely blocked:

- leave the handoff comment
- remove `claimed` and the owner label
- add `waiting`

## Review Workflow

Default review split:

- one family makes the latest code-changing step
- the other family reviews
- the implementing family moves the issue to the other family's handoff label when asking for review
- the reviewing family must claim the issue before starting review
- only the family that did not make the latest code-changing step may close the issue

Review is fix-forward by default:

- if the reviewing family finds a concrete in-scope problem, it should make the smallest complete fix in the same turn
- after making code changes during review, that family becomes the implementing side for the latest step and must hand the issue back to the other family
- do not spend a full turn on findings-only review when the fix is clear, local, and safely within scope
- use a findings-only handoff only when blocked, out of scope, design-ambiguous, or explicitly asked to review without changing code

Review should focus on:

- behavior regressions
- authority violations
- missing tests
- spec drift

If there are no findings and tests pass:

- leave the handoff comment with `State: claimed -> close`
- close the issue

Initiative issue exception:

- for initiative or epic tracker issues, "no findings" on the issue body is not enough to close
- only use `claimed -> close` when the initiative acceptance is satisfied and downstream slice work is complete
- if the initiative body is now correct but the acceptance is still unmet, leave the handoff comment with `State: claimed -> waiting`, list the remaining slice issues or unmet acceptance items in `Open`, remove `claimed` and the owner label, and add `waiting`

If review finds a blocker:

- leave the handoff comment with `State: claimed -> needs-claude` or `claimed -> needs-codex` for the family expected to follow up
- remove `claimed` and the owner label
- add the target family handoff label

## Shared Skill

Use the shared workflow skill when you want either agent family to pick up one issue and carry it through the current workflow without extra prompting:

- Claude: `/ai-collab 3`
- Codex: `$ai-collab 3`
- No argument: claim the next eligible `needs-<your-family>` issue, otherwise the next eligible `ready` issue, using the current process claim from `C:/Users/Abix/.claude/ai-collab/settings.json`

Expected behavior:

- read this workflow doc, the target issue, the canonical spec, and the latest handoff comments
- respect the state-machine and ownership rules above
- claim `ready` and family-targeted handoff issues before starting work
- perform the smallest complete next step
- run the required tests
- open or update the PR before handing off implementation work
- leave a GitHub handoff comment
- transition labels immediately as part of the handoff

## Progress Tracking

Use these signals, in order:

1. milestone completion
2. open vs closed slice issues
3. issue state labels (`ready`, `claimed`, `needs-claude`, `needs-codex`, `waiting`)
4. latest handoff comments

This is enough for a one-person project with up to twenty agents unless coordination starts breaking down.

## When to Move to GitHub Projects

Adopt a Project board only if one or more of these start happening:

- progress is hard to read from milestones and issue labels
- multiple agents still step on each other despite the claim protocol
- blockers vanish inside issue comments
- `docs/roadmap.md` starts turning into a live tracker again
- the initiative has too many concurrent slices to follow comfortably

Until then, stay with milestone + issues + labels + docs.
