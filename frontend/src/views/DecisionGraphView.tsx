/**
 * DecisionGraphView — Phase 5
 *
 * 3-pane layout:
 *   LEFT  — scan selector + filter panel + node list
 *   CENTER— <Canvas> Three.js scene (spheres for nodes, lines for edges)
 *   RIGHT — node inspector
 *
 * Node positions and sphere radii are pre-computed on the Rust backend using
 * a hierarchical radial distance-vector layout (NodeSizeConfig). The frontend
 * reads `node.px / node.py / node.pz` and `node.radius` directly.
 */
import { useEffect, useMemo, useRef, useState } from 'react'
import { Canvas, useFrame } from '@react-three/fiber'
import { OrbitControls, Line, Html } from '@react-three/drei'
import * as THREE from 'three'

import type { ApiResponse, DecisionGraph, GraphEdge, GraphNode, NodeSizeConfig, ScanSummary } from '../types'
import { initParticles, stepSimulation, extractPositions, type SimParticle } from '../utils/forceSimulation'
import styles from './DecisionGraphView.module.css'

// ── Constants ────────────────────────────────────────────────────────────────

const API_BASE = import.meta.env.VITE_API_BASE ?? 'http://localhost:8080'

// ── Colour palette ───────────────────────────────────────────────────────────

const KIND_COLOUR: Record<string, string> = {
    root: '#a78bfa',       // purple
    dimension: '#38bdf8',  // sky-blue
    signal: '#4ade80',     // green
}
const HIGHLIGHT_COLOUR = '#f87171' // red for failing nodes

const LOCALSTORAGE_KEY = 'rv-layout-config'

const DEFAULT_LAYOUT: NodeSizeConfig = {
    orbit_l1: 5.0,
    orbit_l2: 2.0,
    root_base_radius: 0.50,
    dim_base_radius: 0.28,
    sig_base_radius: 0.12,
    score_scale: 0.003,
    sig_points_scale: 0.008,
    dim_signal_scale: 0.02,
    sig_detail_boost: 0.08,
}

// ── 3D sphere node ───────────────────────────────────────────────────────────

interface NodeSphereProps {
    node: GraphNode
    selected: boolean
    onClick: () => void
    delta?: number
    ghost?: boolean
    showLabels?: boolean
}

function NodeSphere({ node, selected, onClick, delta, ghost, showLabels }: NodeSphereProps) {
    const meshRef = useRef<THREE.Mesh>(null!)
    const position: [number, number, number] = [node.px, node.py, node.pz]
    const baseColour = ghost ? '#64748b' : (node.highlight ? HIGHLIGHT_COLOUR : KIND_COLOUR[node.kind] ?? '#ffffff')
    let emissiveColour = '#000000'
    let emissiveIntensity = 0
    if (!ghost) {
        if (selected) {
            emissiveColour = baseColour
            emissiveIntensity = 0.4
        } else if (delta !== undefined && delta !== 0) {
            emissiveColour = delta > 0 ? '#22c55e' : '#ef4444'
            emissiveIntensity = Math.min(0.5, Math.abs(delta) / 50)
        }
    }

    useFrame(() => {
        if (selected && meshRef.current) {
            meshRef.current.rotation.y += 0.02
        }
    })

    return (
        <mesh
            ref={meshRef}
            position={position}
            onClick={(e) => { e.stopPropagation(); onClick() }}
        >
            <sphereGeometry args={[node.radius, 24, 24]} />
            <meshStandardMaterial
                color={baseColour}
                emissive={emissiveColour}
                emissiveIntensity={emissiveIntensity}
                transparent={ghost}
                opacity={ghost ? 0.35 : 1}
            />
            {showLabels && !ghost && (
                <Html
                    center
                    distanceFactor={8}
                    position={[0, node.radius + 0.15, 0]}
                    style={{ pointerEvents: 'none' }}
                >
                    <span
                        data-testid={`node-label-${node.id}`}
                        style={{
                            fontSize: node.kind === 'root' ? '13px' : node.kind === 'dimension' ? '11px' : '9px',
                            fontWeight: node.kind !== 'signal' ? 600 : 400,
                            color: '#f8fafc',
                            textShadow: '0 1px 3px #000,0 0 6px #000',
                            whiteSpace: 'nowrap',
                            userSelect: 'none',
                        }}
                    >
                        {node.label.length > 20 ? node.label.slice(0, 18) + '…' : node.label}
                    </span>
                </Html>
            )}
        </mesh>
    )
}

