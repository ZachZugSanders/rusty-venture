import { useState, useEffect, useCallback } from 'react'
import ReportView from '../components/ReportView.jsx'
import styles from './ReposPage.module.css'

const GRADE_CLASS = g => {
  const tier = (g ?? '').toUpperCase()

  if (['DIAMOND', 'PLATINUM', 'EXEMPLARY', 'ESTABLISHED'].includes(tier)) return 'green'
  if (['GOLD', 'SILVER', 'DEVELOPING', 'EMERGING'].includes(tier)) return 'yellow'
  return 'red'
}

const RISK_LABEL = s =>
  s <= 20 ? ['LOW', 'green'] : s <= 50 ? ['MEDIUM', 'yellow'] : s <= 80 ? ['HIGH', 'red'] : ['CRITICAL', 'red']

function Badge({ value, cls }) {
  return <span className={`${styles.badge} ${styles[cls]}`}>{value}</span>
}

// ── Repo detail panel ───────────────────────────────────────────────────────

function ScanHistoryTab({ repoUrl }) {
  const [scans, setScans] = useState([])
  const [loading, setLoading] = useState(true)
  const [selected, setSelected] = useState(null)
  const [detail, setDetail] = useState(null)
  const [detailLoading, setDetailLoading] = useState(false)

  useEffect(() => {
    fetch(`/scans?repo=${encodeURIComponent(repoUrl)}&limit=50`)
      .then(r => r.json())
      .then(j => { setScans(j.data ?? []); setLoading(false) })
      .catch(() => setLoading(false))
  }, [repoUrl])

  async function loadDetail(scanId) {
    if (selected === scanId) { setSelected(null); setDetail(null); return }
    setSelected(scanId)
    setDetailLoading(true)
    const res = await fetch(`/scans/${scanId}`)
    const json = await res.json()
    setDetail(json.data ?? null)
    setDetailLoading(false)
  }

  if (loading) return <p className={styles.muted}>Loading…</p>
  if (!scans.length) return <p className={styles.muted}>No scans found for this repo.</p>

  return (
    <div className={styles.historyTab}>
      <table className={styles.miniTable}>
        <thead>
          <tr><th>Date</th><th>Grade</th><th>Maturity</th><th>Risk</th><th>Duration</th><th /></tr>
        </thead>
        <tbody>
          {scans.map(s => {
            const [rl, rc] = RISK_LABEL(s.risk_score)
            const gc = GRADE_CLASS(s.maturity_grade)
            return (
              <>
                <tr
                  key={s.id}
                  className={`${styles.scanRow} ${selected === s.id ? styles.scanRowActive : ''}`}
                  onClick={() => loadDetail(s.id)}
                >
                  <td className={styles.muted}>{s.scanned_at?.slice(0, 10)}</td>
                  <td><Badge value={s.maturity_grade} cls={gc} /></td>
                  <td>{s.composite_maturity}/100</td>
                  <td><Badge value={rl} cls={rc} /></td>
                  <td className={styles.muted}>{(s.duration_ms / 1000).toFixed(1)}s</td>
                  <td className={styles.chevron}>{selected === s.id ? '▲' : '▼'}</td>
                </tr>
                {selected === s.id && (
                  <tr key={`${s.id}-detail`}>
                    <td colSpan={6} className={styles.expandedCell}>
                      {detailLoading
                        ? <p className={styles.muted}>Loading report…</p>
                        : detail && <ReportView result={detail} />}
                    </td>
                  </tr>
                )}
              </>
            )
          })}
        </tbody>
      </table>
    </div>
  )
}

