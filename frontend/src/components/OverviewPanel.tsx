import { useEffect, useState } from 'react'
import GradeBadge from './GradeBadge'
import type { BlockerItem, DimensionOverview, RepoOverview, TrendPoint } from '../types'
import styles from './OverviewPanel.module.css'

interface OverviewPanelProps {
    repoId: string
}

export default function OverviewPanel({ repoId }: OverviewPanelProps) {
    const [overview, setOverview] = useState<RepoOverview | null>(null)
    const [trends, setTrends] = useState<TrendPoint[]>([])
    const [loading, setLoading] = useState(true)
    const [error, setError] = useState<string | null>(null)

    useEffect(() => {
        setLoading(true)
        setError(null)

        Promise.all([
            fetch(`/repos/${repoId}/overview`).then(r => r.json()),
            fetch(`/repos/${repoId}/trends?window=10`).then(r => r.json()),
        ])
            .then(([ovRes, trRes]) => {
                if (ovRes.success) setOverview(ovRes.data)
                else setError(ovRes.error ?? 'No overview available')
                if (trRes.success) setTrends(trRes.data)
            })
            .catch(e => setError(String(e)))
            .finally(() => setLoading(false))
    }, [repoId])

    if (loading) return <div className={styles.loading}>Loading overview…</div>
    if (error || !overview) return <div className={styles.empty}>{error ?? 'No overview available'}</div>

    return (
        <div className={styles.panel} data-testid="overview-panel">
            {/* Posture card */}
            <section className={styles.card}>
                <h3 className={styles.cardTitle}>Posture</h3>
                <div className={styles.postureRow}>
                    <GradeBadge grade={overview.grade} data-testid="overview-grade" />
                    <span className={styles.composite} data-testid="overview-composite">
                        {overview.composite}
                    </span>
                </div>
                {overview.dimensions.length > 0 && (
                    <ul className={styles.dimList}>
                        {overview.dimensions.map((d: DimensionOverview) => (
                            <li key={d.dimension} className={styles.dimItem}>
                                <span className={styles.dimName}>{d.dimension}</span>
                                <span className={styles.dimScore}>{d.score}</span>
                                <span className={styles.dimPassed}>
                                    ({d.passed_count}/{d.total_count})
                                </span>
                            </li>
                        ))}
                    </ul>
                )}
            </section>

            {/* Confidence card */}
            <section className={styles.card}>
                <h3 className={styles.cardTitle}>Confidence</h3>
                <div className={styles.confidenceValue} data-testid="overview-confidence">
                    {overview.confidence.toFixed(1)}%
                </div>
                <p className={styles.confidenceLabel}>of signals passed</p>
            </section>

            {/* Top blockers card */}
            <section className={styles.card}>
                <h3 className={styles.cardTitle}>Top Blockers</h3>
                {overview.top_blockers.length === 0 ? (
                    <p className={styles.empty}>No blockers — all signals passed!</p>
                ) : (
                    <ul className={styles.blockerList}>
                        {overview.top_blockers.map((b: BlockerItem) => (
                            <li key={b.signal_name} className={styles.blockerItem} data-testid="blocker-item">
                                <span className={styles.blockerName}>{b.signal_name}</span>
                                <span className={styles.blockerPoints}>−{b.points}pts</span>
                                {b.detail && (
                                    <p className={styles.blockerDetail}>{b.detail}</p>
                                )}
                            </li>
                        ))}
                    </ul>
                )}
            </section>

            {/* Trend card */}
            <section className={styles.card}>
                <h3 className={styles.cardTitle}>Trend</h3>
                {trends.length === 0 ? (
                    <p className={styles.empty}>No trend data yet</p>
                ) : (
                    <ul className={styles.trendList}>
                        {trends.map((pt: TrendPoint) => (
                            <li key={pt.scanned_at} className={styles.trendPoint} data-testid="trend-point">
                                <span className={styles.trendDate}>{pt.scanned_at.slice(0, 10)}</span>
                                <GradeBadge grade={pt.grade} />
                                <span className={styles.trendComposite}>{pt.composite}</span>
                            </li>
                        ))}
                    </ul>
                )}
            </section>
        </div>
    )
}