// ── 3D edge line ─────────────────────────────────────────────────────────────

interface EdgeLineProps {
    from: [number, number, number]
    to: [number, number, number]
    weight: number
    color?: string
}

function EdgeLine({ from, to, weight, color }: EdgeLineProps) {
    const points = useMemo<[number, number, number][]>(
        () => [from, to],
        [from, to]
    )

    return (
        <Line
            points={points}
            color={color ?? '#475569'}
            transparent
            opacity={0.5}
            lineWidth={Math.max(0.5, weight * 6)}
        />
    )
}

// ── Config slider row ───────────────────────────────────────────────────────

interface ConfigSliderProps {
    label: string
    value: number
    min: number
    max: number
    step: number
    onChange: (v: number) => void
}

function ConfigSlider({ label, value, min, max, step, onChange }: ConfigSliderProps) {
    return (
        <div className={styles.sliderRow}>
            <span className={styles.sliderLabel}>{label}</span>
            <input
                type="range"
                min={min}
                max={max}
                step={step}
                value={value}
                onChange={e => onChange(parseFloat(e.target.value))}
                className={styles.slider}
            />
            <span className={styles.sliderValue}>{value.toFixed(2)}</span>
        </div>
    )
}

// ── Client-side re-layout (TypeScript port of Rust radial algorithm) ─────────

/** Mirrors `DecisionGraph::from_maturity_with_config` in `graph.rs`. */
function recomputeLayout(
    nodes: GraphNode[],
    edges: GraphEdge[],
    config: NodeSizeConfig,
): GraphNode[] {
    const TAU = 2 * Math.PI
    const nodeMap = new Map(nodes.map(n => [n.id, { ...n }]))

    const rootNode = nodes.find(n => n.kind === 'root')
    if (!rootNode) return nodes

    const root = nodeMap.get(rootNode.id)!
    root.px = 0; root.py = 0; root.pz = 0
    root.radius = config.root_base_radius + config.score_scale * root.score

    const dimIds = edges.filter(e => e.from === rootNode.id).map(e => e.to)
    const nDims = dimIds.length

    dimIds.forEach((dimId, dimIdx) => {
        const dimAngle = TAU * dimIdx / Math.max(1, nDims)
        const dimPx = config.orbit_l1 * Math.cos(dimAngle)
        const dimPz = config.orbit_l1 * Math.sin(dimAngle)
        const tangX = -Math.sin(dimAngle)
        const tangZ = Math.cos(dimAngle)

        const dim = nodeMap.get(dimId)!
        dim.px = dimPx; dim.py = 0; dim.pz = dimPz
        dim.radius = config.dim_base_radius + config.score_scale * dim.score + dim.signal_count * config.dim_signal_scale

        const sigIds = edges.filter(e => e.from === dimId).map(e => e.to)
        const nSigs = sigIds.length

        sigIds.forEach((sigId, sigIdx) => {
            const sigAngle = TAU * sigIdx / Math.max(1, nSigs)
            // Expand orbit when dimension has many signals to prevent overlap.
            const effectiveOrbitL2 = Math.max(config.orbit_l2, nSigs * 0.4)
            const sig = nodeMap.get(sigId)!
            sig.px = dimPx + effectiveOrbitL2 * Math.cos(sigAngle) * tangX
            sig.py = effectiveOrbitL2 * Math.sin(sigAngle)
            sig.pz = dimPz + effectiveOrbitL2 * Math.cos(sigAngle) * tangZ
            sig.radius = config.sig_base_radius + config.sig_points_scale * sig.max_points + (sig.has_detail ? config.sig_detail_boost : 0)
        })
    })

    return Array.from(nodeMap.values())
}

// ── Scene ────────────────────────────────────────────────────────────────────

interface SceneProps {
    graph: DecisionGraph
    visibleKinds: Set<string>
    highlightOnly: boolean
    selected: string | null
    onSelectNode: (id: string) => void
    deltaMap: Map<string, number>
    ghostNodes: GraphNode[]
    showLabels: boolean
}

