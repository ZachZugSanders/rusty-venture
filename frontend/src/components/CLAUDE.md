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
"Analyze Repository" dialog. Collects repo URL, branch, and analysis tier (T1/T2/T3), then POSTs to `/api/analyze`.

**Props:**
- `onClose: () => void`
- `onStarted: (runId: string, repoUrl: string) => void` — called when the server accepts the run

**Tier system:**
- T1 (default): Static analysis — shows the full pipeline tree with per-action toggles.
- T2: Content quality — shows an info panel with checks list. Requires a prior T1 scan server-side.
- T3: Active validation — shows an info panel. Requires a prior T2 scan. Spawns `rv-exec` container.
- The `tier` field is sent in the POST body; the server enforces the unlock requirement.

### `RepoActionsModal` / `RepoActionsModal.module.css`
Per-repository action menu. Shows available actions for a scanned repo.

### `RunProgressPanel` / `RunProgressPanel.module.css`
Streams SSE from `/api/analyze` and displays live progress for an in-progress run.

## CSS conventions
All styles are CSS Modules (`.module.css`). No global class names. Use `--tier-color` CSS custom properties for tier-specific theming (set inline via `style={{ '--tier-color': color } as React.CSSProperties}`).
