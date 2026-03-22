import { useEffect, useRef, useState } from 'react'
import type { RunStreamEvent } from '../types'
import styles from './RunProgressPanel.module.css'

interface LogEntry {
    level: string
    step?: string
    message: string
}

interface Props {
    runId: string
    repoUrl: string
    onComplete: (repoUrl: string) => void
    onError: () => void
}

export default function RunProgressPanel({ runId, repoUrl, onComplete, onError }: Props) {
    const [lines, setLines] = useState<LogEntry[]>([])
    const [status, setStatus] = useState<'running' | 'done' | 'error'>('running')
    const [errorMsg, setErrorMsg] = useState<string | null>(null)
    const logEndRef = useRef<HTMLDivElement>(null)

    useEffect(() => {
        const es = new EventSource(`/runs/${runId}/stream`)

        es.onmessage = (e) => {
            try {
                const event: RunStreamEvent = JSON.parse(e.data)
                if (event.type === 'log') {
                    setLines(prev => [...prev, {
                        level: event.level,
                        step: event.step,
                        message: event.message,
                    }])
                } else if (event.type === 'done') {
                    setStatus('done')
                    es.close()
                    setTimeout(() => onComplete(event.repo_url), 800)
                } else if (event.type === 'failed') {
                    setStatus('error')
                    setErrorMsg(event.error)
                    es.close()
                    setTimeout(() => onError(), 3000)
                }
            } catch {
                // ignore malformed events
            }
        }

        es.onerror = () => {
            if (status === 'running') {
                setStatus('error')
                setErrorMsg('Lost connection to server')
                es.close()
                setTimeout(() => onError(), 3000)
            }
        }

        return () => es.close()
    // eslint-disable-next-line react-hooks/exhaustive-deps
    }, [runId])

    // Auto-scroll log to bottom
    useEffect(() => {
        logEndRef.current?.scrollIntoView({ behavior: 'smooth' })
    }, [lines])

    const displayUrl = repoUrl.replace(/^https?:\/\/(www\.)?github\.com\//, '')

    return (
        <div className={styles.panel}>
            <div className={styles.header}>
                <div className={styles.repoInfo}>
                    <span className={styles.repoUrl}>{displayUrl}</span>
                </div>
                <div className={`${styles.badge} ${styles[status]}`}>
                    {status === 'running' && <span className={styles.pulse} />}
                    {status === 'running' ? 'Analyzing…' : status === 'done' ? 'Complete' : 'Failed'}
                </div>
            </div>

            {status === 'done' && (
                <div className={styles.doneBar}>Analysis complete — loading results…</div>
            )}
            {status === 'error' && errorMsg && (
                <div className={styles.errorBar}>{errorMsg}</div>
            )}

            <div className={styles.logWrap}>
                {lines.length === 0 && status === 'running' && (
                    <div className={styles.logEmpty}>Waiting for workflow to start…</div>
                )}
                {lines.map((line, i) => (
                    <div key={i} className={`${styles.logLine} ${styles[`level_${line.level}`]}`}>
                        <span className={styles.logLevel}>{line.level.toUpperCase()}</span>
                        {line.step && <span className={styles.logStep}>{line.step}</span>}
                        <span className={styles.logMsg}>{line.message}</span>
                    </div>
                ))}
                <div ref={logEndRef} />
            </div>
        </div>
    )
}
