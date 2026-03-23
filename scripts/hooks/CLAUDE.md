# scripts/hooks

## Purpose
Git hooks for the rusty-venture repository. Install once per clone:

```bash
git config core.hooksPath scripts/hooks
```

## Hooks

### `pre-commit`
Runs on every `git commit`. Scans staged files, finds directories that are missing a `CLAUDE.md`, and writes `required_claude_changes.md` at the repo root in a format optimised for dropping directly into a Claude Code session.

- **Non-blocking by default** — writes the file and warns, but does not abort the commit.
- Set `RV_CLAUDE_BLOCK=1` to make it blocking (useful in CI or pair-programming sessions where stale docs should be a hard error).

## Design decisions
- The hook walks every ancestor directory of each staged file, not just the immediate parent, so it catches cases where a whole subtree is newly added.
- It only flags directories that have at least one staged file **directly** inside them (not just transitively), avoiding false positives from intermediate path segments.
- `required_claude_changes.md` is gitignored — it is a working file, not committed output.
