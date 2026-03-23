# rusty-venture-store/src

## Purpose
SQLite persistence layer. Three modules: `models.rs` (plain structs), `queries.rs` (all SQL), `db.rs` (pool setup). Everything is re-exported from `crate::lib.rs` — callers import from the crate root, never from submodules directly.

## Key structs (`models.rs`)
- `RepoSummary` — one row per tracked repo, with latest grade + scan count (returned by `list_repos`)
- `ScanRow` / `ScanSummary` — full scan row vs. lightweight list item. `ScanSummary` includes `branch: Option<String>`
- `TrendPoint` — one point in the score history series; includes `branch: Option<String>` so mixed-branch trends are distinguishable
- `RepoBranchRow` — a known remote branch for a repo (`repo_branches` table)
- `RepoOverview` / `DimensionOverview` / `DimensionSignals` / `SignalItem` / `BlockerItem` — assembled view returned by `get_repo_overview`

## Key queries (`queries.rs`)
- `insert_scan` — persists a complete analysis run in a transaction (repo upsert → scan → dimensions → signals → grade → graph)
- `get_repo_overview(pool, repo_id, scan_id: Option<&str>)` — returns the full overview for the most-recent scan, or a specific scan when `scan_id` is `Some`
- `list_scans` / `list_scans_for_repo` — both return `Vec<ScanSummary>` including `branch`
- `get_repo_trends` — returns `Vec<TrendPoint>` with `branch` per point
- `upsert_branches` / `list_branches_for_repo` — branch management for the `repo_branches` table
- `ensure_repo` — upserts a minimal repo row (URL only), returns its `id`; used by the scan-branches endpoint before a full scan exists

## What NOT to put here
- Business logic or scoring — that lives in `rusty-venture-actions`
- HTTP concerns — that lives in `rusty-venture-server`
- Any code that imports `axum`, `tokio::process`, or other non-persistence crates
