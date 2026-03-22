/**
 * MaturityGraphView — Phase 5
 *
 * Layout modes:
 *   Solar  — solar system: star (root), orbiting gas giants (dims), moons (signals) [default]
 *   Grid   — score axis (X = 0→100), dimensions in Y/Z plane
 *   Gravity— physics sim with score-based Y forces
 *   Radial — hierarchical radial layout
 *   Force  — physics spring simulation
 */
import { useEffect, useMemo, useRef, useState } from 'react'
import { Canvas, useFrame } from '@react-three/fiber'
import { OrbitControls, Line, Html, Stars } from '@react-three/drei'
import * as THREE from 'three'

import type { ApiResponse, DecisionGraph, GraphEdge, GraphNode, NodeSizeConfig, ScanSummary } from '../types'
import { initParticles, stepSimulation, extractPositions, type SimParticle } from '../utils/forceSimulation'
import styles from './MaturityGraphView.module.css'

// ── Layout constants ──────────────────────────────────────────────────────────

const X_SCALE = 10           // score 100 → x=10 world units (horizontal axis)
const GRID_DIM_ORBIT = 3.2   // base Y-Z orbit radius for dimensions
const GRID_SIG_ORBIT = 1.2   // Y-Z orbit radius for signals
const GRID_RING_RADIUS = 9.0 // radius of Y-Z guide rings

// Solar system
const SOLAR_PLANET_BASE = 4.0  // first planet orbit radius
const SOLAR_PLANET_GAP  = 2.5  // additional radius per planet
const SOLAR_MOON_BASE   = 1.1  // first moon orbit radius
const SOLAR_MOON_GAP    = 0.6  // additional moon orbit radius per moon

// ── Colour palette ────────────────────────────────────────────────────────────

const KIND_COLOUR: Record<string, string> = {
    root:      '#fde68a',
    dimension: '#38bdf8',
    signal:    '#4ade80',
}
const HIGHLIGHT_COLOUR = '#f87171'
const LOCALSTORAGE_KEY = 'rv-layout-config'

// ── Texture generation ────────────────────────────────────────────────────────

function seededPRNG(seed: number): () => number {
    let s = seed >>> 0
    return () => {
        s = Math.imul(s ^ (s >>> 16), 0x45d9f3b)
        s = Math.imul(s ^ (s >>> 16), 0x45d9f3b)
        s = (s ^ (s >>> 16)) >>> 0
        return s / 0x100000000
    }
}

function makePlanetTexture(baseColors: string[], style: 'gas' | 'rocky', seed: number): THREE.CanvasTexture {
    const S = 512
    const canvas = document.createElement('canvas')
    canvas.width = S; canvas.height = S
    const ctx = canvas.getContext('2d')!
    const rand = seededPRNG(seed)

    ctx.fillStyle = baseColors[0]
    ctx.fillRect(0, 0, S, S)

    if (style === 'gas') {
        for (let b = 0; b < 18; b++) {
            const y = rand() * S
            const h = S * (0.03 + rand() * 0.10)
            const col = baseColors[Math.floor(rand() * baseColors.length)]
            const g = ctx.createLinearGradient(0, y, 0, y + h)
            g.addColorStop(0, 'transparent')
            g.addColorStop(0.3, col)
            g.addColorStop(0.7, col)
            g.addColorStop(1, 'transparent')
            ctx.globalAlpha = 0.35 + rand() * 0.45
            ctx.fillStyle = g
            ctx.fillRect(0, y, S, h)
        }
        const sx = S * (0.25 + rand() * 0.5), sy = S * (0.25 + rand() * 0.5)
        const sr = ctx.createRadialGradient(sx, sy, 0, sx, sy, S * 0.07)
        sr.addColorStop(0, baseColors[baseColors.length - 1] + 'dd')
        sr.addColorStop(1, 'transparent')
        ctx.globalAlpha = 0.8
        ctx.fillStyle = sr
        ctx.fillRect(0, 0, S, S)
    } else {
        for (let p = 0; p < 30; p++) {
            const px = rand() * S, py = rand() * S
            const r = S * (0.02 + rand() * 0.09)
            const col = baseColors[Math.floor(rand() * baseColors.length)]
            const g = ctx.createRadialGradient(px, py, 0, px, py, r)
            g.addColorStop(0, col)
            g.addColorStop(1, 'transparent')
            ctx.globalAlpha = 0.3 + rand() * 0.5
            ctx.fillStyle = g
            ctx.fillRect(Math.max(0, px - r), Math.max(0, py - r), r * 2, r * 2)
        }
        for (let c = 0; c < 8; c++) {
            const cx = rand() * S, cy = rand() * S, cr = S * (0.01 + rand() * 0.03)
            ctx.globalAlpha = 0.4 + rand() * 0.3
            ctx.fillStyle = 'rgba(0,0,0,0.6)'
            ctx.beginPath(); ctx.arc(cx, cy, cr, 0, Math.PI * 2); ctx.fill()
        }
    }
    ctx.globalAlpha = 1
    const shading = ctx.createRadialGradient(S * 0.32, S * 0.32, S * 0.01, S * 0.5, S * 0.5, S * 0.5)
    shading.addColorStop(0, 'rgba(255,255,255,0.22)')
    shading.addColorStop(0.45, 'rgba(255,255,255,0.04)')
    shading.addColorStop(0.75, 'rgba(0,0,0,0)')
    shading.addColorStop(1, 'rgba(0,0,0,0.50)')
    ctx.fillStyle = shading
    ctx.fillRect(0, 0, S, S)
    const tex = new THREE.CanvasTexture(canvas)
    tex.needsUpdate = true
    return tex
}

