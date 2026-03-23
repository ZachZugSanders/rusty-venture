# rusty-venture-store/migrations

## Purpose
SQLx migration files applied in order at server startup via `sqlx::migrate!("./migrations")` embedded at compile time. Each file is a one-way, non-destructive SQL change.

## Naming convention
`NNN_description.sql` — three-digit zero-padded sequence number, snake_case description. Never rename or reorder existing files; SQLx tracks applied migrations by filename hash.

## Migration history
| File | Change |
|------|--------|
| `001_init.sql` | Core tables: `repos`, `scans`, `maturity_dimensions`, `violations` |
| `002_grade_v2.sql` | `grade_models`, `scan_grades`, `scan_dimension_scores_v2`, `scan_signals` |
| `003_graph_payload.sql` | `decision_graphs` table + `graph_payload` column |
| `004_commit_hash.sql` | `scans.commit_hash` — tracks which git commit was scanned |
| `005_tiers.sql` | `scans.scan_tier`, `repos.max_unlocked_tier` — maturity tier gating |
| `006_branches.sql` | `repo_branches` table — known remote branches per repo (FK to `repos`) |
| `007_scan_branch.sql` | `scans.branch` — which branch was scanned (NULL for pre-migration rows) |

## Rules
- Never `DROP` or `ALTER … DROP COLUMN` on existing columns — old data must remain readable
- New nullable columns (`ALTER TABLE … ADD COLUMN foo TEXT`) are safe; use `NULL` as the sentinel for "not recorded before this migration"
- New tables are always `CREATE TABLE IF NOT EXISTS`
- Add indexes in the same migration as the table or column they cover
