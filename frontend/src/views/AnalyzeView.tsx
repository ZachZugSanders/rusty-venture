import { useState } from 'react'
import styles from './AnalyzeView.module.css'

export default function AnalyzeView() {
    const [repoUrl, setRepoUrl] = useState('')
    const [branch, setBranch] = useState('')
    const [status, setStatus] = useState<'idle' | 'loading' | 'done' | 'error'>('idle')
    const [result, setResult] = useState<Record<string, unknown> | null>(null)
    const [error, setError] = useState<string | null>(null)

    const handleSubmit = async (e: React.FormEvent) => {
        e.preventDefault()
        setStatus('loading')
        setError(null)
        try {
            const res = await fetch('/analyze', {
                method: 'POST',
                headers: { 'Content-Type': 'application/json' },
                body: JSON.stringify({ repo_url: repoUrl, branch: branch || undefined }),
            })
            const json = await res.json()
            if (json.success) {
                setResult(json.data)
                setStatus('done')
            } else {
                setError(json.error ?? 'Analysis failed')
                setStatus('error')
            }
        } catch (e) {
            setError(String(e))
            setStatus('error')
        }
    }

    return (
        <div className={styles.root}>
            <form onSubmit={handleSubmit} className={styles.form}>
                <h2 className={styles.heading}>Analyze a Repository</h2>
                <label className={styles.label}>
                    Repository URL
                    <input
                        type="url"
                        value={repoUrl}
                        onChange={e => setRepoUrl(e.target.value)}
                        placeholder="https://github.com/owner/repo"
                        required
                        className={styles.input}
                    />
                </label>
                <label className={styles.label}>
                    Branch (optional)
                    <input
                        type="text"
                        value={branch}
                        onChange={e => setBranch(e.target.value)}
                        placeholder="main"
                        className={styles.input}
                    />
                </label>
                <button type="submit" disabled={status === 'loading'} className={styles.button}>
                    {status === 'loading' ? 'Analyzing…' : 'Analyze'}
                </button>
            </form>

            {status === 'error' && (
                <p className={styles.error}>{error}</p>
            )}
            {status === 'done' && result && (
                <pre className={styles.result}>{JSON.stringify(result, null, 2)}</pre>
            )}
        </div>
    )
}