function makeStarTexture(): THREE.CanvasTexture {
    const S = 512
    const canvas = document.createElement('canvas')
    canvas.width = S; canvas.height = S
    const ctx = canvas.getContext('2d')!
    const rand = seededPRNG(99)

    const bg = ctx.createRadialGradient(S / 2, S / 2, 0, S / 2, S / 2, S / 2)
    bg.addColorStop(0, '#fff5b4')
    bg.addColorStop(0.35, '#ffcc00')
    bg.addColorStop(0.65, '#ff8800')
    bg.addColorStop(1, '#cc3300')
    ctx.fillStyle = bg
    ctx.fillRect(0, 0, S, S)

    for (let g = 0; g < 70; g++) {
        const gx = rand() * S, gy = rand() * S
        const gr = S * (0.015 + rand() * 0.05)
        const cell = ctx.createRadialGradient(gx, gy, 0, gx, gy, gr)
        cell.addColorStop(0, 'rgba(255,255,200,0.50)')
        cell.addColorStop(1, 'transparent')
        ctx.globalAlpha = 0.4 + rand() * 0.5
        ctx.fillStyle = cell
        ctx.fillRect(Math.max(0, gx - gr), Math.max(0, gy - gr), gr * 2, gr * 2)
    }
    for (let s = 0; s < 6; s++) {
        const sx = S * (0.15 + rand() * 0.70), sy = S * (0.15 + rand() * 0.70)
        const sr = S * (0.015 + rand() * 0.035)
        ctx.globalAlpha = 0.85
        ctx.fillStyle = 'rgba(30,5,0,0.9)'
        ctx.beginPath(); ctx.arc(sx, sy, sr, 0, Math.PI * 2); ctx.fill()
        const pen = ctx.createRadialGradient(sx, sy, sr, sx, sy, sr * 2.2)
        pen.addColorStop(0, 'rgba(80,20,0,0.5)')
        pen.addColorStop(1, 'transparent')
        ctx.globalAlpha = 0.6
        ctx.fillStyle = pen
        ctx.beginPath(); ctx.arc(sx, sy, sr * 2.5, 0, Math.PI * 2); ctx.fill()
    }
    ctx.globalAlpha = 1
    const tex = new THREE.CanvasTexture(canvas)
    tex.needsUpdate = true
    return tex
}

// Created once at module load, reused across renders
const STAR_TEX = makeStarTexture()
const GAS_GIANT_TEXTURES = [
    makePlanetTexture(['#1e3a5f','#1d4ed8','#3b82f6','#60a5fa','#93c5fd','#172554'], 'gas', 10),
    makePlanetTexture(['#78350f','#92400e','#d97706','#fbbf24','#c2856a','#451a03'], 'gas', 11),
    makePlanetTexture(['#4c1d95','#5b21b6','#7c3aed','#a78bfa','#c4b5fd','#2e1065'], 'gas', 12),
    makePlanetTexture(['#064e3b','#065f46','#059669','#34d399','#6ee7b7','#022c22'], 'gas', 13),
    makePlanetTexture(['#7f1d1d','#991b1b','#dc2626','#ef4444','#fca5a5','#450a0a'], 'gas', 14),
    makePlanetTexture(['#134e4a','#0f766e','#14b8a6','#5eead4','#99f6e4','#042f2e'], 'gas', 15),
]
const PLANET_TEX = {
    signal_pass: makePlanetTexture(['#14532d','#166534','#4ade80','#86efac','#713f12','#78350f'], 'rocky', 3),
    signal_fail: makePlanetTexture(['#7f1d1d','#991b1b','#ef4444','#dc2626','#450a0a','#b91c1c'], 'rocky', 4),
    ghost:       makePlanetTexture(['#1e293b','#334155','#475569','#64748b','#0f172a','#1e293b'], 'rocky', 5),
}

function hashId(id: string): number {
    let h = 0
    for (let i = 0; i < id.length; i++) h = (Math.imul(31, h) + id.charCodeAt(i)) | 0
    return (h >>> 0) % GAS_GIANT_TEXTURES.length
}

// ── Layout config ─────────────────────────────────────────────────────────────

const DEFAULT_LAYOUT: NodeSizeConfig = {
    orbit_l1: 5.0,
    orbit_l2: 2.0,
    root_base_radius: 0.50,
    dim_base_radius: 0.28,
    sig_base_radius: 0.12,
    score_scale: 0.009,
    sig_points_scale: 0.024,
    dim_signal_scale: 0.02,
    sig_detail_boost: 0.08,
}

// ── Grid layout (horizontal X score axis) ─────────────────────────────────────

function gridLayout(nodes: GraphNode[], edges: GraphEdge[], config: NodeSizeConfig): GraphNode[] {
    const TAU = 2 * Math.PI
    const nodeMap = new Map(nodes.map(n => [n.id, { ...n }]))
    const rootNode = nodes.find(n => n.kind === 'root')
    if (!rootNode) return nodes

    const root = nodeMap.get(rootNode.id)!
    root.px = (root.score / 100) * X_SCALE
    root.py = 0
    root.pz = 0
    root.radius = config.root_base_radius + config.score_scale * root.score

    const dimIds = edges.filter(e => e.from === rootNode.id).map(e => e.to)
    const nDims = dimIds.length

    dimIds.forEach((dimId, dimIdx) => {
        const dim = nodeMap.get(dimId)!
        const angle = TAU * dimIdx / Math.max(1, nDims)
        const scoreFraction = dim.max_points > 0 ? dim.score / dim.max_points : 0.5
        const orbitDist = GRID_DIM_ORBIT * (2.0 - scoreFraction)
        dim.px = root.px
        dim.py = orbitDist * Math.cos(angle)
        dim.pz = orbitDist * Math.sin(angle)
        dim.radius = config.dim_base_radius + config.score_scale * dim.score + dim.signal_count * config.dim_signal_scale

        const sigIds = edges.filter(e => e.from === dimId).map(e => e.to)
        const nSigs = sigIds.length
        sigIds.forEach((sigId, sigIdx) => {
            const sig = nodeMap.get(sigId)!
            const sigAngle = TAU * sigIdx / Math.max(1, nSigs)
            const effectiveOrbit = Math.max(GRID_SIG_ORBIT, nSigs * 0.28)
            sig.px = dim.px
            sig.py = dim.py + effectiveOrbit * Math.cos(sigAngle)
            sig.pz = dim.pz + effectiveOrbit * Math.sin(sigAngle)
            sig.radius = config.sig_base_radius
                + config.sig_points_scale * sig.max_points
                + (sig.has_detail ? config.sig_detail_boost : 0)
        })
    })

    return Array.from(nodeMap.values())
}

// ── Radial layout ─────────────────────────────────────────────────────────────

