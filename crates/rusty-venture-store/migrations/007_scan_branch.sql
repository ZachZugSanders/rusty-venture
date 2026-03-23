-- Track which git branch was scanned.
-- NULL for scans performed before this migration.
ALTER TABLE scans ADD COLUMN branch TEXT;
