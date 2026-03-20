import { useEffect, useState } from 'react'
import GradeBadge from '../components/GradeBadge'
import OverviewPanel from '../components/OverviewPanel'
import type { ApiResponse, RepoSummary } from '../types'
import styles from './ReposView.module.css'

export default function ReposView() {
    const [repos, setRepos] = useState<RepoSummary[]>([])
    const [selected, setSelected] = useState<RepoSummary | null>(null)
    const [loading, setLoading] = useState(true)

    useEffect(() => {
        fetch('/repos')
            .then(r => r.json() as Promise<ApiResponse<RepoSummary[]>>)
            .then(res => { if (res.success) setRepos(res.data) })
            .finally(() => setLoading(false))
    }, [])

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
                </div>

                <div className={styles.paneContent}>
                    {loading ? (
                        <div className={styles.emptyState}>
                            <span className={styles.emptyMsg}>Loading…</span>
                        </div>
                    ) : repos.length === 0 ? (
                        <div className={styles.emptyState}>
                            <p className={styles.emptyMsg}>No repositories scanned yet.</p>
                            <p className={styles.emptyHint}>Use the Analyze tab to scan your first repository.</p>
                        </div>
                    ) : (
                        <table className={styles.table}>
                            <thead>
                                <tr>
                                    <th>Repository</th>
                                    <th>Grade</th>
                                    <th>Score</th>
                                    <th>Scans</th>
                                    <th>Last Scanned</th>
                                </tr>
                            </thead>
                            <tbody>
                                {repos.map(repo => (
                                    <tr
                                        key={repo.id}
                                        className={`${styles.row} ${selected?.id === repo.id ? styles.rowSelected : ''}`}
                                        onClick={() => setSelected(repo)}
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
                                        <td className={styles.countCell}>{repo.scan_count}</td>
                                        <td className={styles.dateCell}>
                                            {repo.last_scanned ? repo.last_scanned.slice(0, 10) : '—'}
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
                {selected ? (
                    <OverviewPanel repoId={selected.id} />
                ) : (
                    <div className={styles.detailPlaceholder}>
                        <svg viewBox="0 0 16 16" fill="currentColor" width="24" height="24" style={{ opacity: 0.3 }}>
                            <path d="M2 2.5A2.5 2.5 0 0 1 4.5 0h8.75a.75.75 0 0 1 .75.75v12.5a.75.75 0 0 1-.75.75h-2.5a.75.75 0 0 1 0-1.5h1.75v-2h-8a1 1 0 0 0-.714 1.7.75.75 0 1 1-1.072 1.05A2.495 2.495 0 0 1 2 11.5Zm10.5-1h-8a1 1 0 0 0-1 1v6.708A2.486 2.486 0 0 1 4.5 9h8Z" />
                        </svg>
                        <p>Select a repository<br />to view its overview</p>
                    </div>
                )}
            </div>
        </div>
    )
}
