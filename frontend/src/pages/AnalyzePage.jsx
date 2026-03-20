import { useState } from 'react'
import ReportView from '../components/ReportView.jsx'
import styles from './AnalyzePage.module.css'

export default function AnalyzePage() {
  const [repoUrl, setRepoUrl] = useState('')
  const [branch, setBranch] = useState('')
  const [status, setStatus] = useState('idle') // idle | loading | done | error
  const [result, setResult] = useState(null)
  const [errorMsg, setErrorMsg] = useState('')

  async function handleSubmit(e) {
    e.preventDefault()
    if (!repoUrl.trim()) return

    setStatus('loading')
    setResult(null)
    setErrorMsg('')

    try {
      const res = await fetch('/analyze', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({
          repo_url: repoUrl.trim(),
          branch: branch.trim() || undefined,
        }),
      })

      const json = await res.json()

      if (!res.ok || !json.success) {
        setErrorMsg(json.error || `HTTP ${res.status}`)
        setStatus('error')
      } else {
        setResult(json.data)
        setStatus('done')
      }
    } catch (err) {
      setErrorMsg(err.message)
      setStatus('error')
    }
  }

  return (
    <div>
      <h1 className={styles.heading}>Repository Analysis</h1>
      <p className={styles.sub}>
        Clones the repo in an isolated container, detects language and
        dependencies, audits for security issues, and scores maturity.
      </p>

      <form onSubmit={handleSubmit} className={styles.form}>
        <div className={styles.row}>
          <input
            type="url"
            placeholder="https://github.com/owner/repo"
            value={repoUrl}
            onChange={e => setRepoUrl(e.target.value)}
            required
            disabled={status === 'loading'}
          />
          <input
            type="text"
            placeholder="branch (optional)"
            value={branch}
            onChange={e => setBranch(e.target.value)}
            disabled={status === 'loading'}
            className={styles.branchInput}
          />
          <button
            type="submit"
            disabled={status === 'loading'}
            className={styles.submitBtn}
          >
            {status === 'loading' ? 'Analyzing…' : 'Analyze'}
          </button>
        </div>
      </form>

      {status === 'loading' && (
        <div className={styles.progress}>
          <span className={styles.spinner} />
          Running pipeline — this takes ~60 seconds on first run (image pull)…
        </div>
      )}

      {status === 'error' && (
        <div className={styles.error}>
          <strong>Analysis failed:</strong> {errorMsg}
        </div>
      )}

      {status === 'done' && result && <ReportView result={result} />}
    </div>
  )
}
