# frontend/src/views

## Purpose
Full-page view components. Each view corresponds to one tab in `App.tsx`. Views fetch their own data and own their local state — no shared store.

## View inventory

### `ReposView` / `ReposView.module.css`
Repository list page. Fetches `GET /api/repos`, displays scan status, and opens `AnalyzeModal` to start a new run. Opens `RepoActionsModal` for per-repo actions.

### `MaturityGraphView` / `MaturityGraphView.tsx`
Interactive 3D solar system visualisation of a maturity scan result. The heaviest view — see below.

### `DecisionGraphView` / `DecisionGraphView.tsx`
Dependency decision graph view (improvement pipeline).

## MaturityGraphView internals

### Solar system model
| Graph concept | 3D object |
|---|---|
| Root (repo) | Star — glowing sun with pulsing corona |
| Dimension node | Gas giant planet — orbits the star |
| Signal node | Rocky moon — orbits its parent planet |

### Camera fly-to
Clicking a sidebar node triggers `cameraFocusRequest: FocusRequest | null` state (`{ id, seq }`). `seq` increments on every click — even re-clicking the same node fires a new animation. Two focus components handle this:
- **`SolarCameraFocus`** (inside the solar Canvas): detects a new `seq` in `useFrame`, snapshots the object's world position at that frame's clock time, then lerps `camera.position` and `controls.target` over ~28 frames using smooth-step easing.
- **`GenericCameraFocus`** (non-solar layouts): same pattern but reads static `px/py/pz` from node data.

### Planet pass/fail glow
`OrbitingPlanet` reads `planet.node.passed` and `planet.node.highlight`. If `passed && !highlight` → green atmosphere (`#22c55e`); otherwise → red (`#ef4444`). Outer atmosphere sphere opacity: 0.10 (pass) / 0.22 (fail). Surface emissive intensity: 0.05 (pass) / 0.18 (fail).

### Animation pattern
`useFrame` mutates Three.js object refs directly — **never** put 60fps orbital positions in React state. Camera animation likewise lives entirely in `useFrame` refs.

### Controls access
`useThree().controls` returns the `OrbitControls` instance when `makeDefault` is set on `<OrbitControls>`. Cast to `any` to call `.target.lerpVectors()` and `.update()`.

## Adding a new view
1. Create `src/views/MyView.tsx` + `MyView.module.css`
2. Add a `Tab` entry and nav item in `App.tsx`
3. Add `{tab === 'my-view' && <MyView />}` in the main content area
4. Add this directory's CLAUDE.md entry above
