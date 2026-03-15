-- Migration 002: v2 multi-dimensional grade model
--
-- Introduces six new tables that persist the full structured result of a
-- maturity scan: grade model registry, per-scan grade, per-dimension scores,
-- individual signals, signal evidence, and decision-graph artifacts.
--
-- All v2 tables cascade-delete from `scans` so that removing a scan row
-- automatically removes every piece of derived data in a single transaction.

-- ── Grade model registry ─────────────────────────────────────────────────────
-- One row per version of the scoring model.  Seeded lazily on first use.
CREATE TABLE IF NOT EXISTS grade_models (
    id          TEXT    PRIMARY KEY,
    version     TEXT    NOT NULL,
    description TEXT    NOT NULL,
    created_at  TEXT    NOT NULL
);

-- ── Per-scan v2 grade ────────────────────────────────────────────────────────
-- One row per scan.  Stores the composite score and league-tier label.
CREATE TABLE IF NOT EXISTS scan_grades (
    id          TEXT    PRIMARY KEY,
    scan_id     TEXT    NOT NULL REFERENCES scans(id) ON DELETE CASCADE,
    model_id    TEXT    NOT NULL REFERENCES grade_models(id),
    composite   INTEGER NOT NULL,   -- 0–100
    grade       TEXT    NOT NULL,   -- BRONZE / SILVER / GOLD / PLATINUM / DIAMOND
    created_at  TEXT    NOT NULL
);

-- ── Per-dimension breakdown ──────────────────────────────────────────────────
-- One row per maturity dimension per scan grade.
CREATE TABLE IF NOT EXISTS scan_dimension_scores_v2 (
    id              INTEGER PRIMARY KEY AUTOINCREMENT,
    scan_grade_id   TEXT    NOT NULL REFERENCES scan_grades(id) ON DELETE CASCADE,
    dimension       TEXT    NOT NULL,   -- e.g. "Security", "Dependency Health"
    score           INTEGER NOT NULL,   -- 0–100
    weight          REAL    NOT NULL    -- dimension weight, e.g. 0.25
);

-- ── Individual signals ───────────────────────────────────────────────────────
-- One row per signal evaluated within a dimension score.
CREATE TABLE IF NOT EXISTS scan_signals (
    id                  INTEGER PRIMARY KEY AUTOINCREMENT,
    dimension_score_id  INTEGER NOT NULL REFERENCES scan_dimension_scores_v2(id) ON DELETE CASCADE,
    name                TEXT    NOT NULL,           -- short identifier
    description         TEXT    NOT NULL,           -- human-readable label
    passed              INTEGER NOT NULL,           -- 1 = passed, 0 = failed
    points              INTEGER NOT NULL,           -- max points for this signal
    detail              TEXT                        -- optional recommendation (nullable)
);

-- ── Signal evidence ──────────────────────────────────────────────────────────
-- Zero or more evidence records per signal (file paths, counts, metrics).
CREATE TABLE IF NOT EXISTS signal_evidence (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    signal_id   INTEGER NOT NULL REFERENCES scan_signals(id) ON DELETE CASCADE,
    kind        TEXT    NOT NULL,   -- e.g. "file_found", "file_missing", "count"
    value       TEXT    NOT NULL    -- the evidence value (may be JSON)
);

-- ── Decision graph artifacts ─────────────────────────────────────────────────
-- One graph artifact per scan: a JSON blob capturing the evaluation DAG.
CREATE TABLE IF NOT EXISTS decision_graphs (
    id          TEXT PRIMARY KEY,
    scan_id     TEXT NOT NULL REFERENCES scans(id) ON DELETE CASCADE,
    graph_json  TEXT NOT NULL,
    created_at  TEXT NOT NULL
);

-- ── Indices ──────────────────────────────────────────────────────────────────
CREATE INDEX IF NOT EXISTS idx_scan_grades_scan        ON scan_grades(scan_id);
CREATE INDEX IF NOT EXISTS idx_dim_scores_v2_grade     ON scan_dimension_scores_v2(scan_grade_id);
CREATE INDEX IF NOT EXISTS idx_scan_signals_dim        ON scan_signals(dimension_score_id);
CREATE INDEX IF NOT EXISTS idx_signal_evidence_signal  ON signal_evidence(signal_id);
CREATE INDEX IF NOT EXISTS idx_decision_graphs_scan    ON decision_graphs(scan_id);
