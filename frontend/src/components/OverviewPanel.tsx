import { useEffect, useState } from 'react'
import GradeBadge from './GradeBadge'
import type { BlockerItem, DimensionOverview, DimensionSignals, RepoOverview, SignalItem, TrendPoint } from '../types'
import styles from './OverviewPanel.module.css'

interface OverviewPanelProps {
    repoId: string
}

function LlmSection({ title, items, accent }: { title: string; items: string[]; accent?: boolean }) {
    if (items.length === 0) return null
    return (
        <section className={styles.card}>
            <h3 className={styles.cardTitle}>{title}</h3>
            <ul className={`${styles.llmList} ${accent ? styles.llmListAccent : ''}`}>
                {items.map((item, i) => (
                    <li key={i} className={styles.llmItem}>{item}</li>
                ))}
            </ul>
        </section>
    )
}

function SignalRow({ signal }: { signal: SignalItem }) {
    return (
        <li className={`${styles.signalItem} ${signal.passed ? styles.signalPassed : styles.signalFailed}`}>
            <span className={styles.signalIcon}>{signal.passed ? '✓' : '✗'}</span>
            <span className={styles.signalName}>{signal.name}</span>
            <span className={styles.signalPoints}>
                {signal.passed ? `+${signal.points}` : `−${signal.points}`}pts
            </span>
            {!signal.passed && signal.detail && (
                <p className={styles.signalDetail}>{signal.detail}</p>
            )}
        </li>
    )
}

function DimensionCard({ dim }: { dim: DimensionSignals }) {
    const [expanded, setExpanded] = useState(false)
    const failed = dim.signals.filter(s => !s.passed)
    const passed = dim.signals.filter(s => s.passed)
    return (
        <div className={styles.dimCard}>
            <button className={styles.dimCardHeader} onClick={() => setExpanded(e => !e)}>
                <span className={styles.dimCardName}>{dim.dimension}</span>
                <span className={styles.dimCardScore}>{dim.score}</span>
                <span className={styles.dimCardCounts}>
                    <span className={styles.passedCount}>{passed.length} passed</span>
                    {failed.length > 0 && <span className={styles.failedCount}>{failed.length} failed</span>}
                </span>
                <span className={styles.dimCardChevron}>{expanded ? '▲' : '▼'}</span>
            </button>
            {expanded && (
                <ul className={styles.signalList}>
                    {dim.signals.map(s => <SignalRow key={s.name} signal={s} />)}
                </ul>
            )}
        </div>
    )
}

const TIER_DEFS = [
    { num: 1, label: 'Static Discovery', desc: 'File presence · 25 signals' },
    { num: 2, label: 'Content Quality',  desc: 'File content inspection' },
    { num: 3, label: 'Active Validation', desc: 'Build · test · lint' },
]

function TierTrack({ scanTier, maxUnlockedTier }: { scanTier: number; maxUnlockedTier: number }) {
    return (
        <div style={{ display: 'flex', gap: 0, alignItems: 'stretch' }}>
            {TIER_DEFS.map((t, i) => {
                const done    = t.num < maxUnlockedTier
                const current = t.num === scanTier
                const locked  = t.num > maxUnlockedTier

                let bg     = 'var(--surface-2, #1e1e2e)'
                let border = 'var(--border)'
                let color  = 'var(--text-muted)'
                if (done)    { bg = 'rgba(80,180,100,0.08)'; border = '#50b464'; color = '#50b464' }
                if (current) { bg = 'rgba(79,163,224,0.1)';  border = '#4fa3e0'; color = '#e8e8f0' }

                return (
                    <div key={t.num} style={{ display: 'flex', alignItems: 'center', gap: 0 }}>
                        {i > 0 && (
                            <div style={{ width: 16, height: 1, background: done ? '#50b464' : 'var(--border)', flexShrink: 0 }} />
                        )}
                        <div style={{
                            border: `1px solid ${border}`,
                            background: bg,
                            borderRadius: 6,
                            padding: '6px 10px',
                            minWidth: 110,
                        }}>
                            <div style={{ display: 'flex', alignItems: 'center', gap: 6, marginBottom: 2 }}>
                                <span style={{ fontSize: '0.65rem', fontWeight: 700, color: done ? '#50b464' : current ? '#4fa3e0' : 'var(--text-muted)' }}>
                                    T{t.num}
                                </span>
                                <span style={{ fontSize: '0.7rem', fontWeight: 600, color }}>
                                    {t.label}
                                </span>
                                <span style={{ marginLeft: 'auto', fontSize: '0.8rem' }}>
                                    {done ? '✓' : locked ? '🔒' : '▶'}
                                </span>
                            </div>
                            <div style={{ fontSize: '0.65rem', color: 'var(--text-muted)' }}>{t.desc}</div>
                        </div>
                    </div>
                )
            })}
        </div>
    )
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

    const llm = overview.llm_report

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

            {/* Tier progression track */}
            <section className={styles.card}>
                <h3 className={styles.cardTitle}>Scan Tier Progression</h3>
                <TierTrack
                    scanTier={overview.scan_tier ?? 1}
                    maxUnlockedTier={overview.max_unlocked_tier ?? 1}
                />
            </section>

            {/* Confidence card */}
            <section className={styles.card}>
                <h3 className={styles.cardTitle}>Confidence</h3>
                <div className={styles.confidenceValue} data-testid="overview-confidence">
                    {overview.confidence.toFixed(1)}%
                </div>
                <p className={styles.confidenceLabel}>of signals passed</p>
            </section>

            {/* LLM Summary */}
            {llm && llm.summary && (
                <section className={styles.card}>
                    <h3 className={styles.cardTitle}>Summary</h3>
                    <p className={styles.llmSummary}>{llm.summary}</p>
                </section>
            )}

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

            {/* Signals by dimension */}
            {overview.signals_by_dimension.length > 0 && (
                <section className={styles.card}>
                    <h3 className={styles.cardTitle}>Signals by Dimension</h3>
                    <div className={styles.dimCards}>
                        {overview.signals_by_dimension.map((dim: DimensionSignals) => (
                            <DimensionCard key={dim.dimension} dim={dim} />
                        ))}
                    </div>
                </section>
            )}

            {/* LLM recommendation sections */}
            {llm && (
                <>
                    <LlmSection title="General Recommendations" items={llm.general_recommendations} />
                    <LlmSection title="Language Insights" items={llm.language_insights} />
                    <LlmSection title="Dependency Recommendations" items={llm.dependency_recommendations} />
                    {llm.dockerfile_findings.length > 0 && (
                        <LlmSection title="Dockerfile Findings" items={llm.dockerfile_findings} />
                    )}
                    {llm.security_violations.length > 0 && (
                        <LlmSection title="Security Violations" items={llm.security_violations} accent />
                    )}
                </>
            )}

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
