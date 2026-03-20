export interface RepoSummary {
    id: string
    url: string
    first_seen: string
    last_scanned: string | null
    scan_count: number
    latest_maturity_grade: string | null
    latest_composite_maturity: number | null
    latest_risk_score: number | null
}

export interface DimensionOverview {
    dimension: string
    score: number
    weight: number
    passed_count: number
    total_count: number
}

export interface BlockerItem {
    signal_name: string
    dimension: string
    points: number
    detail: string | null
}

export interface RepoOverview {
    repo_id: string
    repo_url: string
    composite: number
    grade: string
    confidence: number
    dimensions: DimensionOverview[]
    top_blockers: BlockerItem[]
}

export interface TrendPoint {
    scanned_at: string
    composite: number
    grade: string
    dimensions: Record<string, number>
}

export interface ScanSummary {
    id: string
    repo_url: string
    scanned_at: string
    duration_ms: number
    risk_score: number
    composite_maturity: number
    maturity_grade: string
}

export interface ApiResponse<T> {
    success: boolean
    data: T
}

// ── Decision Graph ───────────────────────────────────────────────────────────

/**
 * Static configuration controlling the 3D radial layout and visual sphere
 * sizes. Mirrored from Rust `NodeSizeConfig` in `rusty-venture-actions`.
 */
export interface NodeSizeConfig {
    /** Root → dimension orbital radius. */
    orbit_l1: number
    /** Dimension → signal orbital radius. */
    orbit_l2: number
    /** Base sphere radius for the root node. */
    root_base_radius: number
    /** Base sphere radius for dimension nodes. */
    dim_base_radius: number
    /** Base sphere radius for signal nodes. */
    sig_base_radius: number
    /** Score-driven size multiplier for root and dimension radii. */
    score_scale: number
    /** Points-driven size multiplier for signal radii. */
    sig_points_scale: number
    /** Signal-count boost for dimension radius (more children = larger hub). */
    dim_signal_scale: number
    /** Extra radius boost for signal nodes that carry an actionable detail string. */
    sig_detail_boost: number
}

export interface GraphNode {
    id: string
    label: string
    kind: 'root' | 'dimension' | 'signal'
    // Pre-computed 3D world-space position (hierarchical radial layout)
    px: number
    py: number
    pz: number
    /** Pre-computed visual sphere radius from NodeSizeConfig. */
    radius: number
    // Semantic metadata
    /** Earned value: composite / dim-score / signal points (0 if failed). */
    score: number
    /** Dimension weight 0–1; 1.0 for root. */
    weight: number
    /** Maximum points this signal can contribute; 0 for root/dim nodes. */
    max_points: number
    /** Number of direct signal children; 0 for root/signal nodes. */
    signal_count: number
    /** Whether this signal node carries an actionable detail string. */
    has_detail: boolean
    passed: boolean
    highlight: boolean
}

export interface GraphEdge {
    from: string
    to: string
    /** Visual weight for edge rendering; root→dim = dim weight, dim→sig = half dim weight. */
    weight: number
}

export interface DecisionGraph {
    nodes: GraphNode[]
    edges: GraphEdge[]
    config: NodeSizeConfig
}

