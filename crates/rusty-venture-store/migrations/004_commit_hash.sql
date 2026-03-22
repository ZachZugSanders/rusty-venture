-- Add commit_hash column to track which git commit each scan was run against.
-- NULL for scans performed before this migration.
ALTER TABLE scans ADD COLUMN commit_hash TEXT;

-- Index for the "already scanned this commit?" lookup used by the rescan endpoint.
CREATE INDEX IF NOT EXISTS idx_scans_repo_commit ON scans(repo_id, commit_hash);
