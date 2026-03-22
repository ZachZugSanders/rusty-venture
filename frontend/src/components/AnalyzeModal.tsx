import { useState } from 'react'
import Modal from './Modal'
import styles from './AnalyzeModal.module.css'

interface Props {
    onClose: () => void
    onStarted: (runId: string, repoUrl: string) => void
}

type ActionId =
    | 'spawn_container'
    | 'clone_repo'
    | 'detect_language'
    | 'analyze_deps'
    | 'find_dockerfiles'
    | 'audit_files'
    | 'generate_report'
    | 'cleanup_container'

interface ActionDef {
    id: ActionId
    icon: string
    name: string
    desc: string
    required: boolean
    requiresContainer?: boolean
}

interface Stage {
    id: string
    mode: 'sequential' | 'parallel'
    actions: ActionDef[]
}

const PIPELINE: Stage[] = [
    {
        id: 's1',
        mode: 'sequential',
        actions: [{
            id: 'spawn_container',
            icon: '🐳',
            name: 'Spawn Container',
            desc: 'Boots an isolated Ubuntu 22.04 analysis environment via Docker',
            required: false,
            requiresContainer: true,
        }],
    },
    {
        id: 's2',
        mode: 'sequential',
        actions: [{
            id: 'clone_repo',
            icon: '📥',
            name: 'Clone Repository',
            desc: 'git clone into the analysis environment',
            required: true,
        }],
    },
    {
        id: 's3',
        mode: 'sequential',
        actions: [{
            id: 'detect_language',
            icon: '🔍',
            name: 'Detect Language',
            desc: 'File marker scoring to identify primary languages',
            required: true,
        }],
    },
    {
        id: 's4',
        mode: 'parallel',
        actions: [
            {
                id: 'analyze_deps',
                icon: '📦',
                name: 'Analyze Dependencies',
                desc: 'Language-specific dependency parsers',
                required: true,
            },
            {
                id: 'find_dockerfiles',
                icon: '🐋',
                name: 'Find Dockerfiles',
                desc: 'Recursive search for Docker configurations',
                required: false,
            },
            {
                id: 'audit_files',
                icon: '🔒',
                name: 'Audit Committed Files',
                desc: 'Deny-list scan for secrets and sensitive files',
                required: false,
            },
        ],
    },
    {
        id: 's5',
        mode: 'sequential',
        actions: [{
            id: 'generate_report',
            icon: '✨',
            name: 'Generate Report',
            desc: 'LLM synthesis of all collected analysis signals',
            required: true,
        }],
    },
    {
        id: 's6',
        mode: 'sequential',
        actions: [{
            id: 'cleanup_container',
            icon: '🧹',
            name: 'Cleanup Container',
            desc: 'Stop and remove the analysis container',
            required: false,
            requiresContainer: true,
        }],
    },
]

