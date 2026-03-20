import { useState } from 'react'
import GradeBadge from '../components/GradeBadge'
import styles from './AnalyzeView.module.css'

// ── Types ────────────────────────────────────────────────────────────────────
interface ScanSignal {
    name: string
    description: string
    passed: boolean
    points: number
    detail: string | null
}

interface ScanDimension {
    dimension: string
    score: number
    weight: number
    signals: ScanSignal[]
}

interface AnalyzeResult {
    report: {
        summary: string
        risk_score: number
        language_insights: string[]
        dependency_recommendations: string[]
        dockerfile_findings: string[]
        security_violations: string[]
        general_recommendations: string[]
    }
    maturity: {
        composite: number
        grade: string
        dimensions: ScanDimension[]
    }
    duration_ms: number
}

// ── Helpers ──────────────────────────────────────────────────────────────
function scoreColor(score: number): string {
    if (score >= 70) return 'var(--green)'
    if (score >= 40) return 'var(--yellow)'
    return 'var(--red)'
}

function fmtDim(raw: string): string {
    return raw.replace(/_/g, ' ').replace(/\b\w/g, c => c.toUpperCase())
}

// ── DimensionCard ────────────────────────────────────────────────────────────
function DimensionCard({ dim }: { dim: ScanDimension }) {
    const [open, setOpen] = useState(false)
    const passCount = dim.signals.filter(s => s.passed).length

    return (
        <div className={styles.dimCard}>
            <button className={styles.dimHeader} onClick={() => setOpen(x => !x)}>
                <span className={styles.dimName}>{fmtDim(dim.dimension)}</span>
                <div className={styles.dimMeta}>
                    <span className={styles.dimPassedCount}>{passCount}/{dim.signals.length} passed</span>
                    <span className={styles.dimScore} style={{ color: scoreColor(dim.score) }}>
                        {dim.score}
                    </span>
                    <span className={styles.dimChevron}>{open ? '\u25be' : '\u25b8'}</span>
                </div>
            </button>
            <div className={styles.dimBar}>
                <div
                    className={styles.dimBarFill}
                    style={{ width: `${dim.score}%`, background: scoreColor(dim.score) }}
                />
            </div>
            {open && (
                <ul className={styles.signalList}>
                    {dim.signals.map(sig => (
                        <li
                            key={sig.name}
                            className={`${styles.signalItem} ${sig.passed ? styles.signalPass : styles.signalFail}`}
                        >
                            <span className={styles.signalIcon}>{sig.passed ? '\u2713' : '\u2717'}</span>
                            <div className={styles.signalBody}>
                                <span className={styles.signalName}>{sig.description}</span>
                                {sig.detail && (
                                    <span className={styles.signalDetail}>{sig.detail}</span>
                                )}
                            </div>
                            {!sig.passed && sig.points > 0 && (
                                <span className={styles.signalPoints}>\u2212{sig.points}pts</span>
                            )}
                        </li>
                    ))}
                </ul>
            )}
        </div>
    )
}

