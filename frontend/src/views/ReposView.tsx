import { useCallback, useEffect, useState } from 'react'
import GradeBadge from '../components/GradeBadge'
import OverviewPanel from '../components/OverviewPanel'
import RunProgressPanel from '../components/RunProgressPanel'
import AnalyzeModal from '../components/AnalyzeModal'
import RepoActionsModal from '../components/RepoActionsModal'
import type { ApiResponse, RepoSummary } from '../types'
import styles from './ReposView.module.css'

const TIER_LABELS: Record<number, string> = { 1: 'T1', 2: 'T2', 3: 'T3' }
const TIER_COLORS: Record<number, string> = {
    1: 'var(--text-muted)',
    2: '#4fa3e0',
    3: '#f0c040',
}

function TierBadge({ tier }: { tier: number }) {
    return (
        <span style={{
            fontSize: '0.7rem',
            fontWeight: 700,
            letterSpacing: '0.04em',
            color: TIER_COLORS[tier] ?? 'var(--text-muted)',
            border: `1px solid ${TIER_COLORS[tier] ?? 'var(--border)'}`,
            borderRadius: 4,
            padding: '1px 5px',
        }}>
            {TIER_LABELS[tier] ?? `T${tier}`}
        </span>
    )
}

interface ActiveRun {
    runId: string
    repoUrl: string
}