export default function AnalyzeModal({ onClose, onStarted }: Props) {
    const [repoUrl, setRepoUrl] = useState('')
    const [branch, setBranch] = useState('')
    const [containerEnabled, setContainerEnabled] = useState(true)
    const [cacheRepoImage, setCacheRepoImage] = useState(false)
    const [enabledOptionals, setEnabledOptionals] = useState<Set<ActionId>>(
        new Set(['find_dockerfiles', 'audit_files', 'cleanup_container'])
    )
    const [submitting, setSubmitting] = useState(false)
    const [error, setError] = useState<string | null>(null)

    const toggleOptional = (id: ActionId) => {
        setEnabledOptionals(prev => {
            const next = new Set(prev)
            if (next.has(id)) next.delete(id)
            else next.add(id)
            return next
        })
    }

    const isActive = (action: ActionDef): boolean => {
        if (action.id === 'spawn_container') return containerEnabled
        if (action.requiresContainer && !containerEnabled) return false
        if (action.required) return true
        return enabledOptionals.has(action.id)
    }

    const handleSubmit = async (e: React.FormEvent) => {
        e.preventDefault()
        setSubmitting(true)
        setError(null)
        try {
            const res = await fetch('/analyze', {
                method: 'POST',
                headers: { 'Content-Type': 'application/json' },
                body: JSON.stringify({
                    repo_url: repoUrl,
                    branch: branch || undefined,
                    no_container: !containerEnabled,
                    cache_repo_image: cacheRepoImage && containerEnabled,
                }),
            })
            const json = await res.json()
            if (json.success && json.data?.run_id) {
                onStarted(json.data.run_id, repoUrl)
            } else {
                setError(json.error ?? `Server error ${res.status}`)
                setSubmitting(false)
            }
        } catch (err) {
            setError(`Network error — is rusty-venture-server running? (${String(err)})`)
            setSubmitting(false)
        }
    }

    return (
        <Modal title="Analyze Repository" onClose={onClose} size="lg">
            <form onSubmit={handleSubmit} className={styles.form}>

                {/* ── Inputs ─────────────────────────────────────────────── */}
                <div className={styles.inputs}>
                    <label className={styles.fieldGroup}>
                        <span className={styles.fieldLabel}>Repository URL</span>
                        <input
                            type="url"
                            value={repoUrl}
                            onChange={e => setRepoUrl(e.target.value)}
                            placeholder="https://github.com/owner/repo"
                            required
                            className={styles.input}
                            disabled={submitting}
                        />
                    </label>
                    <label className={styles.fieldGroup}>
                        <span className={styles.fieldLabel}>
                            Branch <span className={styles.optional}>(optional)</span>
                        </span>
                        <input
                            type="text"
                            value={branch}
                            onChange={e => setBranch(e.target.value)}
                            placeholder="main"
                            className={styles.input}
                            disabled={submitting}
                        />
                    </label>
                </div>

                {/* ── Pipeline tree ───────────────────────────────────────── */}
                <div className={styles.pipelineSection}>
                    <span className={styles.pipelineSectionLabel}>Pipeline</span>
                    <div className={styles.pipeline}>
                        {PIPELINE.map((stage, si) => (
                            <div key={stage.id} className={styles.stageWrap}>
                                {si > 0 && (
                                    <div className={styles.connector}>
                                        <div className={styles.connLine} />
                                        <div className={styles.connArrow}>↓</div>
                                        <div className={styles.connLine} />
                                    </div>
                                )}
                                <div className={`${styles.stage} ${stage.mode === 'parallel' ? styles.stageParallel : ''}`}>
                                    <div className={styles.stageModeTag}>
                                        {stage.mode === 'parallel' ? '⚡ Parallel' : '→ Sequential'}
                                    </div>
                                    <div className={`${styles.stageActions} ${stage.mode === 'parallel' ? styles.stageActionsRow : ''}`}>
                                        {stage.actions.map(action => {
                                            const active = isActive(action)
                                            const isContainerToggle = action.id === 'spawn_container'
                                            const forceDisabled = action.requiresContainer && !containerEnabled && action.id !== 'spawn_container'

                                            return (
                                                <div
                                                    key={action.id}
                                                    className={`${styles.actionCard} ${!active ? styles.actionDimmed : ''} ${stage.mode === 'parallel' ? styles.actionCardFlex : ''}`}
                                                >
                                                    <div className={styles.actionRow}>
                                                        <span className={styles.actionIcon}>{action.icon}</span>
                                                        <div className={styles.actionInfo}>
                                                            <span className={styles.actionName}>{action.name}</span>
                                                            <span className={styles.actionDesc}>{action.desc}</span>
                                                        </div>
                                                        <div className={styles.actionControl}>
                                                            {action.required ? (
                                                                <span className={styles.requiredBadge}>Required</span>
                                                            ) : (
                                                                <label className={styles.toggleSwitch}>
                                                                    <input
                                                                        type="checkbox"
                                                                        checked={isContainerToggle ? containerEnabled : enabledOptionals.has(action.id)}
                                                                        onChange={() => {
                                                                            if (isContainerToggle) {
                                                                                setContainerEnabled(v => {
                                                                                    if (v) setCacheRepoImage(false)
                                                                                    return !v
                                                                                })
                                                                            } else {
                                                                                toggleOptional(action.id)
                                                                            }
                                                                        }}
                                                                        disabled={submitting || forceDisabled}
                                                                    />
                                                                    <span className={styles.toggleSlider} />
                                                                </label>
                                                            )}
                                                        </div>
                                                    </div>

                                                    {/* Sub-option: cache image (clone step only) */}
                                                    {action.id === 'clone_repo' && containerEnabled && (
                                                        <label className={styles.subOption}>
                                                            <label className={styles.toggleSwitch}>
                                                                <input
                                                                    type="checkbox"
                                                                    checked={cacheRepoImage}
                                                                    onChange={e => setCacheRepoImage(e.target.checked)}
                                                                    disabled={submitting}
                                                                />
                                                                <span className={styles.toggleSlider} />
                                                            </label>
                                                            <span className={styles.subOptionLabel}>Cache repository image after clone</span>
                                                        </label>
                                                    )}
                                                </div>
                                            )
                                        })}
                                    </div>
                                </div>
                            </div>
                        ))}
                    </div>
                </div>

                {/* ── Footer ─────────────────────────────────────────────── */}
                <div className={styles.formFooter}>
                    {error && (
                        <div className={styles.errorCard}>
                            <strong>Failed to start analysis</strong>
                            <p>{error}</p>
                        </div>
                    )}
                    <button type="submit" disabled={submitting} className={styles.submitBtn}>
                        {submitting ? (
                            <><span className={styles.spinner} />Starting…</>
                        ) : 'Run Analysis'}
                    </button>
                </div>

            </form>
        </Modal>
    )
}
