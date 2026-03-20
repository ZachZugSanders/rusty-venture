/**
 * DecisionGraphView — Phase 3
 *
 * 3-pane layout:
 *   LEFT  — scan selector + filter panel + node list
 *   CENTER— <Canvas> Three.js scene (spheres for nodes, lines for edges)
 *   RIGHT — node inspector
 */
import { useEffect, useMemo, useRef, useState } from 'react'
import { Canvas, useFrame } from '@react-three/fiber'
import { OrbitControls, Line } from '@react-three/drei'
import * as THREE from 'three'

import type { ApiResponse, DecisionGraph, GraphNode, ScanSummary } from '../types'
import styles from './DecisionGraphView.module.css'

// ── Constants ────────────────────────────────────────────────────────────────

const API_BASE = import.meta.env.VITE_API_BASE ?? 'http://localhost:8080'

// Scale factors to spread the graph across the scene
const SCALE_X = 8  // weight axis  (0–1) → –4…4
const SCALE_Y = 0.08 // score axis (0–100) → 0…8
const SCALE_Z = 0.15 // points axis (0–100) → 0…15

function toVec3(n: GraphNode): [number, number, number] {
    return [
        (n.x - 0.5) * SCALE_X,
        n.y * SCALE_Y,
        n.z * SCALE_Z,
    ]
}

// ── Colour palette ───────────────────────────────────────────────────────────

const KIND_COLOUR: Record<string, string> = {
    root: '#a78bfa',       // purple
    dimension: '#38bdf8',  // sky-blue
    signal: '#4ade80',     // green
}
const HIGHLIGHT_COLOUR = '#f87171' // red for failing nodes

// ── 3D sphere node ───────────────────────────────────────────────────────────

interface NodeSphereProps {
    node: GraphNode
    selected: boolean
    onClick: () => void
}

function NodeSphere({ node, selected, onClick }: NodeSphereProps) {
    const meshRef = useRef<THREE.Mesh>(null!)
    const pos = toVec3(node)

    const radius = node.kind === 'root' ? 0.35 : node.kind === 'dimension' ? 0.22 : 0.14
    const colour = node.highlight ? HIGHLIGHT_COLOUR : KIND_COLOUR[node.kind] ?? '#ffffff'

    useFrame(() => {
        if (selected && meshRef.current) {
            meshRef.current.rotation.y += 0.02
        }
    })

    return (
        <mesh
            ref={meshRef}
            position={pos}
            onClick={(e) => { e.stopPropagation(); onClick() }}
        >
            <sphereGeometry args={[radius, 24, 24]} />
            <meshStandardMaterial
                color={colour}
                emissive={selected ? colour : '#000000'}
                emissiveIntensity={selected ? 0.4 : 0}
            />
        </mesh>
    )
}

// ── 3D edge line ─────────────────────────────────────────────────────────────

interface EdgeLineProps {
    from: [number, number, number]
    to: [number, number, number]
}

function EdgeLine({ from, to }: EdgeLineProps) {
    const points = useMemo<[number, number, number][]>(
        () => [from, to],
        [from, to]
    )

    return (
        <Line
            points={points}
            color="#475569"
            transparent
            opacity={0.5}
            lineWidth={1}
        />
    )
}

// ── Scene ────────────────────────────────────────────────────────────────────

interface SceneProps {
    graph: DecisionGraph
    visibleKinds: Set<string>
    highlightOnly: boolean
    selected: string | null
    onSelectNode: (id: string) => void
}

function Scene({ graph, visibleKinds, highlightOnly, selected, onSelectNode }: SceneProps) {
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

    return (
        <>
            <ambientLight intensity={0.6} />
            <pointLight position={[10, 10, 10]} intensity={1.2} />
            <OrbitControls makeDefault />

            {visibleEdges.map((edge, i) => {
                const fromNode = nodeMap.get(edge.from)
                const toNode = nodeMap.get(edge.to)
                if (!fromNode || !toNode) return null
                return (
                    <EdgeLine
                        key={i}
                        from={toVec3(fromNode)}
                        to={toVec3(toNode)}
                    />
                )
            })}

            {visibleNodes.map(node => (
                <NodeSphere
                    key={node.id}
                    node={node}
                    selected={selected === node.id}
                    onClick={() => onSelectNode(node.id)}
                />
            ))}
        </>
    )
}

// ── Inspector panel ──────────────────────────────────────────────────────────

interface InspectorProps {
    node: GraphNode | null
}

function Inspector({ node }: InspectorProps) {
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
                    <dt>Score (Y)</dt>
                    <dd>{node.y.toFixed(1)}</dd>
                    <dt>Weight (X)</dt>
                    <dd>{node.x.toFixed(3)}</dd>
                    <dt>Max pts (Z)</dt>
                    <dd>{node.z.toFixed(1)}</dd>
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
                </dl>
            )}
        </aside>
    )
}

// ── Main view ────────────────────────────────────────────────────────────────

export default function DecisionGraphView() {
    // ── Data state ───────────────────────────────────────────────────────────
    const [scans, setScans] = useState<ScanSummary[]>([])
    const [selectedScanId, setSelectedScanId] = useState<string>('')
    const [graph, setGraph] = useState<DecisionGraph | null>(null)
    const [loadingGraph, setLoadingGraph] = useState(false)
    const [error, setError] = useState<string | null>(null)

    // ── Filter state ─────────────────────────────────────────────────────────
    const [showRoot, setShowRoot] = useState(true)
    const [showDimension, setShowDimension] = useState(true)
    const [showSignal, setShowSignal] = useState(true)
    const [highlightOnly, setHighlightOnly] = useState(false)

    // ── Inspector state ──────────────────────────────────────────────────────
    const [selectedNodeId, setSelectedNodeId] = useState<string | null>(null)

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

    // ── Derived ──────────────────────────────────────────────────────────────
    const visibleKinds = useMemo(() => {
        const s = new Set<string>()
        if (showRoot) s.add('root')
        if (showDimension) s.add('dimension')
        if (showSignal) s.add('signal')
        return s
    }, [showRoot, showDimension, showSignal])

    const visibleNodes = useMemo(() => {
        if (!graph) return []
        return graph.nodes.filter(n =>
            visibleKinds.has(n.kind) && (!highlightOnly || n.highlight)
        )
    }, [graph, visibleKinds, highlightOnly])

    const selectedNode = useMemo(
        () => (selectedNodeId ? graph?.nodes.find(n => n.id === selectedNodeId) ?? null : null),
        [selectedNodeId, graph]
    )

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
                {graph && !loadingGraph && (
                    <Canvas camera={{ position: [0, 4, 14], fov: 50 }} style={{ background: '#0f172a' }}>
                        <Scene
                            graph={graph}
                            visibleKinds={visibleKinds}
                            highlightOnly={highlightOnly}
                            selected={selectedNodeId}
                            onSelectNode={id => setSelectedNodeId(id === selectedNodeId ? null : id)}
                        />
                    </Canvas>
                )}
            </div>

            {/* ── INSPECTOR ────────────────────────────────────────────────── */}
            <Inspector node={selectedNode ?? null} />
        </div>
    )
}
