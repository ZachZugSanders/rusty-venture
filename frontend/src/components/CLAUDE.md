# frontend/src/components

## Purpose
Shared, reusable UI components used across multiple views. Not page-level views — components here are building blocks.

## Component inventory

### `Modal` / `Modal.module.css`
Generic modal overlay. Wraps content in a centered dialog with a backdrop.

**Props:**
- `title: string` — header text
- `onClose: () => void` — called when the X button or backdrop is clicked
- `size?: 'sm' | 'md' | 'lg'` — controls max-width (default `'md'`)
- `children: ReactNode`

Use this as the shell for any new modal. Never build a custom backdrop/overlay.

### `AnalyzeModal` / `AnalyzeModal.module.css`
"Analyze Repository" dialog. Collects repo URL, branch, and analysis tier (T1/T2/T3), then POSTs to `/analyze`.

**Props:**
- `onClose: () => void`
- `onStarted: (runId: string, repoUrl: string) => void` — called when the server accepts the run

**Repo search combobox:** On focus, shows a dropdown of existing `RepoSummary` rows filtered by URL. Selecting one populates the URL field and loads known branches via `GET /repos/:id/branches`.

**Branch picker:** Plain text input by default. Becomes a `<select>` once branches are loaded. "⎇ Scan for Branches" button calls `POST /repos/scan-branches` to run `git ls-remote` server-side and populate the dropdown.

**Tier system:**
- T1 (default): Static analysis — shows the full pipeline tree with per-action toggles.
- T2: Content quality — shows an info panel with checks list. Requires a prior T1 scan server-side.
- T3: Active validation — shows an info panel. Requires a prior T2 scan. Spawns `rv-exec` container.
- The `tier` field is sent in the POST body; the server enforces the unlock requirement.

### `OverviewPanel` / `OverviewPanel.module.css`

Full maturity report for a repo. Props: `repoId: string`, `scanId?: string | null`. When `scanId` is set, fetches `GET /repos/:id/overview?scan_id=...` to show that specific scan rather than the most recent. The Trend list shows a `⎇ branch` badge on each point that has a branch recorded.

### `RepoActionsModal` / `RepoActionsModal.module.css`
Per-repository action menu. Lists all scans with Grade, Score, Branch, Risk, Duration, and Scanned At columns. Each row has a "View" button: calls `onSelectScan(scanId)` then `onClose()`, which drives `ReposView` to open that scan's report in `OverviewPanel`.

**Props:** `repo`, `onClose`, `onStarted`, `onSelectScan: (scanId: string) => void`

### `RunProgressPanel` / `RunProgressPanel.module.css`
Streams SSE from `/runs/:id/stream` and displays live log output for an in-progress run. Events: `log` (displayed), `done` (triggers `onComplete`), `failed` (triggers `onError` after 3 s).

## CSS conventions
All styles are CSS Modules (`.module.css`). No global class names. Use `--tier-color` CSS custom properties for tier-specific theming (set inline via `style={{ '--tier-color': color } as React.CSSProperties}`).
