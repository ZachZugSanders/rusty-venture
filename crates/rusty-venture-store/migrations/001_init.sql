-- Tracked repositories
CREATE TABLE IF NOT EXISTS repos (
    id          TEXT PRIMARY KEY,           -- UUID
    url         TEXT NOT NULL UNIQUE,
    first_seen  TEXT NOT NULL,              -- ISO-8601 datetime
    last_scanned TEXT                       -- ISO-8601 datetime, updated each scan
);

-- One row per analysis run
CREATE TABLE IF NOT EXISTS scans (
    id              TEXT PRIMARY KEY,       -- run_id UUID
    repo_id         TEXT NOT NULL REFERENCES repos(id),
    scanned_at      TEXT NOT NULL,          -- ISO-8601 datetime
    duration_ms     INTEGER NOT NULL,
    risk_score      INTEGER NOT NULL,       -- 0-100 from LLM report
    composite_maturity INTEGER NOT NULL,    -- 0-100 weighted maturity score
    maturity_grade  TEXT NOT NULL,          -- NASCENT | EMERGING | DEVELOPING | ESTABLISHED | EXEMPLARY
    raw_report      TEXT NOT NULL,          -- JSON blob of FinalReport
    raw_maturity    TEXT NOT NULL           -- JSON blob of MaturityScore
);

-- Per-dimension scores (queryable for trending)
CREATE TABLE IF NOT EXISTS maturity_dimensions (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    scan_id     TEXT NOT NULL REFERENCES scans(id),
    dimension   TEXT NOT NULL,              -- Security | DependencyHealth | etc.
    score       INTEGER NOT NULL            -- 0-100
);

-- Individual violations for filtering and trending
CREATE TABLE IF NOT EXISTS violations (
    id              INTEGER PRIMARY KEY AUTOINCREMENT,
    scan_id         TEXT NOT NULL REFERENCES scans(id),
    severity        TEXT NOT NULL,          -- Critical | Warning | Info
    file_path       TEXT NOT NULL,
    recommendation  TEXT NOT NULL
);

-- Indices for common query patterns
CREATE INDEX IF NOT EXISTS idx_scans_repo ON scans(repo_id);
CREATE INDEX IF NOT EXISTS idx_scans_at   ON scans(scanned_at);
CREATE INDEX IF NOT EXISTS idx_dims_scan  ON maturity_dimensions(scan_id);
CREATE INDEX IF NOT EXISTS idx_dims_dim   ON maturity_dimensions(dimension);