function Scene({ graph, visibleKinds, highlightOnly, selected, onSelectNode, deltaMap, ghostNodes, showLabels }: SceneProps) {
    const nodeMap = useMemo(() => {
        const m = new Map<string, GraphNode>()
        graph.nodes.forEach(n => m.set(n.id, n))
        return m
    }, [graph])

    const visibleNodes = useMemo(
        () => graph.nodes.filter(n =>
            visibleKinds.has(n.kind) && (!highlightOnly || n.highlight)
        ),
        [graph, visibleKinds, highlightOnly]
    )

    const visibleEdges = useMemo(() => {
        const visIds = new Set(visibleNodes.map(n => n.id))
        return graph.edges.filter(e => visIds.has(e.from) && visIds.has(e.to))
    }, [graph, visibleNodes])

    const visibleGhostNodes = useMemo(
        () => ghostNodes.filter(n => visibleKinds.has(n.kind)),
        [ghostNodes, visibleKinds]
    )

    return (
        <>
            <ambientLight intensity={0.6} />
            <pointLight position={[10, 10, 10]} intensity={1.2} />
            <OrbitControls makeDefault />

            {visibleEdges.map((edge, i) => {
                const fromNode = nodeMap.get(edge.from)
                const toNode = nodeMap.get(edge.to)
                if (!fromNode || !toNode) return null
                const d1 = deltaMap.get(edge.from)
                const d2 = deltaMap.get(edge.to)
                let edgeColour: string | undefined
                if (d1 !== undefined && d2 !== undefined) {
                    const avg = (d1 + d2) / 2
                    if (avg !== 0) edgeColour = avg > 0 ? '#22c55e' : '#ef4444'
                }
                return (
                    <EdgeLine
                        key={i}
                        from={[fromNode.px, fromNode.py, fromNode.pz]}
                        to={[toNode.px, toNode.py, toNode.pz]}
                        weight={edge.weight}
                        color={edgeColour}
                    />
                )
            })}

            {visibleNodes.map(node => (
                <NodeSphere
                    key={node.id}
                    node={node}
                    selected={selected === node.id}
                    onClick={() => onSelectNode(node.id)}
                    delta={deltaMap.get(node.id)}
                    showLabels={showLabels}
                />
            ))}

            {visibleGhostNodes.map(node => (
                <NodeSphere
                    key={`ghost-${node.id}`}
                    node={node}
                    selected={false}
                    onClick={() => onSelectNode(node.id)}
                    ghost
                />
            ))}
        </>
    )
}

// ── Inspector panel ──────────────────────────────────────────────────────────

interface InspectorProps {
    node: GraphNode | null
    delta?: number
}

function Inspector({ node, delta }: InspectorProps) {
    return (
        <aside className={styles.inspector} data-testid="node-inspector">
            <h3 className={styles.inspectorTitle}>Inspector</h3>
            {node === null ? (
                <p className={styles.placeholder} data-testid="inspector-placeholder">
                    Select a node to inspect
                </p>
            ) : (
                <dl className={styles.nodeDetail}>
                    <dt>ID</dt>
                    <dd data-testid="inspector-node-id">{node.id}</dd>
                    <dt>Label</dt>
                    <dd>{node.label}</dd>
                    <dt>Kind</dt>
                    <dd>{node.kind}</dd>
                    <dt>Score</dt>
                    <dd>{node.score.toFixed(1)}{node.max_points > 0 ? ` / ${node.max_points.toFixed(0)} pts` : ''}</dd>
                    <dt>Weight</dt>
                    <dd>{node.weight.toFixed(3)}</dd>
                    {node.signal_count > 0 && (<><dt>Signals</dt><dd>{node.signal_count}</dd></>)}
                    <dt>Radius</dt>
                    <dd>{node.radius.toFixed(3)}</dd>
                    <dt>Position</dt>
                    <dd>({node.px.toFixed(2)}, {node.py.toFixed(2)}, {node.pz.toFixed(2)})</dd>
                    <dt>Status</dt>
                    <dd>
                        {node.passed ? '✅ Passed' : '❌ Failed'}
                        {node.highlight && (
                            <span
                                className={styles.highlightBadge}
                                data-testid="inspector-highlight-badge"
                            >
                                ⚠ Highlight
                            </span>
                        )}
                    </dd>
                    {node.has_detail && (
                        <><dt>Detail</dt><dd data-testid="inspector-has-detail">📝 Has actionable detail</dd></>
                    )}
                    {delta !== undefined && (
                        <><dt>Δ Score</dt><dd className={delta > 0 ? styles.deltaPositive : delta < 0 ? styles.deltaNegative : ''} data-testid="inspector-delta">{delta > 0 ? '+' : ''}{delta.toFixed(1)}</dd></>
                    )}
                </dl>
            )}
        </aside>
    )
}

