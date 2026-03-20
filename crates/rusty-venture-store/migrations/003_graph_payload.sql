-- Migration 003: full DecisionGraph JSON payload
--
-- `decision_graphs.graph_json` was originally written with raw MaturityScore
-- JSON and then server-side reconstructed into a DecisionGraph on every
-- request.  This column adds the pre-computed DecisionGraph JSON so the
-- server can serve it directly without rebuilding from scratch.
--
-- The column is nullable so that legacy rows (inserted before this migration)
-- continue to work via the old fallback path.

ALTER TABLE decision_graphs ADD COLUMN graph_payload TEXT;
