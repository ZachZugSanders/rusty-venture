# rusty-venture — TODO

> Completed phases (1–6) archived in [archive/ThreeJs.md](archive/ThreeJs.md).

---

## Known risks & mitigations (source of phases below)

| Item | Risk | Mitigation |
|------|------|-----------|
| Self-scan requires working Docker | Container spawn may fail in CI | Add `--no-container` flag to CLI for static-only analysis |
| `decision_graphs.graph_json` stores `MaturityScore`, not `DecisionGraph` | Config changes don't affect stored data | Store the full `DecisionGraph` JSON (with config) in a future migration |
| Three.js label rendering | `@react-three/drei` `<Text>` requires a WOFF2 font asset | Bundle a font or use sprite-based labels |
| Large repos (100+ signals) | Sphere overlap is possible at default `orbit_l2=2.0` | Increase `orbit_l2` dynamically: `orbit_l2 = max(2.0, n_signals * 0.4)` |

---

## Phase 7 — CLI `--no-container` flag & self-scan ✅

> **Goal:** make the CLI work without Docker so a self-scan can run in any
> environment (local dev, CI, sandboxed agent) — and then actually scan this
> repo.
>
> Risk addressed: *Container spawn may fail in CI / sandboxed environments.*

- [x] **Audit CLI → actions coupling**  
      Read `rusty-venture-cli/src/main.rs` and `rusty-venture-actions` to map
      exactly where Docker / container execution is invoked.  Identify every
      code path that would fail without a Docker socket.

- [x] **Add `skip_container: bool` to `RepoAnalysisRequest`**  
      In `crates/rusty-venture-actions/src/repo/mod.rs` (or wherever
      `RepoAnalysisRequest` lives), add the field and thread it through to the
      containerise step.

- [x] **Short-circuit the container step when `skip_container = true`**  
      `run_repo_analysis_no_container()` clones with `git clone --depth 1`,
      uses local filesystem helpers, calls `GenerateReportAction` (LLM only),
      and computes maturity with `Default` container/audit/dep reports.

- [x] **Expose `--no-container` (`-n`) flag in the CLI**  
      Add the flag to the `analyze` subcommand argument parser in
      `rusty-venture-cli/src/main.rs`, map it to `skip_container: true` in
      `RepoAnalysisRequest`.

- [ ] **Unit test: `skip_container=true` produces a valid `FinalReport`**  
      In `rusty-venture-actions`, add a test that calls `run_repo_analysis`
      with a stub repo URL and `skip_container: true` and asserts the report
      is `Ok(...)` and `container_findings` is empty (or the field is absent).

- [ ] **Self-scan the repo**  
      With a running server and valid `ANTHROPIC_API_KEY`:
      ```
  cargo run -p rusty-venture-cli -- analyze <github-remote-url> --no-container
      ```
      Confirm a row is inserted into `decision_graphs` and
      `GET /scans/:id/decision-graph` returns a valid graph payload.

- [ ] **Verify in browser**  
      Boot `cargo run -p rusty-venture-server` + `npm run dev` in `frontend/`,
      navigate to the Decision Graph tab, select the self-scan, and confirm
      the 3D graph renders without errors.

---

## Phase 8 — Persist full `DecisionGraph` JSON ✅

> **Goal:** store the fully-computed `DecisionGraph` payload (nodes + edges +
> config) in the database rather than the raw `MaturityScore` blob, so that
> changing `NodeSizeConfig` defaults never silently alters historical results.
>
> Risk addressed: *`decision_graphs.graph_json` stores `MaturityScore`, not
> `DecisionGraph` — config changes don't affect stored data.*

- [x] **New migration `003_graph_payload.sql`**  
      Add a nullable `graph_payload TEXT` column to the `decision_graphs`
      table.  Nullable so existing rows remain valid without a backfill
      being a hard dependency.
      ```sql
      ALTER TABLE decision_graphs ADD COLUMN graph_payload TEXT;
      ```

- [x] **Update `insert_decision_graph()` in `rusty-venture-store`**  
      Accept an optional `graph_payload: Option<&str>` parameter (serialised
      `DecisionGraph` JSON) and write it into the new column.

- [x] **Update `scan_decision_graph_handler` in `rusty-venture-server`**  
      After computing `graph = DecisionGraph::from_maturity_full(...)`, call
      `serde_json::to_string(&graph)` and pass it to `insert_decision_graph`.
      This means new scans always persist the canonical payload.

- [x] **Update `get_decision_graph_for_scan()` in `rusty-venture-store`**  
      Return `graph_payload` when it is non-null; fall back to `graph_json`
      (legacy `MaturityScore` path) when `graph_payload IS NULL`.  The handler
      can then deserialise as `DecisionGraph` directly from `graph_payload`.

