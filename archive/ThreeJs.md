# rusty-venture — 3D Decision Map TODO (Completed Phases Archive)

## Context

The **3D library** in use is **Three.js** (`three` npm package) accessed through
`@react-three/fiber` (React bindings) and `@react-three/drei` (helpers like
`OrbitControls`, `Line`).

The decision graph pipeline:

1. A repository is scanned via CLI (`rusty-venture analyze <url>`) or
   `POST /analyze`.
2. The resulting `MaturityScore` is stored in SQLite (`decision_graphs.graph_json`).
3. `GET /scans/:id/decision-graph` deserialises the stored `MaturityScore` and
   calls `DecisionGraph::from_maturity()` to produce the typed graph payload
   (with pre-computed positions and radii).
4. The React `DecisionGraphView` renders it with Three.js spheres + edges.

---

## Completed ✅

- [x] **`NodeSizeConfig` struct** (`crates/rusty-venture-actions/src/graph.rs`)  
      Static configuration that drives orbit radii and sphere sizing.  
      Exported from `lib.rs` and mirrored in `frontend/src/types.ts`.

- [x] **New `GraphNode` schema**  
      Replaced raw `x/y/z` axis values with:
      - `px / py / pz` — pre-computed 3D world-space positions  
      - `radius` — pre-computed visual sphere radius  
      - `score / weight / max_points / signal_count` — semantic metadata

- [x] **Hierarchical radial position algorithm**  
      Root → origin `(0,0,0)`.  
      Dimensions → evenly spaced circle of radius `orbit_l1` in the XZ plane.  
      Signals → sub-circle of radius `orbit_l2` in the *tangential + up* plane
      around their parent dimension (no cluster overlap between adjacent dims).

- [x] **`DecisionGraph::from_maturity_with_config()`**  
      Accepts a custom `NodeSizeConfig`; `from_maturity()` uses the default.

- [x] **Frontend `DecisionGraphView` updated**  
      `NodeSphere` now reads `node.radius` and `[node.px, node.py, node.pz]`
      directly. Removed the old `SCALE_X/Y/Z` front-end scale factors and
      `toVec3()` helper. Inspector shows `score`, `weight`, `position`, `radius`.

- [x] **34 graph unit tests green**  
      Tests cover node IDs, edges, semantic metadata, highlight logic,
      structural completeness, world-space positions (orbit distances), radii
      (score-driven scaling), and config round-trip.

---

## Phase 1 — Self-scan (scan this repo) 🔜

> **Goal:** perform a live scan of the `rusty-venture` repository itself using
> the CLI, then visualise the resulting decision graph in the 3D viewer.

- [ ] **CLI self-scan**  
      Run `cargo run -p rusty-venture-cli -- analyze <remote-url>` targeting
      this repo's GitHub remote.  
      Confirm a `decision_graphs` row is inserted and
      `GET /scans/:id/decision-graph` returns valid data.

- [ ] **Verify graph in browser**  
      Boot the dev server (`npm run dev` in `frontend/`) + the API server
      (`cargo run -p rusty-venture-server`), navigate to Decision Graph, and
      confirm the self-scan appears as a loaded 3D graph.

---

## Phase 2 — NodeSizeConfig UI panel ✅

> **Goal:** expose the static `NodeSizeConfig` values as interactive sliders so
> the layout can be explored without re-scanning.

- [x] **Save config to localStorage** on change so tweaks persist across
      page reloads.

- [x] **Left-panel "Layout" section** in `DecisionGraphView`  
      - Sliders for `orbit_l1`, `orbit_l2`  
      - Sliders for `root_base_radius`, `dim_base_radius`, `sig_base_radius`  
      - Toggle: `score_scale` on/off (set to 0 for uniform sizes)

- [x] **Client-side re-layout** (`recomputeLayout()` in `DecisionGraphView.tsx`)  
      TypeScript port of Rust `from_maturity_with_config` radial algorithm.  
      Positions recomputed instantly on slider change — no re-fetch needed.  
      Server remains source-of-truth for the default; overrides are visual-only.

- [x] **"Reset to defaults" button** restores `graph.config` from the server
      and clears the localStorage entry.

---

## Phase 3 — Richer node data ✅

> **Goal:** add more dimensions to node sizing so the visual weight of each
> sphere reflects real-world impact.

- [x] **Dimension node radius** driven by `signal_count`  
      `dim_signal_scale` config field (default `0.02`); more signals → larger
      cluster hub sphere, so visually prominent dimensions stand out.

