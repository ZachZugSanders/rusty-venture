import { useEffect, useState } from 'react'
import GradeBadge from './GradeBadge'
import Modal from './Modal'
import type { ApiResponse, RepoSummary, ScanSummary } from '../types'
import styles from './RepoActionsModal.module.css'

interface Props {
    repo: RepoSummary
    onClose: () => void
    onStarted: (runId: string, repoUrl: string) => void
    onSelectScan: (scanId: string) => void
}

type RescanStatus = 'idle' | 'checking' | 'upToDate' | 'error'

const TIER_NAMES: Record<number, string> = {
    1: 'Static Discovery',
    2: 'Content Quality',
    3: 'Active Validation',
}

export default function RepoActionsModal({ repo, onClose, onStarted, onSelectScan }: Props) {
    const [scans, setScans] = useState<ScanSummary[]>([])
    const [loading, setLoading] = useState(true)
    const [rescanStatus, setRescanStatus] = useState<RescanStatus>('idle')
    const [commitSha, setCommitSha] = useState<string | null>(null)
    const [errorMsg, setErrorMsg] = useState<string | null>(null)

    const displayUrl = repo.url.replace('https://github.com/', '')

    useEffect(() => {
        fetch('/scans?limit=50')
            .then(r => r.json() as Promise<ApiResponse<ScanSummary[]>>)
            .then(res => {
                if (res.success) setScans(res.data.filter(s => s.repo_url === repo.url))
            })
            .finally(() => setLoading(false))
    }, [repo.url])

    const maxTier = repo.max_unlocked_tier ?? 1
    const nextTier = maxTier < 3 ? maxTier + 1 : null

    const handleRescan = async () => {
        setRescanStatus('checking')
        setErrorMsg(null)
        try {
            const res = await fetch(`/repos/${repo.id}/rescan`, { method: 'POST' })
            const json = await res.json()
            if (!json.success) {
                setErrorMsg(json.error ?? `Server error ${res.status}`)
                setRescanStatus('error')
                return
            }
            const data = json.data
            setCommitSha(data.commit_sha ?? null)
            if (data.skipped) {
                setRescanStatus('upToDate')
            } else {
                onStarted(data.run_id, repo.url)
                onClose()
            }
        } catch (err) {
            setErrorMsg(String(err))
            setRescanStatus('error')
        }
    }

    const handleScanNextTier = async () => {
        if (!nextTier) return
        setRescanStatus('checking')
        setErrorMsg(null)
        try {
            const res = await fetch('/analyze', {
                method: 'POST',
                headers: { 'Content-Type': 'application/json' },
                body: JSON.stringify({ repo_url: repo.url, tier: nextTier }),
            })
            const json = await res.json()
            if (!json.success) {
                setErrorMsg(json.error ?? `Server error ${res.status}`)
                setRescanStatus('error')
                return
            }
            onStarted(json.data.run_id, repo.url)
            onClose()
        } catch (err) {
            setErrorMsg(String(err))
            setRescanStatus('error')
        }
    }

    const footer = (
        <>
            {/* Status messages sit left of the buttons */}
            <div className={styles.footerStatus}>
                {rescanStatus === 'checking' && (
                    <span className={styles.rescanChecking}>
                        <span className={styles.rescanSpinner} />
                        Fetching latest commit…
                    </span>
                )}
                {rescanStatus === 'upToDate' && (
                    <span className={styles.footerUpToDate}>
                        Already up to date
                        {commitSha && <> — <code>{commitSha.slice(0, 7)}</code></>}
                    </span>
                )}
                {rescanStatus === 'error' && (
                    <span className={styles.footerError}>{errorMsg}</span>
                )}
            </div>

            <div className={styles.footerActions}>
                {nextTier && (
                    <button
                        className={styles.rescanBtn}
                        onClick={handleScanNextTier}
                        disabled={rescanStatus === 'checking'}
                        title={`Run Tier ${nextTier}: ${TIER_NAMES[nextTier]}`}
                        style={{ background: 'rgba(79,163,224,0.12)', borderColor: '#4fa3e0', color: '#4fa3e0' }}
                    >
                        Scan T{nextTier} →
                    </button>
                )}
                <button
                    className={styles.rescanBtn}
                    onClick={rescanStatus === 'upToDate' || rescanStatus === 'error'
                        ? () => setRescanStatus('idle')
                        : handleRescan}
                    disabled={rescanStatus === 'checking'}
                >
                    {rescanStatus === 'upToDate' ? 'Check again'
                        : rescanStatus === 'error' ? 'Try again'
                        : 'Rescan T1'}
                </button>
                <button className={styles.cancelBtn} onClick={onClose}>Cancel</button>
            </div>
        </>
    )

    return (
        <Modal title={displayUrl} onClose={onClose} size="lg" footer={footer}>
            {loading ? (
                <div className={styles.emptyState}><span className={styles.emptyMsg}>Loading…</span></div>
            ) : scans.length === 0 ? (
                <div className={styles.emptyState}>
                    <p className={styles.emptyMsg}>No scans recorded for this repository.</p>
                    <p className={styles.emptyHint}>Click Rescan to run a new analysis.</p>
                </div>
            ) : (
                <table className={styles.table}>
                    <thead>
                        <tr>
                            <th>Grade</th>
                            <th>Score</th>
                            <th>Branch</th>
                            <th>Risk</th>
                            <th>Duration</th>
                            <th>Scanned At</th>
                            <th></th>
                        </tr>
                    </thead>
                    <tbody>
                        {scans.map(scan => (
                            <tr key={scan.id} className={styles.row}>
                                <td><GradeBadge grade={scan.maturity_grade} /></td>
                                <td className={styles.scoreCell}>{scan.composite_maturity}</td>
                                <td className={styles.mutedCell}>
                                    {scan.branch
                                        ? <span className={styles.branchBadge}>⎇ {scan.branch}</span>
                                        : <span className={styles.mutedCell}>—</span>
                                    }
                                </td>
                                <td className={styles.mutedCell}>{scan.risk_score}</td>
                                <td className={styles.mutedCell}>{(scan.duration_ms / 1000).toFixed(1)}s</td>
                                <td className={styles.mutedCell}>
                                    {scan.scanned_at.slice(0, 16).replace('T', ' ')}
                                </td>
                                <td className={styles.selectCell}>
                                    <button
                                        className={styles.selectBtn}
                                        onClick={() => { onSelectScan(scan.id); onClose() }}
                                    >
                                        View
                                    </button>
                                </td>
                            </tr>
                        ))}
                    </tbody>
                </table>
            )}
        </Modal>
    )
}