// ── ResultView ─────────────────────────────────────────────────────────────
function ResultView({ result, repoUrl }: { result: AnalyzeResult; repoUrl: string }) {
    const { report: r, maturity: m, duration_ms } = result
    const displayUrl = repoUrl.replace(/^https?:\/\/(www\.)?github\.com\//, '')
    const allRecs = [
        ...r.general_recommendations,
        ...r.dependency_recommendations,
        ...r.dockerfile_findings,
    ].filter(Boolean)

    return (
        <div className={styles.resultView}>
            {/* Header card */}
            <div className={styles.resultHeader}>
                <div className={styles.resultMeta}>
                    <span className={styles.resultUrl}>{displayUrl}</span>
                    <span className={styles.resultDuration}>⏱ {(duration_ms / 1000).toFixed(1)}s</span>
                </div>
                <div className={styles.resultScores}>
                    <GradeBadge grade={m.grade} />
                    <span className={styles.resultComposite} style={{ color: scoreColor(m.composite) }}>
                        {m.composite}
                    </span>
                    <span className={styles.resultCompositeLabel}>/100</span>
                </div>
            </div>

            {/* Security violations banner */}
            {r.security_violations.length > 0 && (
                <div className={styles.riskBanner}>
                    <strong>⚠ {r.security_violations.length} security violation{r.security_violations.length !== 1 ? 's' : ''}</strong>
                    <ul className={styles.violationList}>
                        {r.security_violations.map((v, i) => <li key={i}>{v}</li>)}
                    </ul>
                </div>
            )}

            {/* LLM Summary */}
            {r.summary && (
                <div className={styles.summaryCard}>
                    <p className={styles.summaryText}>{r.summary}</p>
                </div>
            )}

            {/* Dimensions */}
            {m.dimensions.length > 0 && (
                <>
                    <h3 className={styles.sectionTitle}>Maturity Dimensions</h3>
                    <div className={styles.dimGrid}>
                        {m.dimensions.map(dim => (
                            <DimensionCard key={dim.dimension} dim={dim} />
                        ))}
                    </div>
                </>
            )}

            {/* Recommendations */}
            {allRecs.length > 0 && (
                <>
                    <h3 className={styles.sectionTitle}>Recommendations</h3>
                    <ul className={styles.recList}>
                        {allRecs.map((rec, i) => (
                            <li key={i} className={styles.recItem}>{rec}</li>
                        ))}
                    </ul>
                </>
            )}

            {/* Language insights */}
            {r.language_insights.length > 0 && (
                <>
                    <h3 className={styles.sectionTitle}>Language Insights</h3>
                    <ul className={styles.recList}>
                        {r.language_insights.map((ins, i) => (
                            <li key={i} className={styles.recItem}>{ins}</li>
                        ))}
                    </ul>
                </>
            )}
        </div>
    )
}

// ── Main view ─────────────────────────────────────────────────────────────
export default function AnalyzeView() {
    const [repoUrl, setRepoUrl] = useState('')
    const [branch, setBranch] = useState('')
    const [noContainer, setNoContainer] = useState(false)
    const [status, setStatus] = useState<'idle' | 'loading' | 'done' | 'error'>('idle')
    const [result, setResult] = useState<AnalyzeResult | null>(null)
    const [error, setError] = useState<string | null>(null)

    const handleSubmit = async (e: React.FormEvent) => {
        e.preventDefault()
        setStatus('loading')
        setError(null)
        try {
            const res = await fetch('/analyze', {
                method: 'POST',
                headers: { 'Content-Type': 'application/json' },
                body: JSON.stringify({
                    repo_url: repoUrl,
                    branch: branch || undefined,
                    no_container: noContainer,
                }),
            })
            // Guard against empty or non-JSON bodies (e.g. proxy 502 when
            // the server is down, or a connection drop mid-response).
            const text = await res.text()
            if (!text) {
                setError(
                    res.ok
                        ? 'Server returned an empty response.'
                        : `Server error ${res.status} — is rusty-venture-server running?`,
                )
                setStatus('error')
                return
            }
            let json: { success: boolean; data?: AnalyzeResult; error?: string }
            try {
                json = JSON.parse(text)
            } catch {
                setError(`Unexpected server response (HTTP ${res.status}): ${text.slice(0, 200)}`)
                setStatus('error')
                return
            }
            if (json.success) {
                setResult(json.data as AnalyzeResult)
                setStatus('done')
            } else {
                setError(json.error ?? 'Analysis failed')
                setStatus('error')
            }
        } catch (err) {
            setError(`Network error — is rusty-venture-server running? (${String(err)})`)
            setStatus('error')
        }
    }

    return (
        <div className={styles.root}>
            <div className={styles.pageHeader}>
                <div>
                    <h1 className={styles.pageTitle}>Analyze</h1>
                    <p className={styles.pageSubtitle}>Run a full maturity scan on a git repository</p>
                </div>
            </div>

            <div className={styles.pageContent}>
                <div className={styles.formWrap}>
                    <form onSubmit={handleSubmit} className={styles.form}>
                        <label className={styles.fieldGroup}>
                            <span className={styles.fieldLabel}>Repository URL</span>
                            <input
                                type="url"
                                value={repoUrl}
                                onChange={e => setRepoUrl(e.target.value)}
                                placeholder="https://github.com/owner/repo"
                                required
                                className={styles.input}
                                disabled={status === 'loading'}
                            />
                        </label>
                        <label className={styles.fieldGroup}>
                            <span className={styles.fieldLabel}>
                                Branch <span className={styles.optional}>(optional)</span>
                            </span>
                            <input
                                type="text"
                                value={branch}
                                onChange={e => setBranch(e.target.value)}
                                placeholder="main"
                                className={styles.input}
                                disabled={status === 'loading'}
                            />
                        </label>
                        <label className={styles.fieldGroup} style={{ flexDirection: 'row', alignItems: 'center', gap: '0.5rem' }}>
                            <input
                                type="checkbox"
                                id="no-container"
                                checked={noContainer}
                                onChange={e => setNoContainer(e.target.checked)}
                                disabled={status === 'loading'}
                            />
                            <span className={styles.fieldLabel} style={{ margin: 0 }}>
                                Skip Docker (no-container mode)
                            </span>
                        </label>
                        <button
                            type="submit"
                            disabled={status === 'loading'}
                            className={styles.submitBtn}
                        >
                            {status === 'loading' ? (
                                <><span className={styles.spinner} />Analyzing…</>
                            ) : 'Run Analysis'}
                        </button>
                    </form>

                    {status === 'loading' && (
                        <div className={styles.loadingCard}>
                            <div className={styles.loadingDot} />
                            <div>
                                <p className={styles.loadingTitle}>Analysis in progress</p>
                                <p className={styles.loadingSubtitle}>
                                    Cloning, scanning, and scoring the repository — this may take 1–3 minutes.
                                </p>
                            </div>
                        </div>
                    )}

                    {status === 'error' && error && (
                        <div className={styles.errorCard}>
                            <strong>Analysis failed</strong>
                            <p>{error}</p>
                        </div>
                    )}
                </div>

                {status === 'done' && result && (
                    <ResultView result={result} repoUrl={repoUrl} />
                )}
            </div>
        </div>
    )
}
