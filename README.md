# rusty-venture

LLM-powered repository analysis for engineering maturity, security hygiene, and governance readiness.

`rusty-venture` scans a Git repository inside isolated Docker containers, computes structured risk + maturity signals, generates an actionable report with Claude, and stores scan history in SQLite. It includes:

- a Rust CLI (`rusty-venture`) for terminal workflows,
- a Rust API server (`rusty-venture-server`) for automation/UI integration,
- and a React/Vite frontend for interactive browsing of scan results.

---

## What it does

For each analyzed repository, the pipeline:

1. Clones the repo into an isolated Docker volume.
2. Detects primary language (with secondary language hints).
3. Parses dependencies / lock-file state.
4. Finds Dockerfiles and flags common container risks.
5. Audits tracked + present files against deny-list patterns (secrets, artifacts, etc.).
6. Checks governance/quality signals (README, LICENSE, CI, SECURITY.md, lint, tests, etc.).
7. Asks Claude to produce a structured final report.
8. Computes a weighted maturity score and grade.
9. Persists results to SQLite for history/trending.

---

## Maturity model (high level)

Composite maturity is a weighted score from six dimensions:

- **Security** (25%)
- **Dependency Health** (20%)
- **Build & CI** (15%)
- **Code Organization** (15%)
- **Project Governance** (15%)
- **Testing & Quality** (10%)

Grade mapping:

- `0–20`: **BRONZE**
- `21–40`: **SILVER**
- `41–60`: **GOLD**
- `61–80`: **PLATINUM**
- `81–100`: **DIAMOND**

---

## Supported repository stacks

### Language detection

Detects markers for:

- Rust
- Node.js
- Python
- Go
- Java
- Ruby
- PHP
- C#
- Swift
- Kotlin

### Dependency parsing

Dependency analyzers currently implemented for:

- Rust (`Cargo.toml`)
- Node.js (`package.json`)
- Python (`pyproject.toml` / `requirements.txt`)
- Go (`go.mod`)
- Java (`pom.xml` metadata capture)
- Ruby (`Gemfile`)
- PHP (`composer.json`)

---

## Workspace layout

This repository is a Cargo workspace with focused crates:

- `crates/rusty-venture-core` — workflow engine, retry/validation, execution context, DAG runtime.
- `crates/rusty-venture-llm` — LLM connector abstraction + Anthropic Claude integration.
- `crates/rusty-venture-actions` — concrete analysis actions and orchestration entrypoints.
- `crates/rusty-venture-store` — SQLite access layer, migrations, query models.
- `crates/rusty-venture-vcs` — VCS provider abstraction (GitHub/GitLab/Azure DevOps).
- `crates/rusty-venture-improve` — improvement DAG plumbing (scaffolded remediation path).
- `crates/rusty-venture-cli` — terminal UX for analyze/history.
- `crates/rusty-venture-server` — Axum HTTP API.
- `frontend/` — React + Vite UI (proxying API to `localhost:8080`).

---

## Prerequisites

- **Rust** (stable toolchain, edition 2021 compatible)
- **Docker** daemon running locally
- **Node.js** 18+ (for frontend)
- **Anthropic API key** (required for report generation)

---

## Configuration

The project reads environment variables (optionally from `.env`):

- `ANTHROPIC_API_KEY` (**required** for analyze flow)
- `DATABASE_URL` (optional, default: `sqlite://rusty-venture.db`)
- `LISTEN_ADDR` (optional server bind, default: `0.0.0.0:3002`)
- `RUST_LOG` (optional tracing filter, e.g. `info` / `debug`)

Example `.env`:

```env
ANTHROPIC_API_KEY=your_key_here
DATABASE_URL=sqlite://rusty-venture.db
LISTEN_ADDR=0.0.0.0:3002
RUST_LOG=info
```

> Keep `.env` private and never commit real credentials.

---

## Quickstart (CLI)

From repo root:

1. Ensure Docker is running.
2. Ensure `ANTHROPIC_API_KEY` is set (or present in `.env`).
3. Run an analysis:

```bash
cargo run -p rusty-venture-cli -- analyze https://github.com/owner/repo
```

Optional flags:

- `--branch <name>`: analyze a specific branch
- `--output json`: machine-readable output
- `--database-url <sqlite_url>`: override persistence target
- `--api-key <key>`: explicit key instead of env var

View history:

```bash
cargo run -p rusty-venture-cli -- history
```

Filter history by repository:

```bash
cargo run -p rusty-venture-cli -- history --repo https://github.com/owner/repo
```

---

## Running the API server

Start backend API:

```bash
cargo run -p rusty-venture-server
```

Server endpoints:

- `GET /health`
- `POST /analyze`
- `GET /repos`
- `GET /scans?limit=20`
- `GET /scans?repo=<repo_url>&limit=50`
- `GET /scans/:id`

### `POST /analyze` request body

```json
{
  "repo_url": "https://github.com/owner/repo",
  "branch": "main"
}
```

Returns `{ success: true, data: ... }` on success, or `{ success: false, error: ... }` on failure.

---

## Running the frontend

In a separate terminal:

```bash
cd frontend
npm install
npm run dev
```

Open `http://localhost:3000`.

The Vite dev server proxies `/analyze`, `/repos`, `/scans`, and `/health` to `http://localhost:3002`.

---

## Persistence and schema

SQLite migrations run automatically when the CLI/server opens the database.

Core tables:

- `repos` — tracked repositories
- `scans` — one row per analysis run
- `maturity_dimensions` — per-dimension score snapshots
- `violations` — file-level audit findings

Default DB file: `rusty-venture.db` in project root.

---

## Development

Recommended checks from workspace root:

```bash
cargo fmt --all
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

Frontend production build:

```bash
cd frontend
npm run build
```

---

## Notes and current boundaries

- Analysis depends on Docker container access.
- Report generation depends on Anthropic API availability/quotas.
- `rusty-venture-improve` and VCS provider crates provide improvement-path building blocks; main user-facing flows currently focus on analysis + history.

---

## Architecture artifacts

- `system-decision-tree.mmd` — Mermaid decision-tree artifact for system flow.

---

## License

No repository-level license file is currently present. Add one (for example MIT or Apache-2.0) to clarify usage rights.
