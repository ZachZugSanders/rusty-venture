# frontend

## Stack
React 18 + TypeScript + Vite. Two main views: `ReposView` (repository list + scan launcher) and `MaturityGraphView` (interactive 3D solar system). CSS Modules for all styling. React Three Fiber (R3F) + `@react-three/drei` for the 3D canvas.

## Solar System Model (`src/views/MaturityGraphView.tsx`)
The maturity graph is visualised as a solar system:

| Graph concept | 3D object | Visual |
|---|---|---|
| Root node (repo) | Star | Glowing sun with pulsing corona + granulation texture |
| Dimension node | Gas giant planet | 6 unique colour schemes, orbiting the star |
| Signal node | Rocky moon | Orbiting its parent planet, size proportional to signal weight |

**Key layout constants:**
- `X_SCALE = 10` — horizontal axis maps to score (0→100%)
- `SOLAR_PLANET_BASE = 4.0`, `SOLAR_PLANET_GAP = 2.5` — orbital radii
- Planet size = base size × 0.9 (10% smaller than natural)
- Moon radius = `0.08 + (signal.max_points / dimension.max_points) * 0.38`

**Animation pattern:** `useFrame` mutates Three.js object positions directly via `groupRef.current.position.set(...)` — zero React state updates at 60fps. Never put orbital position in React state.

**Texture creation:** All textures (`STAR_TEX`, `GAS_GIANT_TEXTURES[]`, `PLANET_TEX`) are created once at module load using `THREE.CanvasTexture` + Canvas 2D API. Seeded PRNG (`Math.imul`-based LCG) ensures deterministic procedural textures per node ID.

## Type system (`src/types.ts`)
- `GraphNode` — id, kind ('root'|'dimension'|'signal'), px/py/pz (3D position), score, label, tier, passed, max_points
- `GraphEdge` — from, to
- `MaturityScore`, `DimensionScore`, `MaturitySignal` — mirror the Rust structs
- `RepoSummary`, `ScanResult` — API response shapes

## API base URL
`/api` proxied to the backend by Vite dev server (see `vite.config.ts`). In production, nginx routes `/api/` to the backend container.

## State management
No global store. Each view fetches its own data via `fetch('/api/...')` in `useEffect`. The `RunProgressPanel` component streams SSE from `/api/analyze` using `EventSource`.

## Adding a new view
1. Create `src/views/MyView.tsx` + `MyView.module.css`
2. Add a `Tab` entry and nav item in `App.tsx`
3. Add `{tab === 'my-view' && <MyView />}` in the main content area
