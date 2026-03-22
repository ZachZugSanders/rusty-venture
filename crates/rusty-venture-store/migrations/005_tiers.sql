-- Migration 005: Tiered maturity scanning
--
-- Adds tier-progression tracking at three levels:
--   repos      — which tier is currently unlocked for this repo
--   scans      — which tier was executed in this scan
--   scan_signals — which tier each individual signal belongs to
--
-- Default of 1 everywhere means existing data is treated as Tier 1 (Static
-- Discovery), which is correct — all existing signals are Tier 1 signals.

ALTER TABLE repos        ADD COLUMN max_unlocked_tier INTEGER NOT NULL DEFAULT 1;
ALTER TABLE scans        ADD COLUMN scan_tier         INTEGER NOT NULL DEFAULT 1;
ALTER TABLE scan_signals ADD COLUMN tier              INTEGER NOT NULL DEFAULT 1;

-- Speed up "what is the unlocked tier for repo X?" lookups
CREATE INDEX IF NOT EXISTS idx_repos_tier ON repos(max_unlocked_tier);