export default function ReposView() {
    const [repos, setRepos] = useState<RepoSummary[]>([])
    const [selected, setSelected] = useState<RepoSummary | null>(null)
    const [selectedScanId, setSelectedScanId] = useState<string | null>(null)
    const [loading, setLoading] = useState(true)
    const [analyzeOpen, setAnalyzeOpen] = useState(false)
    const [actionsRepo, setActionsRepo] = useState<RepoSummary | null>(null)
    const [activeRun, setActiveRun] = useState<ActiveRun | null>(null)

    const fetchRepos = useCallback(() => {
        return fetch('/repos')
            .then(r => r.json() as Promise<ApiResponse<RepoSummary[]>>)
            .then(res => { if (res.success) setRepos(res.data) })
            .finally(() => setLoading(false))
    }, [])

    useEffect(() => { fetchRepos() }, [fetchRepos])

    const handleStarted = (runId: string, repoUrl: string) => {
        setAnalyzeOpen(false)
        setSelected(null)
        setActiveRun({ runId, repoUrl })
        // Refresh the list — repo may not exist yet but will appear on completion
        fetchRepos()
    }

    const handleRunComplete = useCallback((repoUrl: string) => {
        setActiveRun(null)
        fetch('/repos')
            .then(r => r.json() as Promise<ApiResponse<RepoSummary[]>>)
            .then(res => {
                if (res.success) {
                    setRepos(res.data)
                    const match = res.data.find(r => r.url === repoUrl)
                    if (match) setSelected(match)
                }
            })
    }, [])

    const handleRunError = useCallback(() => {
        setActiveRun(null)
        fetchRepos()
    }, [fetchRepos])

    return (
        <div className={styles.root}>
            {/* List pane */}
            <div className={styles.listPane}>
                <div className={styles.paneHeader}>
                    <div>
                        <h1 className={styles.paneTitle}>Repositories</h1>
                        {!loading && (
                            <p className={styles.paneSubtitle}>
                                {repos.length} {repos.length === 1 ? 'repository' : 'repositories'} analyzed
                            </p>
                        )}
                    </div>
                    <button className={styles.analyzeBtn} onClick={() => setAnalyzeOpen(true)}>
                        <svg viewBox="0 0 16 16" fill="currentColor" width="13" height="13">
                            <path d="M10.68 11.74a6 6 0 0 1-7.922-8.982 6 6 0 0 1 8.982 7.922l3.04 3.04a.749.749 0 0 1-.326 1.275.749.749 0 0 1-.734-.215ZM11.5 7a4.499 4.499 0 1 0-8.997 0A4.499 4.499 0 0 0 11.5 7Z" />
                        </svg>
                        Analyze
                    </button>
                </div>

                <div className={styles.paneContent}>
                    {loading ? (
                        <div className={styles.emptyState}>
                            <span className={styles.emptyMsg}>Loading…</span>
                        </div>
                    ) : repos.length === 0 && !activeRun ? (
                        <div className={styles.emptyState}>
                            <p className={styles.emptyMsg}>No repositories scanned yet.</p>
                            <p className={styles.emptyHint}>Click Analyze to scan your first repository.</p>
                        </div>
                    ) : (
                        <table className={styles.table}>
                            <thead>
                                <tr>
                                    <th>Repository</th>
                                    <th>Grade</th>
                                    <th>Score</th>
                                    <th>Tier</th>
                                    <th>Scans</th>
                                    <th>Last Scanned</th>
                                    <th></th>
                                </tr>
                            </thead>
                            <tbody>
                                {activeRun && !repos.find(r => r.url === activeRun.repoUrl) && (
                                    <tr className={styles.row}>
                                        <td className={styles.urlCell} colSpan={5}>
                                            {activeRun.repoUrl.replace('https://github.com/', '')}
                                            <span style={{ color: 'var(--text-muted)', marginLeft: 8, fontSize: '0.75rem' }}>
                                                — analyzing…
                                            </span>
                                        </td>
                                        <td className={styles.actionsCell} />
                                    </tr>
                                )}
                                {repos.map(repo => (
                                    <tr
                                        key={repo.id}
                                        className={`${styles.row} ${selected?.id === repo.id && !activeRun ? styles.rowSelected : ''}`}
                                        onClick={() => { if (!activeRun) { setSelected(repo); setSelectedScanId(null) } }}
                                        data-testid="repo-row"
                                    >
                                        <td className={styles.urlCell} title={repo.url}>
                                            {repo.url.replace('https://github.com/', '')}
                                        </td>
                                        <td>
                                            {repo.latest_maturity_grade ? (
                                                <GradeBadge grade={repo.latest_maturity_grade} />
                                            ) : (
                                                <span className={styles.none}>—</span>
                                            )}
                                        </td>
                                        <td className={styles.scoreCell}>
                                            {repo.latest_composite_maturity ?? '—'}
                                        </td>
                                        <td>
                                            <TierBadge tier={repo.max_unlocked_tier ?? 1} />
                                        </td>
                                        <td className={styles.countCell}>{repo.scan_count}</td>
                                        <td className={styles.dateCell}>
                                            {repo.last_scanned ? repo.last_scanned.slice(0, 10) : '—'}
                                        </td>
                                        <td className={styles.actionsCell}>
                                            <button
                                                className={styles.actionsBtn}
                                                onClick={e => {
                                                    e.stopPropagation()
                                                    setActionsRepo(repo)
                                                }}
                                            >
                                                Actions
                                            </button>
                                        </td>
                                    </tr>
                                ))}
                            </tbody>
                        </table>
                    )}
                </div>
            </div>

            {/* Detail pane */}
            <div className={styles.detailPane}>
                {activeRun ? (
                    <RunProgressPanel
                        runId={activeRun.runId}
                        repoUrl={activeRun.repoUrl}
                        onComplete={handleRunComplete}
                        onError={handleRunError}
                    />
                ) : selected ? (
                    <OverviewPanel repoId={selected.id} scanId={selectedScanId} />
                ) : (
                    <div className={styles.detailPlaceholder}>
                        <svg viewBox="0 0 16 16" fill="currentColor" width="24" height="24" style={{ opacity: 0.3 }}>
                            <path d="M2 2.5A2.5 2.5 0 0 1 4.5 0h8.75a.75.75 0 0 1 .75.75v12.5a.75.75 0 0 1-.75.75h-2.5a.75.75 0 0 1 0-1.5h1.75v-2h-8a1 1 0 0 0-.714 1.7.75.75 0 1 1-1.072 1.05A2.495 2.495 0 0 1 2 11.5Zm10.5-1h-8a1 1 0 0 0-1 1v6.708A2.486 2.486 0 0 1 4.5 9h8Z" />
                        </svg>
                        <p>Select a repository<br />to view its overview</p>
                    </div>
                )}
            </div>

            {/* Modals */}
            {analyzeOpen && (
                <AnalyzeModal
                    onClose={() => setAnalyzeOpen(false)}
                    onStarted={handleStarted}
                />
            )}
            {actionsRepo && (
                <RepoActionsModal
                    repo={actionsRepo}
                    onClose={() => setActionsRepo(null)}
                    onStarted={(runId, repoUrl) => {
                        setActionsRepo(null)
                        handleStarted(runId, repoUrl)
                    }}
                    onSelectScan={(scanId) => {
                        setSelected(actionsRepo)
                        setSelectedScanId(scanId)
                        setActionsRepo(null)
                    }}
                />
            )}
        </div>
    )
}