- [x] **Signal node radius** driven by `detail` richness  
      Signals with a non-null `detail` string (actionable findings) get an
      extra `sig_detail_boost` (default `0.08`) added to their radius.  
      `has_detail: bool` field added to `GraphNode`; inspector shows
      "📝 Has actionable detail".

- [x] **Root node radius** driven by `risk_score` from `FinalReport`  
      New `DecisionGraph::from_maturity_full(score, risk_score, config)` API.  
      The server now calls `get_scan()` alongside `get_decision_graph_for_scan()`
      to extract `risk_score` and passes it to the graph builder.  
      Falls back to composite score when `risk_score` is not available.

- [x] **Edge thickness** proportional to dimension `weight`  
      `GraphEdge.weight: f32` added; root→dim edges carry the full dimension
      weight; dim→sig edges carry half.  Frontend `lineWidth = max(0.5, w * 6)`.

- [x] **9 new Rust tests** (43 total)  
      `has_detail` correctness, detail-driven radius, signal-count-driven radius,
      edge weight correctness, `from_maturity_full` risk-score integration.

- [x] **2 new Layout panel sliders** (Dim-sigs, Detail boost)

---

## Phase 4 — Graph diff view ✅

> **Goal:** overlay two scans on the same canvas so regressions and improvements
> are immediately visible.

- [x] Add a second scan selector to `DecisionGraphView`.
      — "Compare to" dropdown (— None — disables diff mode) inserted below
        primary scan selector; fetches comparison graph via same endpoint.

- [x] Ghost rendering for nodes that existed in scan A but not B (grey,
      semi-transparent).
      — `ghostNodes` useMemo produces comparison-only nodes; rendered with
        `NodeSphere ghost` flag: `#64748b`, `opacity={0.35}`, `transparent`.

- [x] Colour-shift edges between matched nodes to indicate score delta
      (green = improved, red = regressed).
      — `deltaMap: Map<string, number>` = primary.score − comparison.score per
        shared node; edges average both endpoint deltas → `#22c55e` / `#ef4444`.
      — `NodeSphere` tints emissive green/red when `delta` prop is set.

- [x] Inspector shows delta: `+12 pts` / `-5` when a shared node is selected.
      — Δ Score row with `deltaPositive` / `deltaNegative` CSS classes.

---

## Phase 5 — Force-directed layout option ✅

> **Goal:** add an optional force-directed (spring-graph) layout as an
> alternative to the current deterministic radial layout.

- [x] Implement a simple Verlet-integration force simulation (repulsion +
      spring attraction along edges) in TypeScript that runs client-side.
      — `frontend/src/utils/forceSimulation.ts`: `initParticles`, `stepSimulation`,
        `extractPositions`, `runForceSimulation` (pure batch runner).
      — Coulomb repulsion between every node pair; Hooke spring attraction along
        edges; optional root pinning (default: true).
      — **10 Vitest unit tests** all green (contract, repulsion, spring, pinning,
        convergence).

- [x] Layout toggle in the left panel: **Radial** (default) vs **Force**.
      — Radial/Force radio buttons with `data-testid="layout-mode-radial"` /
        `"layout-mode-force"` inside `data-testid="layout-toggle"` wrapper.
      — `layoutMode: 'radial' | 'force'` state; sliders stay active in both modes.

- [x] Animation: nodes smoothly transition from radial positions to
      force-settled positions on toggle.
      — `useEffect` on `[layoutMode, activeGraph]`: seeds simulation from current
        radial positions, runs 8 steps/frame via `requestAnimationFrame` for up to
        200 frames (~3 s at 60 fps), then stops.
      — `displayGraph` derives from `activeGraph` (radial) or `forceNodes` state
        (force); all rendering (Scene, inspector, node list) uses `displayGraph`.

---

## Phase 6 — E2E test coverage ✅

- [x] Playwright test: scan this repo, navigate to Decision Graph, assert
      at least one `root` sphere is present in the canvas.
      → `tests/e2e/test_decision_graph.py` `TestDecisionGraphRootNode` (2 tests)

- [x] Unit test: verify `NodeSizeConfig` JSON round-trips correctly through
      `serde_json::to_string` → `serde_json::from_str`.
      → `crates/rusty-venture-actions/src/graph.rs` `node_size_config_serde_round_trip`

- [x] API integration test: seed a scan with a known `MaturityScore`, call
      `GET /scans/:id/decision-graph`, assert `config.orbit_l1` matches the
      server default.
      → `crates/rusty-venture-server/src/main.rs` `decision_graph_config_has_default_orbit_l1`
