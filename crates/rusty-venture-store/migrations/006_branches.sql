-- Migration 006: Branch tracking
--
-- Stores the known remote branches for each repo.
-- Populated by the "Scan for Branches" action (git ls-remote).
-- Joined to repos via repo_id.

CREATE TABLE IF NOT EXISTS repo_branches (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    repo_id     TEXT    NOT NULL REFERENCES repos(id) ON DELETE CASCADE,
    name        TEXT    NOT NULL,               -- e.g. "main", "feature/my-work"
    is_default  INTEGER NOT NULL DEFAULT 0,     -- 1 if this is the default branch
    last_seen   TEXT    NOT NULL,               -- ISO-8601 datetime of last scan
    UNIQUE(repo_id, name)
);

CREATE INDEX IF NOT EXISTS idx_branches_repo ON repo_branches(repo_id);
