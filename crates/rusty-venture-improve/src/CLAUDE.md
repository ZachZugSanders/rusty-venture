# rusty-venture-improve/src

## Purpose
The improvement pipeline crate. Two distinct subsystems live here:

1. **VCS-API improvements** (`commit`, `containerize`, `dependents`, `gate`, `governance`, `workflow`) — generate files and commit them to GitHub/GitLab via the provider API. No Docker, no container exec. Used when you have a repo URL but no local clone.

2. **Container-based changes** (`change/`) — apply repairs to a **locally cloned repo** inside a running Docker container using git directly. Used when a container with the repo already exists (e.g. after a Tier 1/2/3 scan).

## Module map

| Module | Purpose |
|---|---|
| `change/` | Container-based repo repair — see `change/CLAUDE.md` |
| `commit.rs` | `CreateBranchAction` — commits generated files via VCS provider API |
| `containerize.rs` | `ContainerizeAction` — LLM-generates a Dockerfile, validates with docker build |
| `dependents.rs` | `DiscoverDependentsAction` — finds repos that depend on this one via VCS API |
| `gate.rs` | `DependencyDecisionGate` — waits for human decision before proceeding |
| `governance.rs` | `GenerateGovernanceFilesAction` — LLM-generates LICENSE, SECURITY.md, etc. |
| `workflow.rs` | `ImprovementWorkflowBuilder` — builds a `DagWorkflow` from a `ScaffoldSpec` |

## Public API (`lib.rs` re-exports)
All public types are re-exported from the crate root. Consumers import from `rusty_venture_improve::` directly — never from submodules.

## What NOT to put here
- Analysis logic → `rusty-venture-actions/src/repo/`
- Core traits / workflow engine → `rusty-venture-core`
- LLM HTTP calls → `rusty-venture-llm`
- VCS provider API calls → `rusty-venture-vcs`
