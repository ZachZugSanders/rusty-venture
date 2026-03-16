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
