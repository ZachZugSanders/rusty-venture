# rusty-venture-server

## Purpose
The HTTP API binary. Exposes repository analysis over REST + SSE (Server-Sent Events for streaming progress logs). Handles VCS webhook integration, tier gating, and scan deduplication.

## Key endpoints
- `POST /analyze` — triggers a full scan. Body: `{ repo_url, branch?, scan_tier? }`. Starts run and returns `{ run_id }` immediately; logs stream via SSE on `/runs/:id/stream`.
- `GET /runs/:id/stream` — SSE log stream for an in-progress run. Events: `log` (level/step/message), `done`, `failed`.
- `GET /repos` — list all scanned repositories with latest maturity scores.
- `GET /repos/:id/overview?scan_id=<id>` — full maturity overview. Without `scan_id` returns the most-recent scan; with `scan_id` returns that specific scan's data.
- `GET /repos/:id/branches` — list known remote branches for a repo (from `repo_branches` table).
- `POST /repos/scan-branches` — body `{ repo_url }`. Runs `git ls-remote --symref --heads`, upserts results into `repo_branches`, returns `{ repo_id, branches }`.
- `POST /repos/:id/rescan` — rescan using the repo's current `max_unlocked_tier`; deduplicates by commit SHA.
- `GET /repos/:id/trends` — trend series for the maturity graph, includes `branch` per point.
- `GET /graph/{repo_id}` — returns `DecisionGraph` (nodes + edges) for the 3D visualisation.
- `POST /webhook/{provider}` — receives push events from GitHub/GitLab/Azure DevOps, triggers automatic re-scans.

## SSE streaming pattern
`run_repo_analysis` accepts an optional `log_tx: LogSink` channel. The server creates a broadcast channel, registers it in `RunRegistry` (a `DashMap<run_id, Sender>`), and streams it as SSE on `/runs/:id/stream`. The entry is removed ~30 s after the run completes. nginx **must** proxy `/runs` with `proxy_buffering off` or events are held until connection close.

## Tier gating
Before launching a scan, the server checks `max_unlocked_tier` for the repo against the requested `scan_tier`. Tier 2 unlocks when tier 1 score ≥ threshold; tier 3 requires tier 2 score ≥ threshold AND `LlmConfig.has_project_config == true` (a project must declare its LLM before active validation is meaningful).

## Scan deduplication
Incoming webhook events include a commit SHA. The server queries the store for an existing scan at that SHA — if found, it returns the cached result rather than re-running the pipeline.