function recomputeLayout(nodes: GraphNode[], edges: GraphEdge[], config: NodeSizeConfig): GraphNode[] {
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

// ── NodeSphere (used in Grid / Radial / Force / Gravity modes) ────────────────

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

    const texture = ghost
        ? PLANET_TEX.ghost
        : node.kind === 'root'      ? STAR_TEX
        : node.kind === 'dimension' ? GAS_GIANT_TEXTURES[hashId(node.id)]
        : (node.highlight || !node.passed) ? PLANET_TEX.signal_fail
        : PLANET_TEX.signal_pass

    const baseColour = ghost ? '#64748b' : (node.highlight ? HIGHLIGHT_COLOUR : KIND_COLOUR[node.kind] ?? '#ffffff')
    let emissiveColour = node.kind === 'root' ? '#ff6600' : '#000000'
    let emissiveIntensity = node.kind === 'root' ? 1.2 : 0
    if (!ghost) {
        if (selected) {
            emissiveColour = node.kind === 'root' ? '#ff8800' : baseColour
            emissiveIntensity = node.kind === 'root' ? 1.7 : 0.5
        } else if (delta !== undefined && delta !== 0) {
            emissiveColour = delta > 0 ? '#22c55e' : '#ef4444'
            emissiveIntensity = Math.min(0.45, Math.abs(delta) / 50)
        }
    }

    useFrame(() => {
        if (meshRef.current) meshRef.current.rotation.y += selected ? 0.025 : 0.004
    })

    const atmosColour = ghost ? '#475569'
        : node.kind === 'root'      ? '#ffaa00'
        : node.kind === 'dimension' ? '#0ea5e9'
        : node.highlight            ? '#ef4444'
        : '#22c55e'

    return (
        <group position={position}>
            {!ghost && (
                <mesh>
                    <sphereGeometry args={[node.radius * (node.kind === 'root' ? 2.0 : 1.18), 32, 32]} />
                    <meshStandardMaterial
                        color={atmosColour}
                        transparent
                        opacity={node.kind === 'root' ? 0.08 : 0.13}
                        side={THREE.BackSide}
                        depthWrite={false}
                    />
                </mesh>
            )}
            <mesh ref={meshRef} onClick={(e) => { e.stopPropagation(); onClick() }}>
                <sphereGeometry args={[node.radius, 32, 32]} />
                <meshStandardMaterial
                    map={texture}
                    emissive={emissiveColour}
                    emissiveIntensity={emissiveIntensity}
                    roughness={node.kind === 'root' ? 1.0 : node.kind === 'signal' ? 0.85 : 0.55}
                    metalness={0.05}
                    transparent={ghost}
                    opacity={ghost ? 0.45 : 1}
                />
            </mesh>
            {node.kind === 'root' && !ghost && (
                <pointLight intensity={1.5} distance={30} color="#fff8e0" />
            )}
            {showLabels && !ghost && (
                <Html center distanceFactor={8} position={[0, node.radius * 1.3 + 0.12, 0]} style={{ pointerEvents: 'none' }}>
                    <span style={{
                        fontSize: node.kind === 'root' ? '13px' : node.kind === 'dimension' ? '11px' : '9px',
                        fontWeight: node.kind !== 'signal' ? 600 : 400,
                        color: node.kind === 'root' ? '#fde68a' : '#f8fafc',
                        textShadow: '0 1px 4px #000,0 0 8px #000',
                        whiteSpace: 'nowrap',
                        userSelect: 'none',
                    }}>
                        {node.label.length > 20 ? node.label.slice(0, 18) + '…' : node.label}
                    </span>
                </Html>
            )}
        </group>
    )
}

// ── EdgeLine ──────────────────────────────────────────────────────────────────

interface EdgeLineProps {
    from: [number, number, number]
    to: [number, number, number]
    weight: number
    color?: string
}

