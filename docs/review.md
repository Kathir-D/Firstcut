# Review board

- **Owner:** senior-dev (**only senior-dev edits this file**)
- **Every agent:** read this at the start and end of every work session. Fix open findings in your
  section and in **All agents**, highest severity first. Respond in your own status file's
  **Incoming requests** table: `REV-<n>` · `fixed in <commit>` or `disputed: reason`.
- Severity: **P0** blocker (stop and fix now; blocks merges in your area) · **P1** must fix before the
  wave gate · **P2** fix within the next wave · **P3** nit. Details: [task.md §0.4](../task.md#04-protocol).

_Last review pass: — (not started)_

## Wave gates

| Wave | Status | Signed off | Notes |
| --- | --- | --- | --- |
| 1. Foundations | not started | | |
| 2. Real data | not started | | |
| 3. Features | not started | | |
| 4. Polish & ship (v0.1.0) | not started | | |

## Contract approvals

| Contract | Version | Status | Notes |
| --- | --- | --- | --- |
| build.md | v0.1 | draft, not reviewed | |
| photo-meta.md | v0.1 | draft, not reviewed | |
| batching.md | v0.1 | draft, not reviewed | |
| session-api.md | v0.1 | draft, not reviewed | |
| pipeline-api.md | v0.1 | draft, not reviewed | |
| app-model.md | v0.1 | draft, not reviewed | |

## Open findings

Format: `| REV-n | P0–P3 | location | what's wrong | what to change | status |`
Status: `open` · `fixed, verifying` · `reopened` · `disputed`

### All agents

| ID | Sev | Location | Finding | Required change | Status |
| --- | --- | --- | --- | --- | --- |

### infra

| ID | Sev | Location | Finding | Required change | Status |
| --- | --- | --- | --- | --- | --- |

### core-meta

| ID | Sev | Location | Finding | Required change | Status |
| --- | --- | --- | --- | --- | --- |

### core-batch

| ID | Sev | Location | Finding | Required change | Status |
| --- | --- | --- | --- | --- | --- |

### core-store

| ID | Sev | Location | Finding | Required change | Status |
| --- | --- | --- | --- | --- | --- |

### pipeline

| ID | Sev | Location | Finding | Required change | Status |
| --- | --- | --- | --- | --- | --- |

### app-logic

| ID | Sev | Location | Finding | Required change | Status |
| --- | --- | --- | --- | --- | --- |

### ui

| ID | Sev | Location | Finding | Required change | Status |
| --- | --- | --- | --- | --- | --- |

### qa

| ID | Sev | Location | Finding | Required change | Status |
| --- | --- | --- | --- | --- | --- |

## Closed findings

| ID | Agent | Sev | Finding | Closed in | Verified |
| --- | --- | --- | --- | --- | --- |