- [ ] **Backfill existing rows**  
      Write a one-shot Rust binary or migration step that iterates all
      `decision_graphs` rows where `graph_payload IS NULL`, deserialises
      `graph_json` as `MaturityScore`, calls `DecisionGraph::from_maturity`,
      and updates `graph_payload`.

- [ ] **Store tests: round-trip `graph_payload`**  
      In `rusty-venture-store/src/tests.rs`, add a test that inserts a row
      with a known `graph_payload`, queries it back, and asserts the JSON is
      identical.

- [ ] **Integration test: payload config is used verbatim**  
      Seed a scan with a `DecisionGraph` that uses `orbit_l1 = 9.0` (non-
      default), store it in `graph_payload`, call
      `GET /scans/:id/decision-graph`, and assert `data.config.orbit_l1 == 9.0`
      — proving the stored payload is returned, not recomputed from the default
      config.

---

## Phase 9 — 3D node labels ✅

> **Goal:** render text labels beside each sphere in the 3D canvas so users
> can read node names without clicking into the inspector.
>
> Risk addressed: *`@react-three/drei` `<Text>` requires a WOFF2 font asset —
> absent font causes invisible or broken labels.*

- [x] **Evaluate rendering approach**  
      Benchmark three options and pick the simplest that looks acceptable:
      1. `@react-three/drei <Html>` — DOM overlay, no font file needed, but
         does not depth-sort with 3D geometry.
      2. `@react-three/drei <Text>` — GPU SDF text, requires WOFF2 font.
      3. Canvas `<Sprite>` — render text into an offscreen `<canvas>`, use as
         a `THREE.Texture`; works with any system font but is more complex.
      Recommended starting point: `<Html>` (zero font dependency, ships fast).

- [x] **Bundle a WOFF2 font (if `<Text>` is chosen)**  
      N/A — `<Html>` selected; no font asset needed.

- [x] **Implement labels in `NodeSphere`**  
      - Root node: show grade badge text (e.g. `"GOLD · 72"`).
      - Dimension nodes: show `node.label`.
      - Signal nodes: show `node.label`, truncated to ~20 chars with CSS
        `text-overflow: ellipsis` (for `<Html>`) or glyphs limit (for `<Text>`).
      - Add `data-testid="node-label-{node.id}"` for Playwright

- [x] **"Show labels" toggle in the Layout panel**  
      Added checkbox (`data-testid="labels-toggle"`) toggling `showLabels:
      boolean` state; passed as prop to scene so labels mount/unmount without
      re-fetching data.

- [ ] **Playwright test: labels visible by default**  
      After navigating to the Graph tab and loading the mock scan, assert that
      `[data-testid="node-label-root"]` is visible.

- [ ] **Playwright test: toggle hides and restores labels**  
      Uncheck "Show labels", assert `[data-testid="node-label-root"]` is
      hidden; re-check, assert it is visible again.

---

## Phase 10 — Dynamic orbit scaling for large repos ✅

> **Goal:** prevent sphere overlap when a dimension has many signals by
> automatically widening `orbit_l2` in proportion to signal count, keeping
> the graph legible for any real-world repo.
>
> Risk addressed: *Sphere overlap is possible at default `orbit_l2=2.0` with
> 100+ signals.*

- [x] **Implement per-dimension `effective_orbit_l2` in `graph.rs`**  
      In `DecisionGraph::from_maturity_full`, before positioning signals for
      a dimension, compute:
      ```rust
      let effective_orbit_l2 = f32::max(config.orbit_l2, n_signals as f32 * 0.4);
      ```
      Use `effective_orbit_l2` only for positioning; the stored
      `config.orbit_l2` remains unchanged (it's the user's floor, not the
      computed value).

- [x] **Mirror the same logic in `recomputeLayout()` (TypeScript)**  
      In `frontend/src/views/DecisionGraphView.tsx`, update the client-side
      radial layout function to apply the same `max(orbit_l2, n * 0.4)`
      formula per dimension so slider-driven re-layouts stay consistent with
      the server output.

- [x] **Rust test: large dimension uses expanded orbit**  
      Add a test that builds a `MaturityScore` with a single dimension
      containing 8 signals (8 × 0.4 = 3.2 > default 2.0) and asserts each
      signal is exactly 3.2 units from its parent dimension centre.

- [x] **Rust test: small dimension uses configured `orbit_l2` unchanged**  
      A dimension with ≤5 signals at the default config must place signals at
      exactly `orbit_l2 = 2.0`, confirming the floor is not applied
      unnecessarily.

- [x] **Rust test: `orbit_l2` slider override is respected as the new floor**  
      If the user sets `config.orbit_l2 = 4.0` and the dimension has 3
      signals (3 × 0.4 = 1.2 < 4.0), signals must still be placed at 4.0.

- [ ] **Visual QA with synthetic large score**  
      Build a synthetic `MaturityScore` with 4 dimensions × 10 signals each
      (40 signals total), load it via mock API in the browser, and confirm no
      spheres overlap in either Radial or Force layout mode.

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
