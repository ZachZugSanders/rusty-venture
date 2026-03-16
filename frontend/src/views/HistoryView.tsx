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

    if (loading) return <p className={styles.empty}>Loading…</p>

    return (
        <div className={styles.root}>
            {scans.length === 0 ? (
                <p className={styles.empty}>No scans yet.</p>
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
                                <td>{scan.composite_maturity}</td>
                                <td>{scan.risk_score}</td>
                                <td>{(scan.duration_ms / 1000).toFixed(1)}s</td>
                                <td className={styles.dateCell}>{scan.scanned_at.slice(0, 19).replace('T', ' ')}</td>
                            </tr>
                        ))}
                    </tbody>
                </table>
            )}
        </div>
    )
}