function EdgeLine({ from, to, weight, color }: EdgeLineProps) {
    const points = useMemo<[number, number, number][]>(() => [from, to], [from, to])
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

// ── Score axis (horizontal X) ─────────────────────────────────────────────────

function ScoreAxis() {
    const TICKS = [0, 25, 50, 75, 100]
    const N_SEGS = 64
    return (
        <>
            <Line points={[[-0.6, 0, 0], [X_SCALE + 0.9, 0, 0]]} color="#334155" lineWidth={1.5} />
            {TICKS.map(score => {
                const x = (score / 100) * X_SCALE
                const ringPts: [number, number, number][] = Array.from({ length: N_SEGS + 1 }, (_, i) => {
                    const a = (2 * Math.PI * i) / N_SEGS
                    return [x, GRID_RING_RADIUS * Math.cos(a), GRID_RING_RADIUS * Math.sin(a)]
                })
                return (
                    <group key={score}>
                        <Line points={ringPts} color="#1e293b" lineWidth={0.7} />
                        <Line points={[[x, -0.25, 0], [x, 0.25, 0]]} color="#475569" lineWidth={1.5} />
                        <Html position={[x, -1.2, 0]} center style={{ pointerEvents: 'none' }}>
                            <span style={{
                                color: score === 0 || score === 100 ? '#94a3b8' : '#475569',
                                fontSize: '10px', userSelect: 'none', fontFamily: 'monospace', fontWeight: 600,
                            }}>{score}</span>
                        </Html>
                    </group>
                )
            })}
            <Html position={[X_SCALE + 1.3, 0, 0]} center style={{ pointerEvents: 'none' }}>
                <span style={{ color: '#64748b', fontSize: '11px', userSelect: 'none', letterSpacing: '0.05em' }}>Score →</span>
            </Html>
        </>
    )
}

// ── Gravity helpers ───────────────────────────────────────────────────────────

const GRAVITY_SIGNAL_FORCE = 0.55

function buildScoreForces(nodes: GraphNode[]): Map<string, number> {
    const map = new Map<string, number>()
    for (const node of nodes) {
        if (node.kind === 'signal') map.set(node.id, node.passed ? +GRAVITY_SIGNAL_FORCE : -GRAVITY_SIGNAL_FORCE)
    }
    return map
}

const DIM_SPRING_BASE = 5.0, DIM_SPRING_K = 0.35, MIN_DIM_LEN = 1.5, MAX_DIM_LEN = 16.0

function buildDimEdgeLengths(nodes: GraphNode[], edges: GraphEdge[]): Map<string, number> {
    const map = new Map<string, number>()
    const nodeById = new Map(nodes.map(n => [n.id, n]))
    const rootNode = nodes.find(n => n.kind === 'root')
    if (!rootNode) return map
    for (const edge of edges) {
        if (edge.from !== rootNode.id) continue
        const dim = nodeById.get(edge.to)
        if (!dim || dim.kind !== 'dimension') continue
        let forceSum = 0
        for (const sigEdge of edges) {
            if (sigEdge.from !== dim.id) continue
            const sig = nodeById.get(sigEdge.to)
            if (!sig || sig.kind !== 'signal') continue
            forceSum += sig.passed ? -(sig.max_points * DIM_SPRING_K) : +(sig.max_points * DIM_SPRING_K)
        }
        map.set(`${edge.from}->${edge.to}`, Math.max(MIN_DIM_LEN, Math.min(MAX_DIM_LEN, DIM_SPRING_BASE + forceSum)))
    }
    return map
}

// ── Solar system types ────────────────────────────────────────────────────────

interface SolarMoonParams {
    node: GraphNode
    orbitRadius: number
    orbitSpeed: number
    orbitPhase: number
}

interface SolarPlanetParams {
    node: GraphNode
    orbitRadius: number
    orbitSpeed: number
    orbitPhase: number
    moons: SolarMoonParams[]
    dimIndex: number
}

interface SolarParams {
    rootNode: GraphNode
    starX: number
    planets: SolarPlanetParams[]
}

function buildSolarSystem(nodes: GraphNode[], edges: GraphEdge[], config: NodeSizeConfig): SolarParams {
    const rawRoot = nodes.find(n => n.kind === 'root')!
    const starX = (rawRoot.score / 100) * X_SCALE
    const rootNode: GraphNode = {
        ...rawRoot,
        radius: config.root_base_radius + config.score_scale * rawRoot.score,
        px: starX, py: 0, pz: 0,
    }

    const nodeById = new Map(nodes.map(n => [n.id, n]))
    const dimIds = edges.filter(e => e.from === rawRoot.id).map(e => e.to)

    const planets: SolarPlanetParams[] = dimIds.map((dimId, idx) => {
        const raw = nodeById.get(dimId)!
        const dimNode: GraphNode = {
            ...raw,
            radius: (config.dim_base_radius + config.score_scale * raw.score + raw.signal_count * config.dim_signal_scale) * 0.9,
        }
        const orbitRadius = SOLAR_PLANET_BASE + idx * SOLAR_PLANET_GAP
        const orbitSpeed = 0.22 / Math.pow(orbitRadius / SOLAR_PLANET_BASE, 1.5)
        const orbitPhase = (2 * Math.PI * idx) / Math.max(1, dimIds.length)

        const sigIds = edges.filter(e => e.from === dimId).map(e => e.to)
        const moons: SolarMoonParams[] = sigIds.map((sigId, mIdx) => {
            const sigRaw = nodeById.get(sigId)!
            // Moon size: proportional to this signal's share of the dimension's total max_points
            const proportion = raw.max_points > 0 ? sigRaw.max_points / raw.max_points : 1 / Math.max(1, sigIds.length)
            const sigNode: GraphNode = {
                ...sigRaw,
                radius: 0.08 + proportion * 0.38,
            }
            const moonRadius = SOLAR_MOON_BASE + mIdx * SOLAR_MOON_GAP
            const moonSpeed = 0.75 / Math.pow(moonRadius / SOLAR_MOON_BASE, 1.5)
            const moonPhase = (2 * Math.PI * mIdx) / Math.max(1, sigIds.length)
            return { node: sigNode, orbitRadius: moonRadius, orbitSpeed: moonSpeed, orbitPhase: moonPhase }
        })

        return { node: dimNode, orbitRadius, orbitSpeed, orbitPhase, moons, dimIndex: idx }
    })

    return { rootNode, starX, planets }
}

// ── StarNode ──────────────────────────────────────────────────────────────────

interface StarNodeProps {
    node: GraphNode
    selected: boolean
    onClick: () => void
    showLabels: boolean
}

function StarNode({ node, selected, onClick, showLabels }: StarNodeProps) {
    const surfRef = useRef<THREE.Mesh>(null!)
    const c1Ref = useRef<THREE.Mesh>(null!)
    const c2Ref = useRef<THREE.Mesh>(null!)

    useFrame(({ clock }) => {
        const t = clock.getElapsedTime()
        if (surfRef.current) surfRef.current.rotation.y = t * 0.07
        if (c1Ref.current) c1Ref.current.scale.setScalar(1 + 0.04 * Math.sin(t * 1.3))
        if (c2Ref.current) c2Ref.current.scale.setScalar(1 + 0.07 * Math.sin(t * 0.8 + 1.5))
    })

    return (
        <group position={[node.px, node.py, node.pz]}>
            {/* Outer corona */}
            <mesh ref={c2Ref}>
                <sphereGeometry args={[node.radius * 2.6, 32, 32]} />
                <meshStandardMaterial color="#ff3300" transparent opacity={0.05} side={THREE.BackSide} depthWrite={false} />
            </mesh>
            {/* Inner corona */}
            <mesh ref={c1Ref}>
                <sphereGeometry args={[node.radius * 1.75, 32, 32]} />
                <meshStandardMaterial color="#ffaa00" transparent opacity={0.12} side={THREE.BackSide} depthWrite={false} />
            </mesh>
            {/* Star surface */}
            <mesh ref={surfRef} onClick={(e) => { e.stopPropagation(); onClick() }}>
                <sphereGeometry args={[node.radius, 32, 32]} />
                <meshStandardMaterial
                    map={STAR_TEX}
                    emissive="#ff7700"
                    emissiveIntensity={selected ? 1.7 : 1.3}
                    roughness={1.0}
                    metalness={0}
                />
            </mesh>
            {/* Solar point light */}
            <pointLight intensity={6.0} distance={120} color="#fff5e0" />
            {showLabels && (
                <Html center distanceFactor={8} position={[0, node.radius * 1.6 + 0.12, 0]} style={{ pointerEvents: 'none' }}>
                    <span style={{
                        fontSize: '13px', fontWeight: 700, color: '#fde68a',
                        textShadow: '0 0 8px #ff8800, 0 1px 4px #000',
                        whiteSpace: 'nowrap', userSelect: 'none',
                    }}>
                        {node.label.length > 20 ? node.label.slice(0, 18) + '…' : node.label}
                    </span>
                </Html>
            )}
        </group>
    )
}

// ── OrbitingMoon ──────────────────────────────────────────────────────────────

interface OrbitingMoonProps {
    moon: SolarMoonParams
    selected: boolean
    onSelect: () => void
    showLabels: boolean
}

function OrbitingMoon({ moon, selected, onSelect, showLabels }: OrbitingMoonProps) {
    const groupRef = useRef<THREE.Group>(null!)
    const meshRef = useRef<THREE.Mesh>(null!)
    const initialPos: [number, number, number] = [
        moon.orbitRadius * Math.cos(moon.orbitPhase), 0, moon.orbitRadius * Math.sin(moon.orbitPhase),
    ]

    useFrame(({ clock }) => {
        const t = clock.getElapsedTime()
        if (groupRef.current) {
            const a = moon.orbitPhase + t * moon.orbitSpeed
            groupRef.current.position.set(moon.orbitRadius * Math.cos(a), 0, moon.orbitRadius * Math.sin(a))
        }
        if (meshRef.current) meshRef.current.rotation.y += 0.006
    })

    const texture = (moon.node.highlight || !moon.node.passed) ? PLANET_TEX.signal_fail : PLANET_TEX.signal_pass
    const atmosColor = moon.node.highlight || !moon.node.passed ? '#ef4444' : '#22c55e'

    return (
        <group ref={groupRef} position={initialPos}>
            <mesh>
                <sphereGeometry args={[moon.node.radius * 1.2, 20, 20]} />
                <meshStandardMaterial color={atmosColor} transparent opacity={0.10} side={THREE.BackSide} depthWrite={false} />
            </mesh>
            <mesh ref={meshRef} onClick={(e) => { e.stopPropagation(); onSelect() }}>
                <sphereGeometry args={[moon.node.radius, 22, 22]} />
                <meshStandardMaterial
                    map={texture}
                    emissive={selected ? '#60a5fa' : '#000000'}
                    emissiveIntensity={selected ? 0.5 : 0}
                    roughness={0.85}
                    metalness={0.05}
                />
            </mesh>
            {showLabels && (
                <Html center distanceFactor={8} position={[0, moon.node.radius * 1.4 + 0.08, 0]} style={{ pointerEvents: 'none' }}>
                    <span style={{
                        fontSize: '8px', color: '#f8fafc',
                        textShadow: '0 1px 3px #000', whiteSpace: 'nowrap', userSelect: 'none',
                    }}>
                        {moon.node.label.length > 16 ? moon.node.label.slice(0, 14) + '…' : moon.node.label}
                    </span>
                </Html>
            )}
        </group>
    )
}

// ── OrbitingPlanet ────────────────────────────────────────────────────────────

interface OrbitingPlanetProps {
    planet: SolarPlanetParams
    starX: number
    selected: string | null
    onSelect: (id: string) => void
    showLabels: boolean
}

function OrbitingPlanet({ planet, starX, selected, onSelect, showLabels }: OrbitingPlanetProps) {
    const groupRef = useRef<THREE.Group>(null!)
    const meshRef = useRef<THREE.Mesh>(null!)
    const initialPos: [number, number, number] = [
        starX + planet.orbitRadius * Math.cos(planet.orbitPhase), 0,
        planet.orbitRadius * Math.sin(planet.orbitPhase),
    ]

    const moonRings = useMemo(
        () => planet.moons.map(m => {
            const N = 64
            return Array.from({ length: N + 1 }, (_, i) => {
                const a = (2 * Math.PI * i) / N
                return [m.orbitRadius * Math.cos(a), 0, m.orbitRadius * Math.sin(a)] as [number, number, number]
            })
        }),
        [planet.moons]
    )

    useFrame(({ clock }) => {
        const t = clock.getElapsedTime()
        if (groupRef.current) {
            const a = planet.orbitPhase + t * planet.orbitSpeed
            groupRef.current.position.set(
                starX + planet.orbitRadius * Math.cos(a), 0, planet.orbitRadius * Math.sin(a),
            )
        }
        if (meshRef.current) meshRef.current.rotation.y += 0.004
    })

    const texture = GAS_GIANT_TEXTURES[planet.dimIndex % GAS_GIANT_TEXTURES.length]
    const isSelected = selected === planet.node.id

    return (
        <group ref={groupRef} position={initialPos}>
            <mesh>
                <sphereGeometry args={[planet.node.radius * 1.18, 30, 30]} />
                <meshStandardMaterial color="#0ea5e9" transparent opacity={0.12} side={THREE.BackSide} depthWrite={false} />
            </mesh>
            <mesh ref={meshRef} onClick={(e) => { e.stopPropagation(); onSelect(planet.node.id) }}>
                <sphereGeometry args={[planet.node.radius, 32, 32]} />
                <meshStandardMaterial
                    map={texture}
                    emissive={isSelected ? '#60a5fa' : '#000000'}
                    emissiveIntensity={isSelected ? 0.4 : 0}
                    roughness={0.55}
                    metalness={0.05}
                />
            </mesh>
            {showLabels && (
                <Html center distanceFactor={8} position={[0, planet.node.radius * 1.3 + 0.12, 0]} style={{ pointerEvents: 'none' }}>
                    <span style={{
                        fontSize: '10px', fontWeight: 600, color: '#f8fafc',
                        textShadow: '0 1px 4px #000', whiteSpace: 'nowrap', userSelect: 'none',
                    }}>
                        {planet.node.label.length > 18 ? planet.node.label.slice(0, 16) + '…' : planet.node.label}
                    </span>
                </Html>
            )}
            {moonRings.map((pts, i) => (
                <Line key={i} points={pts} color="#1e293b" lineWidth={0.4} />
            ))}
            {planet.moons.map(moon => (
                <OrbitingMoon
                    key={moon.node.id}
                    moon={moon}
                    selected={selected === moon.node.id}
                    onSelect={() => onSelect(moon.node.id)}
                    showLabels={showLabels}
                />
            ))}
        </group>
    )
}

// ── SolarScene ────────────────────────────────────────────────────────────────

interface SolarSceneProps {
    graph: DecisionGraph
    config: NodeSizeConfig
    selected: string | null
    onSelectNode: (id: string) => void
    showLabels: boolean
}

function SolarScene({ graph, config, selected, onSelectNode, showLabels }: SolarSceneProps) {
    const solar = useMemo(() => buildSolarSystem(graph.nodes, graph.edges, config), [graph, config])

    const planetRings = useMemo(
        () => solar.planets.map(p => {
            const N = 96
            return Array.from({ length: N + 1 }, (_, i) => {
                const a = (2 * Math.PI * i) / N
                return [solar.starX + p.orbitRadius * Math.cos(a), 0, p.orbitRadius * Math.sin(a)] as [number, number, number]
            })
        }),
        [solar]
    )

    return (
        <>
            <Stars radius={180} depth={70} count={6000} factor={5} saturation={0.3} fade speed={0.5} />
            <ambientLight intensity={0.35} color="#c8d8ff" />
            <directionalLight position={[20, 15, 10]} intensity={0.6} color="#fff8f0" />
            {planetRings.map((pts, i) => (
                <Line key={i} points={pts} color="#1e293b" lineWidth={0.5} />
            ))}
            <StarNode
                node={solar.rootNode}
                selected={selected === solar.rootNode.id}
                onClick={() => onSelectNode(solar.rootNode.id)}
                showLabels={showLabels}
            />
            {solar.planets.map(planet => (
                <OrbitingPlanet
                    key={planet.node.id}
                    planet={planet}
                    starX={solar.starX}
                    selected={selected}
                    onSelect={onSelectNode}
                    showLabels={showLabels}
                />
            ))}
            <OrbitControls makeDefault target={[solar.starX, 0, 0]} />
        </>
    )
}

// ── Scene (Grid / Radial / Force / Gravity) ───────────────────────────────────

interface SceneProps {
    graph: DecisionGraph
    visibleKinds: Set<string>
    highlightOnly: boolean
    selected: string | null
    onSelectNode: (id: string) => void
    deltaMap: Map<string, number>
    ghostNodes: GraphNode[]
    showLabels: boolean
    showGrid: boolean
}

function Scene({ graph, visibleKinds, highlightOnly, selected, onSelectNode, deltaMap, ghostNodes, showLabels, showGrid }: SceneProps) {
    const nodeMap = useMemo(() => {
        const m = new Map<string, GraphNode>()
        graph.nodes.forEach(n => m.set(n.id, n))
        return m
    }, [graph])

    const visibleNodes = useMemo(
        () => graph.nodes.filter(n => visibleKinds.has(n.kind) && (!highlightOnly || n.highlight)),
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
            <Stars radius={160} depth={60} count={6000} factor={5} saturation={0.3} fade speed={0.6} />
            <ambientLight intensity={0.25} />
            <directionalLight position={[15, 20, 10]} intensity={1.6} color="#fff8f0" />
            <pointLight position={[-12, -8, -12]} intensity={0.5} color="#3b82f6" />
            <OrbitControls makeDefault />
            {showGrid && <ScoreAxis />}
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

// ── Inspector ─────────────────────────────────────────────────────────────────

interface InspectorProps {
    node: GraphNode | null
    delta?: number
}

function Inspector({ node, delta }: InspectorProps) {
    return (
        <aside className={styles.inspector} data-testid="node-inspector">
            <h3 className={styles.inspectorTitle}>Inspector</h3>
            {node === null ? (
                <p className={styles.placeholder} data-testid="inspector-placeholder">Select a node to inspect</p>
            ) : (
                <dl className={styles.nodeDetail}>
                    <dt>ID</dt><dd data-testid="inspector-node-id">{node.id}</dd>
                    <dt>Label</dt><dd>{node.label}</dd>
                    <dt>Kind</dt><dd>{node.kind}</dd>
                    <dt>Score</dt><dd>{node.score.toFixed(1)}{node.max_points > 0 ? ` / ${node.max_points.toFixed(0)} pts` : ''}</dd>
                    <dt>Weight</dt><dd>{node.weight.toFixed(3)}</dd>
                    {node.signal_count > 0 && (<><dt>Signals</dt><dd>{node.signal_count}</dd></>)}
                    <dt>Radius</dt><dd>{node.radius.toFixed(3)}</dd>
                    <dt>Status</dt>
                    <dd>
                        {node.passed ? '✅ Passed' : '❌ Failed'}
                        {node.highlight && (
                            <span className={styles.highlightBadge} data-testid="inspector-highlight-badge">⚠ Highlight</span>
                        )}
                    </dd>
                    {node.has_detail && (<><dt>Detail</dt><dd data-testid="inspector-has-detail">📝 Has actionable detail</dd></>)}
                    {delta !== undefined && (
                        <><dt>Δ Score</dt>
                        <dd className={delta > 0 ? styles.deltaPositive : delta < 0 ? styles.deltaNegative : ''} data-testid="inspector-delta">
                            {delta > 0 ? '+' : ''}{delta.toFixed(1)}
                        </dd></>
                    )}
                </dl>
            )}
        </aside>
    )
}

// ── Main view ─────────────────────────────────────────────────────────────────

type LayoutMode = 'solar' | 'grid' | 'gravity' | 'radial' | 'force'

export default function MaturityGraphView() {
    const [showLabels, setShowLabels] = useState(true)
    const [scans, setScans] = useState<ScanSummary[]>([])
    const [selectedScanId, setSelectedScanId] = useState<string>('')
    const [graph, setGraph] = useState<DecisionGraph | null>(null)
    const [loadingGraph, setLoadingGraph] = useState(false)
    const [error, setError] = useState<string | null>(null)
    const [comparisonScanId, setComparisonScanId] = useState<string>('')
    const [comparisonGraph, setComparisonGraph] = useState<DecisionGraph | null>(null)
    const [loadingComparison, setLoadingComparison] = useState(false)
    const [layoutMode, setLayoutMode] = useState<LayoutMode>('solar')
    const [forceNodes, setForceNodes] = useState<GraphNode[] | null>(null)
    const rafRef = useRef<number | null>(null)
    const simParticlesRef = useRef<SimParticle[]>([])
    const simFrameRef = useRef(0)
    const [showRoot, setShowRoot] = useState(true)
    const [showDimension, setShowDimension] = useState(true)
    const [showSignal, setShowSignal] = useState(true)
    const [highlightOnly, setHighlightOnly] = useState(false)
    const [tierFilter, setTierFilter] = useState(0)
    const [selectedNodeId, setSelectedNodeId] = useState<string | null>(null)
    const [layoutConfig, setLayoutConfig] = useState<NodeSizeConfig>(() => {
        try {
            const saved = localStorage.getItem(LOCALSTORAGE_KEY)
            return saved ? (JSON.parse(saved) as NodeSizeConfig) : DEFAULT_LAYOUT
        } catch { return DEFAULT_LAYOUT }
    })

    useEffect(() => {
        fetch('/scans?limit=50')
            .then(r => r.json() as Promise<ApiResponse<ScanSummary[]>>)
            .then(res => {
                if (res.success && res.data.length > 0) {
                    setScans(res.data)
                    setSelectedScanId(res.data[0].id)
                }
            })
            .catch(() => setError('Failed to load scans'))
    }, [])

    useEffect(() => {
        if (!selectedScanId) return
        setLoadingGraph(true); setGraph(null); setSelectedNodeId(null); setError(null)
        fetch(`/scans/${selectedScanId}/decision-graph`)
            .then(r => r.json() as Promise<ApiResponse<DecisionGraph>>)
            .then(res => { if (res.success) setGraph(res.data); else setError('No maturity graph for this scan') })
            .catch(() => setError('Failed to load maturity graph'))
            .finally(() => setLoadingGraph(false))
    }, [selectedScanId])

    useEffect(() => {
        if (!comparisonScanId) { setComparisonGraph(null); return }
        setLoadingComparison(true); setComparisonGraph(null)
        fetch(`/scans/${comparisonScanId}/decision-graph`)
            .then(r => r.json() as Promise<ApiResponse<DecisionGraph>>)
            .then(res => { if (res.success) setComparisonGraph(res.data) })
            .catch(() => { })
            .finally(() => setLoadingComparison(false))
    }, [comparisonScanId])

    const visibleKinds = useMemo(() => {
        const s = new Set<string>()
        if (showRoot) s.add('root')
        if (showDimension) s.add('dimension')
        if (showSignal) s.add('signal')
        return s
    }, [showRoot, showDimension, showSignal])

    const activeGraph = useMemo(() => {
        if (!graph) return null
        if (layoutMode === 'solar' || layoutMode === 'grid' || layoutMode === 'gravity') {
            return { ...graph, nodes: gridLayout(graph.nodes, graph.edges, layoutConfig) }
        }
        return { ...graph, nodes: recomputeLayout(graph.nodes, graph.edges, layoutConfig) }
    }, [graph, layoutConfig, layoutMode])

    useEffect(() => {
        if (rafRef.current !== null) { cancelAnimationFrame(rafRef.current); rafRef.current = null }
        if ((layoutMode !== 'force' && layoutMode !== 'gravity') || !activeGraph) {
            setForceNodes(null); return
        }

        simParticlesRef.current = initParticles(activeGraph.nodes)
        simFrameRef.current = 0
        const edges = activeGraph.edges
        const SIM_STEPS_PER_FRAME = 10
        const SIM_MAX_FRAMES = 300

        let simOptions: Parameters<typeof stepSimulation>[2]
        if (layoutMode === 'gravity') {
            const rootNode = activeGraph.nodes.find(n => n.kind === 'root')
            const pinAt = rootNode ? { x: rootNode.px, y: rootNode.py, z: rootNode.pz } : undefined
            simOptions = {
                pinRoot: true, pinAt,
                scoreForces: buildScoreForces(activeGraph.nodes),
                springK: 0.12, springLen: 4.0, repulsion: 20, damping: 0.88,
            }
        } else {
            simOptions = { edgeLengths: buildDimEdgeLengths(activeGraph.nodes, edges), springK: 0.28, repulsion: 14 }
        }

        function tick() {
            for (let s = 0; s < SIM_STEPS_PER_FRAME; s++) stepSimulation(simParticlesRef.current, edges, simOptions)
            setForceNodes(extractPositions(activeGraph!.nodes, simParticlesRef.current))
            simFrameRef.current += 1
            if (simFrameRef.current < SIM_MAX_FRAMES) { rafRef.current = requestAnimationFrame(tick) }
            else { rafRef.current = null }
        }
        rafRef.current = requestAnimationFrame(tick)
        return () => { if (rafRef.current !== null) { cancelAnimationFrame(rafRef.current); rafRef.current = null } }
        // eslint-disable-next-line react-hooks/exhaustive-deps
    }, [layoutMode, activeGraph])

    const activeComparisonGraph = useMemo(() => {
        if (!comparisonGraph) return null
        if (layoutMode === 'solar' || layoutMode === 'grid' || layoutMode === 'gravity') {
            return { ...comparisonGraph, nodes: gridLayout(comparisonGraph.nodes, comparisonGraph.edges, layoutConfig) }
        }
        return { ...comparisonGraph, nodes: recomputeLayout(comparisonGraph.nodes, comparisonGraph.edges, layoutConfig) }
    }, [comparisonGraph, layoutConfig, layoutMode])

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

    const ghostNodes = useMemo<GraphNode[]>(() => {
        if (!activeComparisonGraph || !activeGraph) return []
        const primaryIds = new Set(activeGraph.nodes.map(n => n.id))
        return activeComparisonGraph.nodes.filter(n => !primaryIds.has(n.id))
    }, [activeGraph, activeComparisonGraph])

    const displayGraph = useMemo(() => {
        if (!activeGraph) return null
        if ((layoutMode === 'force' || layoutMode === 'gravity') && forceNodes) {
            return { ...activeGraph, nodes: forceNodes }
        }
        return activeGraph
    }, [activeGraph, layoutMode, forceNodes])

    const visibleNodes = useMemo(() => {
        if (!displayGraph) return []
        return displayGraph.nodes.filter(n =>
            visibleKinds.has(n.kind) &&
            (!highlightOnly || n.highlight) &&
            (tierFilter === 0 || n.kind !== 'signal' || n.tier === tierFilter)
        )
    }, [displayGraph, visibleKinds, highlightOnly, tierFilter])

    const selectedNode = useMemo(
        () => selectedNodeId ? displayGraph?.nodes.find(n => n.id === selectedNodeId) ?? null : null,
        [selectedNodeId, displayGraph]
    )

    function handleReset() {
        const defaults = graph?.config ?? DEFAULT_LAYOUT
        setLayoutConfig(defaults)
        localStorage.removeItem(LOCALSTORAGE_KEY)
    }

    const starX = useMemo(() => {
        if (!activeGraph) return X_SCALE / 2
        return activeGraph.nodes.find(n => n.kind === 'root')?.px ?? X_SCALE / 2
    }, [activeGraph])

    const cameraPos: [number, number, number] =
        layoutMode === 'solar'   ? [starX, 14, 22]
        : layoutMode === 'grid' || layoutMode === 'gravity' ? [18, 8, 18]
        : [0, 4, 14]

    const canvasKey = layoutMode === 'solar' ? 'solar' : layoutMode === 'grid' || layoutMode === 'gravity' ? 'grid' : 'other'

    return (
        <div className={styles.root} data-testid="maturity-graph-view">
            {/* ── LEFT PANEL ─────────────────────────────────────────────────── */}
            <aside className={styles.sidebar}>
                <div className={styles.selectorWrap}>
                    <label className={styles.label} htmlFor="scan-selector">Scan</label>
                    <select
                        id="scan-selector" data-testid="scan-selector"
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

                <div className={styles.compareWrap}>
                    <label className={styles.label} htmlFor="compare-selector">Compare to</label>
                    <select
                        id="compare-selector" data-testid="compare-selector"
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

                <div className={styles.filterPanel} data-testid="filter-panel">
                    <p className={styles.filterTitle}>Filters</p>
                    <div className={styles.filterChips}>
                        <button className={`${styles.filterChip} ${showRoot ? styles.filterChipActive : ''}`}
                            style={{ '--chip-color': '#fde68a' } as React.CSSProperties} onClick={() => setShowRoot(v => !v)}>
                            <span className={styles.filterChipDot} />Root
                        </button>
                        <button className={`${styles.filterChip} ${showDimension ? styles.filterChipActive : ''}`}
                            style={{ '--chip-color': '#38bdf8' } as React.CSSProperties} onClick={() => setShowDimension(v => !v)}>
                            <span className={styles.filterChipDot} />Dimension
                        </button>
                        <button className={`${styles.filterChip} ${showSignal ? styles.filterChipActive : ''}`}
                            style={{ '--chip-color': '#4ade80' } as React.CSSProperties} onClick={() => setShowSignal(v => !v)}>
                            <span className={styles.filterChipDot} />Signal
                        </button>
                        <button className={`${styles.filterChip} ${highlightOnly ? styles.filterChipActive : ''}`}
                            style={{ '--chip-color': '#f87171' } as React.CSSProperties} onClick={() => setHighlightOnly(v => !v)}>
                            <span className={styles.filterChipDot} />Failed only
                        </button>
                    </div>
                    <p className={styles.filterTitle}>Tier</p>
                    <div className={styles.filterChips}>
                        {([0, 1, 2, 3] as const).map(t => (
                            <button key={t}
                                className={`${styles.filterChip} ${tierFilter === t ? styles.filterChipActive : ''}`}
                                style={{ '--chip-color': t === 2 ? '#4fa3e0' : t === 3 ? '#f0c040' : '#94a3b8' } as React.CSSProperties}
                                onClick={() => setTierFilter(t)}>
                                <span className={styles.filterChipDot} />{t === 0 ? 'All' : `T${t}`}
                            </button>
                        ))}
                    </div>
                    <p className={styles.nodeCountLabel}>
                        Showing <span data-testid="node-count">{visibleNodes.length}</span> nodes
                    </p>
                </div>

                <div className={styles.layoutPanel} data-testid="layout-panel">
                    <p className={styles.filterTitle}>Layout</p>
                    <div data-testid="layout-toggle" className={styles.modeToggle}>
                        <button className={`${styles.modeBtn} ${layoutMode === 'solar' ? styles.modeBtnActive : ''}`}
                            data-testid="layout-mode-solar" onClick={() => setLayoutMode('solar')}>Solar</button>
                        <button className={`${styles.modeBtn} ${layoutMode === 'grid' ? styles.modeBtnActive : ''}`}
                            data-testid="layout-mode-grid" onClick={() => setLayoutMode('grid')}>Grid</button>
                        <button className={`${styles.modeBtn} ${layoutMode === 'gravity' ? styles.modeBtnActive : ''}`}
                            data-testid="layout-mode-gravity" onClick={() => setLayoutMode('gravity')}>Gravity</button>
                        <button className={`${styles.modeBtn} ${layoutMode === 'radial' ? styles.modeBtnActive : ''}`}
                            data-testid="layout-mode-radial" onClick={() => setLayoutMode('radial')}>Radial</button>
                        <button className={`${styles.modeBtn} ${layoutMode === 'force' ? styles.modeBtnActive : ''}`}
                            data-testid="layout-mode-force" onClick={() => setLayoutMode('force')}>Force</button>
                    </div>

                    <div className={styles.toggleRow}>
                        <span className={styles.toggleRowLabel}>Score-driven size</span>
                        <button
                            className={`${styles.toggleSwitch} ${(layoutConfig.score_scale !== 0 || layoutConfig.sig_points_scale !== 0) ? styles.toggleSwitchOn : ''}`}
                            onClick={() => {
                                const on = !(layoutConfig.score_scale !== 0 || layoutConfig.sig_points_scale !== 0)
                                setLayoutConfig(prev => {
                                    const next = { ...prev, score_scale: on ? DEFAULT_LAYOUT.score_scale : 0, sig_points_scale: on ? DEFAULT_LAYOUT.sig_points_scale : 0 }
                                    localStorage.setItem(LOCALSTORAGE_KEY, JSON.stringify(next))
                                    return next
                                })
                            }}
                        >
                            <span className={styles.toggleThumb} />
                        </button>
                    </div>

                    <div className={styles.toggleRow}>
                        <span className={styles.toggleRowLabel}>Show labels</span>
                        <button className={`${styles.toggleSwitch} ${showLabels ? styles.toggleSwitchOn : ''}`}
                            data-testid="labels-toggle" onClick={() => setShowLabels(v => !v)}>
                            <span className={styles.toggleThumb} />
                        </button>
                    </div>

                    <button className={styles.resetBtn} onClick={handleReset}>Reset to defaults</button>
                </div>

                <ul className={styles.nodeList}>
                    {visibleNodes.map(node => (
                        <li key={node.id}
                            className={`${styles.nodeListItem} ${selectedNodeId === node.id ? styles.nodeListItemActive : ''} ${node.highlight ? styles.nodeListItemHighlight : ''}`}
                            data-testid="node-list-item" data-node-id={node.id}
                            onClick={() => setSelectedNodeId(node.id)}>
                            <span className={styles.nodeKindDot} style={{ background: node.highlight ? HIGHLIGHT_COLOUR : KIND_COLOUR[node.kind] }} />
                            <span className={styles.nodeListLabel}>{node.label}</span>
                        </li>
                    ))}
                </ul>
            </aside>

            {/* ── CANVAS ─────────────────────────────────────────────────────── */}
            <div className={styles.canvasWrap}>
                {loadingGraph && <p className={styles.loadingMsg}>Loading graph…</p>}
                {error && <p className={styles.errorMsg}>{error}</p>}
                {graph && !loadingGraph && (
                    <Canvas
                        key={canvasKey}
                        camera={{ position: cameraPos, fov: layoutMode === 'solar' ? 65 : 50 }}
                        style={{ background: '#000008' }}
                    >
                        {layoutMode === 'solar' && (
                            <SolarScene
                                graph={graph}
                                config={layoutConfig}
                                selected={selectedNodeId}
                                onSelectNode={id => setSelectedNodeId(id === selectedNodeId ? null : id)}
                                showLabels={showLabels}
                            />
                        )}
                        {layoutMode !== 'solar' && displayGraph && (
                            <Scene
                                graph={displayGraph}
                                visibleKinds={visibleKinds}
                                highlightOnly={highlightOnly}
                                selected={selectedNodeId}
                                onSelectNode={id => setSelectedNodeId(id === selectedNodeId ? null : id)}
                                deltaMap={deltaMap}
                                ghostNodes={ghostNodes}
                                showLabels={showLabels}
                                showGrid={layoutMode === 'grid' || layoutMode === 'gravity'}
                            />
                        )}
                    </Canvas>
                )}
            </div>

            {/* ── INSPECTOR ──────────────────────────────────────────────────── */}
            <Inspector node={selectedNode ?? null} delta={selectedNodeId ? deltaMap.get(selectedNodeId) : undefined} />
        </div>
    )
}
