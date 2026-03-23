# rusty-venture-server

## Purpose
The HTTP API binary. Exposes repository analysis over REST + SSE (Server-Sent Events for streaming progress logs). Handles VCS webhook integration, tier gating, and scan deduplication.

## Key endpoints
- `POST /analyze` — triggers a full scan. Body: `{ repo_url, branch?, scan_tier? }`. Returns SSE stream of `LogLine` events, then a final `RepoAnalysisResult` JSON.
- `GET /repos` — list all scanned repositories with latest maturity scores.
- `GET /repos/{id}/scans` — scan history for a repository.
- `GET /graph/{repo_id}` — returns `GraphData` (nodes + edges) for the 3D maturity visualisation.
- `POST /webhook/{provider}` — receives push events from GitHub/GitLab/Azure DevOps, triggers automatic re-scans.

## SSE streaming pattern
`run_repo_analysis` accepts an optional `log_tx: LogSink` channel. The server creates an `mpsc::channel`, passes the sender into the request, and streams the receiver as SSE `data:` events. The final result is emitted as a `result` event type.

## Tier gating
Before launching a scan, the server checks `max_unlocked_tier` for the repo against the requested `scan_tier`. Tier 2 unlocks when tier 1 score ≥ threshold; tier 3 requires tier 2 score ≥ threshold AND `LlmConfig.has_project_config == true` (a project must declare its LLM before active validation is meaningful).

## Scan deduplication
Incoming webhook events include a commit SHA. The server queries the store for an existing scan at that SHA — if found, it returns the cached result rather than re-running the pipeline.
