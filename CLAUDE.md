# Rusty Venture — Project Overview

## What this is
A Rust-based repository maturity scanner. It clones a git repository into an isolated Docker container, runs static analysis across multiple dimensions, and produces a scored maturity report (Bronze → Diamond). Both a CLI binary and an HTTP server expose the same analysis pipeline.

## Workspace layout
```
crates/
  rusty-venture-core/      # Traits, workflow engine, DAG engine, execution context
  rusty-venture-llm/       # LLM connector (Anthropic Claude via REST)
  rusty-venture-actions/   # All concrete actions + repo analysis workflow
  rusty-venture-store/     # SQLite persistence via SQLx
  rusty-venture-vcs/       # VCS provider integrations (GitHub, GitLab, Azure DevOps)
  rusty-venture-improve/   # Improvement pipeline (scaffold, commit, containerize)
  rusty-venture-cli/       # `rusty-venture` binary (clap)
  rusty-venture-server/    # `rusty-venture-server` binary (axum, SSE streaming)
runners/                   # Dockerfiles for the three analysis container images
frontend/                  # React + Three.js / React Three Fiber UI
```

## Critical architectural invariants
- **Container isolation**: all repo analysis runs inside Docker containers. Never analyse untrusted code on the host.
- **DooD (Docker-out-of-Docker)**: the backend container mounts the host Docker socket — `bollard` connects to the host daemon directly, no code changes needed.
- **Shared volume**: a named Docker volume (`rusty-venture-{run_id}`) is the only communication channel between the clone container and the analysis container.
- **Three runner images**: `rv-clone` (git + network), `rv-analysis` (POSIX tools, network OFF), `rv-exec` (language toolchains, network ON for package managers). Never mix their purposes.
- **Context is the message bus**: `ExecutionContext` (an `Arc<RwLock<HashMap>>`) is how actions share results. Every action writes its output to a well-known `CTX_*` key.

## Analysis pipeline — three phases
1. **Sequential** (clone): spawn-clone-container → clone-repo → [cache-repo-image] → spawn-analysis-container
2. **Parallel DAG** (analyse): detect-language, analyze-deps, find-dockerfiles, audit-files, governance-check, detect-llm-config — all run concurrently via `DagEngine`
3. **Sequential** (report): [content-quality-check] → [spawn-execution-container → active-validation] → generate-report → cleanup-container

## Maturity tiers
- **Tier 1** (default): Static file presence / static code checks — runs entirely inside `rv-analysis` with no network.
- **Tier 2**: Content quality inspection — reads and analyses file contents in the same container.
- **Tier 3**: Active functional validation — spawns `rv-exec`, actually runs build/test commands.

## Key files to orient yourself
- `crates/rusty-venture-core/src/dag.rs` — parallel DAG engine (Kahn topological sort + JoinSet)
- `crates/rusty-venture-core/src/workflow.rs` — sequential workflow engine
- `crates/rusty-venture-actions/src/repo/mod.rs` — the three-phase `run_repo_analysis` entry point
- `crates/rusty-venture-actions/src/repo/maturity.rs` — scoring model (signals → dimensions → composite)
- `crates/rusty-venture-server/src/main.rs` — SSE streaming, tier gating, VCS webhook handling

## LLM config convention
Projects can place a `.llm-config` file at their root to declare their AI tooling:
```
LLM=CLAUDE
MODEL=claude-sonnet-4-6
```
Rusty Venture detects this and awards tier 3 maturity signals for projects that have structured AI context (CLAUDE.md files, model specifications, etc.).
