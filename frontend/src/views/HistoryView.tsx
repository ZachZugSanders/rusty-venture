import { useEffect, useState } from 'react'
import GradeBadge from '../components/GradeBadge'
import type { ApiResponse, ScanSummary } from '../types'
import styles from './HistoryView.module.css'

export default function HistoryView() {
    const [scans, setScans] = useState<ScanSummary[]>([])
    const [loading, setLoading] = useState(true)

    useEffect(() => {
        fetch('/scans?limit=50')
            .then(r => r.json() as Promise<ApiResponse<ScanSummary[]>>)
            .then(res => { if (res.success) setScans(res.data) })
            .finally(() => setLoading(false))
    }, [])

    return (
        <div className={styles.root}>
            <div className={styles.pageHeader}>
                <div>
                    <h1 className={styles.pageTitle}>Scan History</h1>
                    {!loading && (
                        <p className={styles.pageSubtitle}>
                            {scans.length} scan{scans.length !== 1 ? 's' : ''} recorded
                        </p>
                    )}
                </div>
            </div>

            <div className={styles.tableWrap}>
                {loading ? (
                    <div className={styles.emptyState}>
                        <span className={styles.emptyMsg}>Loading…</span>
                    </div>
                ) : scans.length === 0 ? (
                    <div className={styles.emptyState}>
                        <p className={styles.emptyMsg}>No scans recorded yet.</p>
                        <p className={styles.emptyHint}>Scans appear here after running an analysis.</p>
                    </div>
                ) : (
                    <table className={styles.table}>
                        <thead>
                            <tr>
                                <th>Repository</th>
                                <th>Grade</th>
                                <th>Score</th>
                                <th>Risk</th>
                                <th>Duration</th>
                                <th>Scanned At</th>
                            </tr>
                        </thead>
                        <tbody>
                            {scans.map(scan => (
                                <tr key={scan.id} className={styles.row}>
                                    <td className={styles.urlCell} title={scan.repo_url}>
                                        {scan.repo_url.replace('https://github.com/', '')}
                                    </td>
                                    <td>
                                        <GradeBadge grade={scan.maturity_grade} />
                                    </td>
                                    <td className={styles.scoreCell}>{scan.composite_maturity}</td>
                                    <td className={styles.riskCell}>{scan.risk_score}</td>
                                    <td className={styles.durationCell}>{(scan.duration_ms / 1000).toFixed(1)}s</td>
                                    <td className={styles.dateCell}>
                                        {scan.scanned_at.slice(0, 16).replace('T', ' ')}
                                    </td>
                                </tr>
                            ))}
                        </tbody>
                    </table>
                )}
            </div>
        </div>
    )
}