function RepoDetail({ repo, onClose }) {
  const [tab, setTab] = useState('current')
  const [detail, setDetail] = useState(null)
  const [loading, setLoading] = useState(false)

  useEffect(() => {
    if (!repo.latest_scan_id) return
    setLoading(true)
    fetch(`/scans/${repo.latest_scan_id}`)
      .then(r => r.json())
      .then(j => { setDetail(j.data ?? null); setLoading(false) })
      .catch(() => setLoading(false))
  }, [repo.latest_scan_id])

  return (
    <div className={styles.panel}>
      <div className={styles.panelHeader}>
        <div className={styles.panelTitle}>
          <span className={styles.repoName}>{repo.url.replace(/^https?:\/\/(www\.)?/, '')}</span>
          <span className={styles.scanCount}>{repo.scan_count} scan{repo.scan_count !== 1 ? 's' : ''}</span>
        </div>
        <button className={styles.closeBtn} onClick={onClose}>✕</button>
      </div>

      <div className={styles.panelTabs}>
        {['current', 'history'].map(t => (
          <button
            key={t}
            className={`${styles.panelTab} ${tab === t ? styles.panelTabActive : ''}`}
            onClick={() => setTab(t)}
          >
            {t === 'current' ? 'Current State' : 'History'}
          </button>
        ))}
      </div>

      <div className={styles.panelBody}>
        {tab === 'current' && (
          loading
            ? <p className={styles.muted}>Loading latest report…</p>
            : detail
              ? <ReportView result={detail} />
              : <p className={styles.muted}>No scans yet for this repo.</p>
        )}
        {tab === 'history' && <ScanHistoryTab repoUrl={repo.url} />}
      </div>
    </div>
  )
}

// ── Main page ──────────────────────────────────────────────────────────────

export default function ReposPage() {
  const [repos, setRepos] = useState([])
  const [status, setStatus] = useState('loading')
  const [selected, setSelected] = useState(null)

  const load = useCallback(async () => {
    setStatus('loading')
    try {
      const res = await fetch('/repos')
      const json = await res.json()
      setRepos(json.data ?? [])
      setStatus('done')
    } catch {
      setStatus('error')
    }
  }, [])

  useEffect(() => { load() }, [load])

  return (
    <div className={styles.root}>
      <div className={`${styles.tablePane} ${selected ? styles.tablePaneNarrow : ''}`}>
        <div className={styles.pageHeader}>
          <h1 className={styles.heading}>Repos</h1>
          <button className={styles.refreshBtn} onClick={load}>↺ Refresh</button>
        </div>

        {status === 'loading' && <p className={styles.muted}>Loading…</p>}
        {status === 'error' && <p className={styles.errorText}>Failed to load repos.</p>}

        {status === 'done' && repos.length === 0 && (
          <p className={styles.muted}>No repos yet. Run an analysis to get started.</p>
        )}

        {status === 'done' && repos.length > 0 && (
          <table className={styles.table}>
            <thead>
              <tr>
                <th>Repository</th>
                {!selected && <><th>Last Scanned</th><th>Grade</th><th>Maturity</th><th>Risk</th><th>Scans</th></>}
                <th />
              </tr>
            </thead>
            <tbody>
              {repos.map(repo => {
                const gc = repo.latest_maturity_grade ? GRADE_CLASS(repo.latest_maturity_grade) : 'muted'
                const [rl, rc] = repo.latest_risk_score != null ? RISK_LABEL(repo.latest_risk_score) : ['—', 'muted']
                const isActive = selected?.id === repo.id
                return (
                  <tr
                    key={repo.id}
                    className={`${styles.repoRow} ${isActive ? styles.repoRowActive : ''}`}
                    onClick={() => setSelected(isActive ? null : repo)}
                  >
                    <td className={styles.repoCell}>
                      <span className={styles.repoUrl}>
                        {repo.url.replace(/^https?:\/\/(www\.)?/, '')}
                      </span>
                    </td>
                    {!selected && (
                      <>
                        <td className={styles.muted}>{repo.last_scanned?.slice(0, 10) ?? '—'}</td>
                        <td>{repo.latest_maturity_grade
                          ? <Badge value={repo.latest_maturity_grade} cls={gc} />
                          : <span className={styles.muted}>—</span>}
                        </td>
                        <td>{repo.latest_composite_maturity != null ? `${repo.latest_composite_maturity}/100` : '—'}</td>
                        <td><Badge value={rl} cls={rc} /></td>
                        <td className={styles.muted}>{repo.scan_count}</td>
                      </>
                    )}
                    <td className={styles.chevron}>{isActive ? '◀' : '▶'}</td>
                  </tr>
                )
              })}
            </tbody>
          </table>
        )}
      </div>

      {selected && (
        <div className={styles.detailPane}>
          <RepoDetail repo={selected} onClose={() => setSelected(null)} />
        </div>
      )}
    </div>
  )
}
