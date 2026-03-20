/**
 * Unit tests for `runForceSimulation` (Phase 5).
 *
 * These tests are RED until `src/utils/forceSimulation.ts` is implemented.
 * They fully specify the expected behaviour so the implementation can be
 * written to make them green without guessing the contract.
 */
import { describe, it, expect } from 'vitest'
import { runForceSimulation } from '../utils/forceSimulation'
import type { GraphNode, GraphEdge } from '../types'

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

function makeNode(
    id: string,
    kind: GraphNode['kind'] = 'signal',
    px = 0,
    py = 0,
    pz = 0,
): GraphNode {
    return {
        id,
        label: id,
        kind,
        px,
        py,
        pz,
        radius: 0.12,
        score: 50,
        weight: 0.3,
        max_points: 100,
        signal_count: 0,
        has_detail: false,
        passed: true,
        highlight: false,
    }
}

function makeEdge(from: string, to: string): GraphEdge {
    return { from, to, weight: 0.5 }
}

function dist(a: GraphNode, b: GraphNode): number {
    return Math.sqrt((a.px - b.px) ** 2 + (a.py - b.py) ** 2 + (a.pz - b.pz) ** 2)
}

// ---------------------------------------------------------------------------
// Suite: Basic contract
// ---------------------------------------------------------------------------

describe('runForceSimulation — basic contract', () => {
    it('returns the same number of nodes as the input', () => {
        const nodes = [makeNode('a'), makeNode('b'), makeNode('c')]
        const result = runForceSimulation(nodes, [])
        expect(result).toHaveLength(3)
    })

    it('preserves every node id', () => {
        const nodes = [makeNode('a'), makeNode('b'), makeNode('c')]
        const result = runForceSimulation(nodes, [])
        const ids = result.map(n => n.id).sort()
        expect(ids).toEqual(['a', 'b', 'c'])
    })

    it('does not mutate the original node array', () => {
        const nodes = [makeNode('a', 'signal', 1, 0, 0), makeNode('b', 'signal', -1, 0, 0)]
        const origPx = nodes[0].px
        runForceSimulation(nodes, [])
        expect(nodes[0].px).toBe(origPx)
    })

    it('zero iterations returns positions identical to input', () => {
        const nodes = [makeNode('a', 'signal', 1, 2, 3), makeNode('b', 'signal', -1, 0, 0)]
        const result = runForceSimulation(nodes, [], { iterations: 0 })
        expect(result[0].px).toBeCloseTo(1)
        expect(result[0].py).toBeCloseTo(2)
        expect(result[0].pz).toBeCloseTo(3)
    })
})

// ---------------------------------------------------------------------------
// Suite: Repulsion
// ---------------------------------------------------------------------------

describe('runForceSimulation — repulsion', () => {
    it('separates two nodes that start at the same position', () => {
        const a = makeNode('a', 'signal', 0, 0, 0)
        const b = makeNode('b', 'signal', 0, 0, 0)

        const result = runForceSimulation([a, b], [], { iterations: 100 })

        const ra = result.find(n => n.id === 'a')!
        const rb = result.find(n => n.id === 'b')!
        expect(dist(ra, rb)).toBeGreaterThan(0.01)
    })

    it('pushes overlapping nodes further apart than they started', () => {
        const close = 0.1
        const a = makeNode('a', 'signal', -close / 2, 0, 0)
        const b = makeNode('b', 'signal', +close / 2, 0, 0)

        const initialDist = dist(a, b)
        const result = runForceSimulation([a, b], [], { iterations: 150 })

        const ra = result.find(n => n.id === 'a')!
        const rb = result.find(n => n.id === 'b')!
        expect(dist(ra, rb)).toBeGreaterThan(initialDist)
    })
})

// ---------------------------------------------------------------------------
// Suite: Spring attraction
// ---------------------------------------------------------------------------

describe('runForceSimulation — spring attraction', () => {
    it('pulls two connected nodes closer together when they start far apart', () => {
        const a = makeNode('a', 'signal', -20, 0, 0)
        const b = makeNode('b', 'signal', +20, 0, 0)
        const edge = makeEdge('a', 'b')

        const initialDist = dist(a, b)
        const result = runForceSimulation([a, b], [edge], { iterations: 200 })

        const ra = result.find(n => n.id === 'a')!
        const rb = result.find(n => n.id === 'b')!
        expect(dist(ra, rb)).toBeLessThan(initialDist)
    })
})

// ---------------------------------------------------------------------------
// Suite: Root pinning
// ---------------------------------------------------------------------------

describe('runForceSimulation — root pinning', () => {
    it('keeps the root node at (0,0,0) when pinRoot is true (default)', () => {
        const root = makeNode('root', 'root', 0, 0, 0)
        const child = makeNode('a', 'signal', 5, 0, 0)
        const result = runForceSimulation([root, child], [makeEdge('root', 'a')], {
            iterations: 100,
            pinRoot: true,
        })
        const r = result.find(n => n.id === 'root')!
        expect(r.px).toBeCloseTo(0)
        expect(r.py).toBeCloseTo(0)
        expect(r.pz).toBeCloseTo(0)
    })

    it('allows the root node to move when pinRoot is false', () => {
        const root = makeNode('root', 'root', 0, 0, 0)
        // Put a heavy cluster that will drag root away
        const others = Array.from({ length: 8 }, (_, i) =>
            makeNode(`n${i}`, 'signal', 50, 0, 0)
        )
        const edges = others.map(n => makeEdge('root', n.id))
        const result = runForceSimulation([root, ...others], edges, {
            iterations: 200,
            pinRoot: false,
        })
        const r = result.find(n => n.id === 'root')!
        // With pinRoot: false the root is no longer locked to origin
        const movedFromOrigin = Math.abs(r.px) + Math.abs(r.py) + Math.abs(r.pz)
        expect(movedFromOrigin).toBeGreaterThan(0.01)
    })
})

// ---------------------------------------------------------------------------
// Suite: Convergence
// ---------------------------------------------------------------------------

describe('runForceSimulation — convergence', () => {
    it('reaches a stable state (doubling iterations barely changes positions)', () => {
        const nodes = [
            makeNode('root', 'root', 0, 0, 0),
            makeNode('d1', 'dimension', 5, 0, 0),
            makeNode('d2', 'dimension', -5, 0, 0),
            makeNode('s1', 'signal', 7, 2, 0),
            makeNode('s2', 'signal', 3, -2, 0),
        ]
        const edges = [
            makeEdge('root', 'd1'),
            makeEdge('root', 'd2'),
            makeEdge('d1', 's1'),
            makeEdge('d1', 's2'),
        ]

        const short = runForceSimulation(nodes, edges, { iterations: 300 })
        const long = runForceSimulation(nodes, edges, { iterations: 600 })

        // Positions should be nearly identical (< 0.5 units difference per node)
        for (const ns of short) {
            const nl = long.find(n => n.id === ns.id)!
            expect(Math.abs(ns.px - nl.px)).toBeLessThan(0.5)
            expect(Math.abs(ns.py - nl.py)).toBeLessThan(0.5)
            expect(Math.abs(ns.pz - nl.pz)).toBeLessThan(0.5)
        }
    })
})