// ── Main view ────────────────────────────────────────────────────────────────

export default function DecisionGraphView() {
    // ── Labels state ─────────────────────────────────────────────────────────
    const [showLabels, setShowLabels] = useState(true)

    // ── Data state ───────────────────────────────────────────────────────────
    const [scans, setScans] = useState<ScanSummary[]>([])
    const [selectedScanId, setSelectedScanId] = useState<string>('')
    const [graph, setGraph] = useState<DecisionGraph | null>(null)
    const [loadingGraph, setLoadingGraph] = useState(false)
    const [error, setError] = useState<string | null>(null)
    // ── Comparison state ─────────────────────────────────────────────────────────────
    const [comparisonScanId, setComparisonScanId] = useState<string>('')
    const [comparisonGraph, setComparisonGraph] = useState<DecisionGraph | null>(null)
    const [loadingComparison, setLoadingComparison] = useState(false)
    // ── Force layout state ─────────────────────────────────────────────────────────
    const [layoutMode, setLayoutMode] = useState<'radial' | 'force'>('radial')
    const [forceNodes, setForceNodes] = useState<GraphNode[] | null>(null)
    const rafRef = useRef<number | null>(null)
    const simParticlesRef = useRef<SimParticle[]>([])
    const simFrameRef = useRef(0)
    // ── Filter state ─────────────────────────────────────────────────────────
    const [showRoot, setShowRoot] = useState(true)
    const [showDimension, setShowDimension] = useState(true)
    const [showSignal, setShowSignal] = useState(true)
    const [highlightOnly, setHighlightOnly] = useState(false)

    // ── Inspector state ──────────────────────────────────────────────────────
    const [selectedNodeId, setSelectedNodeId] = useState<string | null>(null)

    // ── Layout config ────────────────────────────────────────────────────────
    const [layoutConfig, setLayoutConfig] = useState<NodeSizeConfig>(() => {
        try {
            const saved = localStorage.getItem(LOCALSTORAGE_KEY)
            return saved ? (JSON.parse(saved) as NodeSizeConfig) : DEFAULT_LAYOUT
        } catch {
            return DEFAULT_LAYOUT
        }
    })

    // ── Load scan list on mount ──────────────────────────────────────────────
    useEffect(() => {
        fetch(`${API_BASE}/scans?limit=50`)
            .then(r => r.json() as Promise<ApiResponse<ScanSummary[]>>)
            .then(res => {
                if (res.success && res.data.length > 0) {
                    setScans(res.data)
                    setSelectedScanId(res.data[0].id)
                }
            })
            .catch(() => setError('Failed to load scans'))
    }, [])

    // ── Load graph when scan changes ─────────────────────────────────────────
    useEffect(() => {
        if (!selectedScanId) return
        setLoadingGraph(true)
        setGraph(null)
        setSelectedNodeId(null)
        setError(null)
        fetch(`${API_BASE}/scans/${selectedScanId}/decision-graph`)
            .then(r => r.json() as Promise<ApiResponse<DecisionGraph>>)
            .then(res => {
                if (res.success) {
                    setGraph(res.data)
                } else {
                    setError('No decision graph for this scan')
                }
            })
            .catch(() => setError('Failed to load decision graph'))
            .finally(() => setLoadingGraph(false))
    }, [selectedScanId])

    // ── Load comparison graph when compare scan changes ──────────────────────
    useEffect(() => {
        if (!comparisonScanId) {
            setComparisonGraph(null)
            return
        }
        setLoadingComparison(true)
        setComparisonGraph(null)
        fetch(`${API_BASE}/scans/${comparisonScanId}/decision-graph`)
            .then(r => r.json() as Promise<ApiResponse<DecisionGraph>>)
            .then(res => { if (res.success) setComparisonGraph(res.data) })
            .catch(() => { })
            .finally(() => setLoadingComparison(false))
    }, [comparisonScanId])

    // ── Derived ──────────────────────────────────────────────────────────────
    const visibleKinds = useMemo(() => {
        const s = new Set<string>()
        if (showRoot) s.add('root')
        if (showDimension) s.add('dimension')
        if (showSignal) s.add('signal')
        return s
    }, [showRoot, showDimension, showSignal])

    // Active graph: positions/radii recomputed client-side from layoutConfig
    const activeGraph = useMemo(() => {
        if (!graph) return null
        return { ...graph, nodes: recomputeLayout(graph.nodes, graph.edges, layoutConfig) }
    }, [graph, layoutConfig])

    // ── Force-directed layout animation via requestAnimationFrame ─────────────
    useEffect(() => {
        if (rafRef.current !== null) {
            cancelAnimationFrame(rafRef.current)
            rafRef.current = null
        }
        if (layoutMode !== 'force' || !activeGraph) {
            setForceNodes(null)
            return
        }
        // Seed simulation from current radial positions for smooth transition
        simParticlesRef.current = initParticles(activeGraph.nodes)
        simFrameRef.current = 0
        const edges = activeGraph.edges
        const SIM_STEPS_PER_FRAME = 8
        const SIM_MAX_FRAMES = 200  // settle within ~3 s at 60 fps

        function tick() {
            for (let s = 0; s < SIM_STEPS_PER_FRAME; s++) {
                stepSimulation(simParticlesRef.current, edges)
            }
            setForceNodes(extractPositions(activeGraph!.nodes, simParticlesRef.current))
            simFrameRef.current += 1
            if (simFrameRef.current < SIM_MAX_FRAMES) {
                rafRef.current = requestAnimationFrame(tick)
            } else {
                rafRef.current = null
            }
        }
        rafRef.current = requestAnimationFrame(tick)

        return () => {
            if (rafRef.current !== null) {
                cancelAnimationFrame(rafRef.current)
                rafRef.current = null
            }
        }
        // eslint-disable-next-line react-hooks/exhaustive-deps
    }, [layoutMode, activeGraph])

    // Active comparison graph (same layout config as primary)
    const activeComparisonGraph = useMemo(() => {
        if (!comparisonGraph) return null
        return { ...comparisonGraph, nodes: recomputeLayout(comparisonGraph.nodes, comparisonGraph.edges, layoutConfig) }
    }, [comparisonGraph, layoutConfig])

    // Delta map: node.id → (primary.score - comparison.score) for matched nodes
    const deltaMap = useMemo<Map<string, number>>(() => {
        const m = new Map<string, number>()
        if (!activeGraph || !activeComparisonGraph) return m
        const compMap = new Map(activeComparisonGraph.nodes.map(n => [n.id, n.score]))
        for (const node of activeGraph.nodes) {
            const compScore = compMap.get(node.id)
            if (compScore !== undefined) m.set(node.id, node.score - compScore)
        }
        return m
    }, [activeGraph, activeComparisonGraph])

    // Ghost nodes: nodes in comparison that don't exist in the primary graph
    const ghostNodes = useMemo<GraphNode[]>(() => {
        if (!activeComparisonGraph || !activeGraph) return []
        const primaryIds = new Set(activeGraph.nodes.map(n => n.id))
        return activeComparisonGraph.nodes.filter(n => !primaryIds.has(n.id))
    }, [activeGraph, activeComparisonGraph])

    // Display graph: radial positions, or force-settled positions when in force mode
    const displayGraph = useMemo(() => {
        if (!activeGraph) return null
        if (layoutMode === 'force' && forceNodes) return { ...activeGraph, nodes: forceNodes }
        return activeGraph
    }, [activeGraph, layoutMode, forceNodes])

    const visibleNodes = useMemo(() => {
        if (!displayGraph) return []
        return displayGraph.nodes.filter(n =>
            visibleKinds.has(n.kind) && (!highlightOnly || n.highlight)
        )
    }, [displayGraph, visibleKinds, highlightOnly])

    const selectedNode = useMemo(
        () => (selectedNodeId ? displayGraph?.nodes.find(n => n.id === selectedNodeId) ?? null : null),
        [selectedNodeId, displayGraph]
    )

    function updateConfig(key: keyof NodeSizeConfig, value: number) {
        setLayoutConfig(prev => {
            const next = { ...prev, [key]: value }
            localStorage.setItem(LOCALSTORAGE_KEY, JSON.stringify(next))
            return next
        })
    }

    function handleReset() {
        const defaults = graph?.config ?? DEFAULT_LAYOUT
        setLayoutConfig(defaults)
        localStorage.removeItem(LOCALSTORAGE_KEY)
    }

    // ── Render ───────────────────────────────────────────────────────────────
    return (
        <div className={styles.root} data-testid="decision-graph-view">
            {/* ── LEFT PANEL ───────────────────────────────────────────────── */}
            <aside className={styles.sidebar}>
                {/* Scan selector */}
                <div className={styles.selectorWrap}>
                    <label className={styles.label} htmlFor="scan-selector">Scan</label>
                    <select
                        id="scan-selector"
                        data-testid="scan-selector"
                        className={styles.select}
                        value={selectedScanId}
                        onChange={e => setSelectedScanId(e.target.value)}
                    >
                        {scans.map(s => (
                            <option key={s.id} value={s.id}>
                                {s.repo_url.replace('https://github.com/', '')} — {s.maturity_grade} ({s.scanned_at.slice(0, 10)})
                            </option>
                        ))}
                    </select>
                </div>

                {/* Compare selector */}
                <div className={styles.compareWrap}>
                    <label className={styles.label} htmlFor="compare-selector">Compare to</label>
                    <select
                        id="compare-selector"
                        data-testid="compare-selector"
                        className={styles.select}
                        value={comparisonScanId}
                        onChange={e => setComparisonScanId(e.target.value)}
                    >
                        <option value="">— None —</option>
                        {scans.filter(s => s.id !== selectedScanId).map(s => (
                            <option key={s.id} value={s.id}>
                                {s.repo_url.replace('https://github.com/', '')} — {s.maturity_grade} ({s.scanned_at.slice(0, 10)})
                            </option>
                        ))}
                    </select>
                    {loadingComparison && <span className={styles.compareLoading}>Loading…</span>}
                </div>

                {/* Filter panel */}
                <div className={styles.filterPanel} data-testid="filter-panel">
                    <p className={styles.filterTitle}>Filters</p>
                    <label>
                        <input type="checkbox" onChange={e => setShowRoot(e.target.checked)} checked={showRoot} />
                        {' '}Root
                    </label>
                    <label>
                        <input type="checkbox" onChange={e => setShowDimension(e.target.checked)} checked={showDimension} />
                        {' '}Dimension
                    </label>
                    <label>
                        <input type="checkbox" onChange={e => setShowSignal(e.target.checked)} checked={showSignal} />
                        {' '}Signal
                    </label>
                    <label>
                        <input type="checkbox" onChange={e => setHighlightOnly(e.target.checked)} checked={highlightOnly} />
                        {' '}Highlight only
                    </label>
                    <p className={styles.nodeCountLabel}>
                        Showing <span data-testid="node-count">{visibleNodes.length}</span> nodes
                    </p>
                </div>

                {/* Layout panel */}
                <div className={styles.layoutPanel} data-testid="layout-panel">
                    <p className={styles.filterTitle}>Layout</p>
                    {/* Radial / Force toggle */}
                    <div data-testid="layout-toggle" className={styles.layoutToggle}>
                        <label className={styles.layoutToggleOption}>
                            <input
                                type="radio"
                                name="layout-mode"
                                value="radial"
                                data-testid="layout-mode-radial"
                                checked={layoutMode === 'radial'}
                                onChange={() => setLayoutMode('radial')}
                            />
                            {' '}Radial
                        </label>
                        <label className={styles.layoutToggleOption}>
                            <input
                                type="radio"
                                name="layout-mode"
                                value="force"
                                data-testid="layout-mode-force"
                                checked={layoutMode === 'force'}
                                onChange={() => setLayoutMode('force')}
                            />
                            {' '}Force
                        </label>
                    </div>
                    <ConfigSlider label="Orbit L1" value={layoutConfig.orbit_l1}
                        min={1} max={15} step={0.5} onChange={v => updateConfig('orbit_l1', v)} />
                    <ConfigSlider label="Orbit L2" value={layoutConfig.orbit_l2}
                        min={0.5} max={8} step={0.25} onChange={v => updateConfig('orbit_l2', v)} />
                    <ConfigSlider label="Root r" value={layoutConfig.root_base_radius}
                        min={0.1} max={2} step={0.05} onChange={v => updateConfig('root_base_radius', v)} />
                    <ConfigSlider label="Dim r" value={layoutConfig.dim_base_radius}
                        min={0.05} max={1.5} step={0.05} onChange={v => updateConfig('dim_base_radius', v)} />
                    <ConfigSlider label="Sig r" value={layoutConfig.sig_base_radius}
                        min={0.02} max={1} step={0.02} onChange={v => updateConfig('sig_base_radius', v)} />
                    <ConfigSlider label="Dim sigs" value={layoutConfig.dim_signal_scale}
                        min={0} max={0.1} step={0.005} onChange={v => updateConfig('dim_signal_scale', v)} />
                    <ConfigSlider label="Detail" value={layoutConfig.sig_detail_boost}
                        min={0} max={0.5} step={0.01} onChange={v => updateConfig('sig_detail_boost', v)} />
                    <label className={styles.toggleLabel}>
                        <input
                            type="checkbox"
                            checked={layoutConfig.score_scale !== 0 || layoutConfig.sig_points_scale !== 0}
                            onChange={e => {
                                const on = e.target.checked
                                setLayoutConfig(prev => {
                                    const next = {
                                        ...prev,
                                        score_scale: on ? DEFAULT_LAYOUT.score_scale : 0,
                                        sig_points_scale: on ? DEFAULT_LAYOUT.sig_points_scale : 0,
                                    }
                                    localStorage.setItem(LOCALSTORAGE_KEY, JSON.stringify(next))
                                    return next
                                })
                            }}
                        />
                        {' '}Score-driven size
                    </label>
                    <button className={styles.resetBtn} onClick={handleReset}>
                        Reset to defaults
                    </button>
                    <label className={styles.toggleLabel}>
                        <input
                            type="checkbox"
                            data-testid="labels-toggle"
                            checked={showLabels}
                            onChange={e => setShowLabels(e.target.checked)}
                        />
                        {' '}Show labels
                    </label>
                </div>

                {/* Node list */}
                <ul className={styles.nodeList}>
                    {visibleNodes.map(node => (
                        <li
                            key={node.id}
                            className={`${styles.nodeListItem} ${selectedNodeId === node.id ? styles.nodeListItemActive : ''} ${node.highlight ? styles.nodeListItemHighlight : ''}`}
                            data-testid="node-list-item"
                            data-node-id={node.id}
                            onClick={() => setSelectedNodeId(node.id)}
                        >
                            <span className={styles.nodeKindDot} style={{ background: node.highlight ? HIGHLIGHT_COLOUR : KIND_COLOUR[node.kind] }} />
                            <span className={styles.nodeListLabel}>{node.label}</span>
                        </li>
                    ))}
                </ul>
            </aside>

            {/* ── CANVAS ───────────────────────────────────────────────────── */}
            <div className={styles.canvasWrap}>
                {loadingGraph && <p className={styles.loadingMsg}>Loading graph…</p>}
                {error && <p className={styles.errorMsg}>{error}</p>}
                {activeGraph && !loadingGraph && (
                    <Canvas camera={{ position: [0, 4, 14], fov: 50 }} style={{ background: '#0f172a' }}>
                        <Scene
                            graph={displayGraph!}
                            visibleKinds={visibleKinds}
                            highlightOnly={highlightOnly}
                            selected={selectedNodeId}
                            onSelectNode={id => setSelectedNodeId(id === selectedNodeId ? null : id)}
                            deltaMap={deltaMap}
                            ghostNodes={ghostNodes}
                            showLabels={showLabels}
                        />
                    </Canvas>
                )}
            </div>

            {/* ── INSPECTOR ────────────────────────────────────────────────── */}
            <Inspector node={selectedNode ?? null} delta={selectedNodeId ? deltaMap.get(selectedNodeId) : undefined} />
        </div>
    )
}
