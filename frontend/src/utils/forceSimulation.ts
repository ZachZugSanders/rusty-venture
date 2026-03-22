/**
 * Force-directed layout simulation (Phase 5).
 *
 * A simple Verlet-integration spring-graph engine:
 *   • Coulomb repulsion between every node pair
 *   • Hooke spring attraction along graph edges
 *   • Optional root pinning (keeps the root node at origin)
 *
 * The public surface is intentionally separated into three levels:
 *   1. `initParticles`     — convert GraphNode[] to mutable SimParticle[]
 *   2. `stepSimulation`    — advance one iteration (mutates particles in-place)
 *   3. `extractPositions`  — map SimParticle[] positions back to GraphNode[]
 *   4. `runForceSimulation`— convenience batch runner (pure, used in unit tests)
 *
 * The view layer can call `initParticles` once, then call `stepSimulation` N
 * times per animation frame, and call `extractPositions` to get updated nodes
 * for rendering — without creating intermediate arrays each frame.
 */

import type { GraphNode, GraphEdge } from '../types'

// ── Public types ─────────────────────────────────────────────────────────────

export interface SimParticle {
    id: string
    kind: string
    x: number
    y: number
    z: number
    vx: number
    vy: number
    vz: number
}

export interface ForceOptions {
    /** Pin the root node. Default: true. */
    pinRoot?: boolean
    /**
     * World-space position to pin the root at.
     * Defaults to the origin (0,0,0) when `pinRoot` is true and this is omitted.
     */
    pinAt?: { x: number; y: number; z: number }
    /** Coulomb repulsion constant. Default: 30. */
    repulsion?: number
    /** Natural rest length for edge springs. Default: 3.0. */
    springLen?: number
    /** Spring stiffness constant. Default: 0.05. */
    springK?: number
    /** Velocity damping factor per step (0-1). Default: 0.85. */
    damping?: number
    /**
     * Per-edge override for the spring rest length.
     * Key format: `"${edge.from}->${edge.to}"`.
     * Falls back to `springLen` when not present.
     */
    edgeLengths?: Map<string, number>
    /**
     * Per-particle Y-axis force, keyed by particle id.
     * Positive = upward pull, negative = downward pull.
     * Used by gravity mode to make passing signals float and failing ones sink.
     */
    scoreForces?: Map<string, number>
}

export interface RunOptions extends ForceOptions {
    /** Number of simulation steps to run. Default: 300. */
    iterations?: number
}

// ── Core functions ────────────────────────────────────────────────────────────

/** Convert a GraphNode array into a mutable particle array at the same positions. */
export function initParticles(nodes: GraphNode[]): SimParticle[] {
    return nodes.map(n => ({
        id: n.id,
        kind: n.kind,
        x: n.px,
        y: n.py,
        z: n.pz,
        vx: 0,
        vy: 0,
        vz: 0,
    }))
}

/**
 * Advance the simulation by one step, mutating `particles` in-place.
 * Call this repeatedly (e.g. 8× per animation frame) for smooth animation.
 */
export function stepSimulation(
    particles: SimParticle[],
    edges: GraphEdge[],
    options: ForceOptions = {},
): void {
    const {
        pinRoot = true,
        pinAt,
        repulsion = 30,
        springLen = 3.0,
        springK = 0.05,
        damping = 0.85,
        edgeLengths,
        scoreForces,
    } = options

    const n = particles.length
    const forces = Array.from({ length: n }, () => ({ fx: 0, fy: 0, fz: 0 }))

    // Build id→index map once per step
    const idxById = new Map<string, number>()
    for (let i = 0; i < n; i++) idxById.set(particles[i].id, i)

    // ── Coulomb repulsion between every pair ──────────────────────────────
    for (let i = 0; i < n; i++) {
        for (let j = i + 1; j < n; j++) {
            let dx = particles[i].x - particles[j].x
            let dy = particles[i].y - particles[j].y
            let dz = particles[i].z - particles[j].z
            let d2 = dx * dx + dy * dy + dz * dz
            // Jitter to escape the singularity when nodes overlap exactly
            if (d2 < 0.0001) {
                dx = (Math.random() - 0.5) * 0.1
                dy = (Math.random() - 0.5) * 0.1
                dz = (Math.random() - 0.5) * 0.1
                d2 = dx * dx + dy * dy + dz * dz
            }
            const d = Math.sqrt(d2)
            const f = repulsion / d2
            const ux = dx / d, uy = dy / d, uz = dz / d
            forces[i].fx += ux * f; forces[i].fy += uy * f; forces[i].fz += uz * f
            forces[j].fx -= ux * f; forces[j].fy -= uy * f; forces[j].fz -= uz * f
        }
    }

    // ── Per-particle score forces (gravity mode) ────────────────────────────
    if (scoreForces) {
        for (let i = 0; i < n; i++) {
            const yf = scoreForces.get(particles[i].id)
            if (yf !== undefined) forces[i].fy += yf
        }
    }

    // ── Hooke spring attraction along edges ────────────────────────────────
    for (const edge of edges) {
        const i = idxById.get(edge.from)
        const j = idxById.get(edge.to)
        if (i === undefined || j === undefined) continue
        const dx = particles[j].x - particles[i].x
        const dy = particles[j].y - particles[i].y
        const dz = particles[j].z - particles[i].z
        const d = Math.sqrt(dx * dx + dy * dy + dz * dz) || 0.0001
        const restLen = edgeLengths?.get(`${edge.from}->${edge.to}`) ?? springLen
        const f = springK * (d - restLen)
        const ux = dx / d, uy = dy / d, uz = dz / d
        forces[i].fx += ux * f; forces[i].fy += uy * f; forces[i].fz += uz * f
        forces[j].fx -= ux * f; forces[j].fy -= uy * f; forces[j].fz -= uz * f
    }

    // ── Integrate velocities and positions ────────────────────────────────
    for (let i = 0; i < n; i++) {
        const p = particles[i]
        if (pinRoot && p.kind === 'root') {
            // Pin root at the specified position (default: world origin)
            p.x = pinAt?.x ?? 0; p.y = pinAt?.y ?? 0; p.z = pinAt?.z ?? 0
            p.vx = 0; p.vy = 0; p.vz = 0
            continue
        }
        p.vx = (p.vx + forces[i].fx) * damping
        p.vy = (p.vy + forces[i].fy) * damping
        p.vz = (p.vz + forces[i].fz) * damping
        p.x += p.vx
        p.y += p.vy
        p.z += p.vz
    }
}

/** Copy force-settled positions from particles back onto a GraphNode array. */
export function extractPositions(nodes: GraphNode[], particles: SimParticle[]): GraphNode[] {
    const posById = new Map(particles.map(p => [p.id, p]))
    return nodes.map(n => {
        const p = posById.get(n.id)
        if (!p) return n
        return { ...n, px: p.x, py: p.y, pz: p.z }
    })
}

/**
 * Pure batch runner — runs `iterations` steps from scratch and returns new nodes.
 *
 * Used by unit tests and by one-shot layout computation.
 * For animated layout the view layer should use `initParticles` + `stepSimulation`
 * + `extractPositions` to avoid recreating arrays each frame.
 */
export function runForceSimulation(
    nodes: GraphNode[],
    edges: GraphEdge[],
    options: RunOptions = {},
): GraphNode[] {
    const { iterations = 300, ...forceOpts } = options
    if (iterations === 0) return nodes.map(n => ({ ...n }))

    const particles = initParticles(nodes)
    for (let i = 0; i < iterations; i++) {
        stepSimulation(particles, edges, forceOpts)
    }
    return extractPositions(nodes, particles)
}
