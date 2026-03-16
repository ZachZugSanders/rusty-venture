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
            <div className={styles.tablePane}>
                {loading ? (
                    <p className={styles.empty}>Loading…</p>
                ) : repos.length === 0 ? (
                    <p className={styles.empty}>No repositories scanned yet.</p>
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
                                    <td>{repo.latest_composite_maturity ?? '—'}</td>
                                    <td>{repo.scan_count}</td>
                                    <td className={styles.dateCell}>
                                        {repo.last_scanned ? repo.last_scanned.slice(0, 10) : '—'}
                                    </td>
                                </tr>
                            ))}
                        </tbody>
                    </table>
                )}
            </div>

            <div className={`${styles.detailPane} ${selected ? styles.detailPaneOpen : ''}`}>
                {selected ? (
                    <OverviewPanel repoId={selected.id} />
                ) : (
                    <div className={styles.detailPlaceholder}>
                        Select a repository to see its overview
                    </div>
                )}
            </div>
        </div>
    )
}
