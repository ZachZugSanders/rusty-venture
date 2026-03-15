import { useState, useEffect } from 'react'
import styles from './HistoryPage.module.css'

const GRADE_COLOR = grade => {
  const tier = (grade ?? '').toUpperCase()

  if (['DIAMOND', 'PLATINUM', 'EXEMPLARY', 'ESTABLISHED'].includes(tier)) return 'green'
  if (['GOLD', 'SILVER', 'DEVELOPING', 'EMERGING'].includes(tier)) return 'yellow'
  return 'red'
}

export default function HistoryPage() {
  const [scans, setScans] = useState([])
  const [status, setStatus] = useState('loading')
  const [filter, setFilter] = useState('')

  useEffect(() => {
    load()
  }, [])

  async function load(repo) {
    setStatus('loading')
    try {
      const url = repo ? `/scans?repo=${encodeURIComponent(repo)}&limit=50` : '/scans?limit=50'
      const res = await fetch(url)
      const json = await res.json()
      setScans(json.data ?? [])
      setStatus('done')
    } catch {
      setStatus('error')
    }
  }

  function handleFilter(e) {
    e.preventDefault()
    load(filter.trim() || undefined)
  }

  const riskLabel = score =>
    score <= 20 ? ['LOW', 'green'] :
      score <= 50 ? ['MEDIUM', 'yellow'] :
        score <= 80 ? ['HIGH', 'red'] :
          ['CRITICAL', 'red']

  return (
    <div>
      <h1 className={styles.heading}>Scan History</h1>

      <form onSubmit={handleFilter} className={styles.filterRow}>
        <input
          type="text"
          placeholder="Filter by repo URL…"
          value={filter}
          onChange={e => setFilter(e.target.value)}
        />
        <button type="submit" className={styles.filterBtn}>Filter</button>
        {filter && (
          <button type="button" className={styles.clearBtn}
            onClick={() => { setFilter(''); load() }}>
            Clear
          </button>
        )}
      </form>

      {status === 'loading' && <p className={styles.muted}>Loading…</p>}
      {status === 'error' && <p className={styles.errorText}>Failed to load scan history.</p>}

      {status === 'done' && scans.length === 0 && (
        <p className={styles.muted}>No scans found. Run an analysis to get started.</p>
      )}

      {status === 'done' && scans.length > 0 && (
        <table className={styles.table}>
          <thead>
            <tr>
              <th>Repository</th>
              <th>Date</th>
              <th>Grade</th>
              <th>Maturity</th>
              <th>Risk</th>
            </tr>
          </thead>
          <tbody>
            {scans.map(scan => {
              const [rl, rc] = riskLabel(scan.risk_score)
              const gc = GRADE_COLOR(scan.maturity_grade)
              return (
                <tr key={scan.id}>
                  <td className={styles.repoCell}>
                    <a href={scan.repo_url} target="_blank" rel="noopener noreferrer">
                      {scan.repo_url.replace(/^https?:\/\/(www\.)?/, '')}
                    </a>
                    <span className={styles.scanId}>{scan.id.slice(0, 8)}</span>
                  </td>
                  <td className={styles.muted}>{scan.scanned_at?.slice(0, 10)}</td>
                  <td><span className={styles[gc]}>{scan.maturity_grade}</span></td>
                  <td>{scan.composite_maturity}/100</td>
                  <td><span className={styles[rc]}>{rl}</span></td>
                </tr>
              )
            })}
          </tbody>
        </table>
      )}
    </div>
  )
}
