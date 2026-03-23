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

type Tier = 1 | 2 | 3

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

interface TierCheck { icon: string; text: string }

interface TierDef {
    tier: Tier
    label: string
    name: string
    badge: string
    color: string
    description: string
    note?: string
    checks: TierCheck[]
}

const TIER_DEFS: TierDef[] = [
    {
        tier: 1,
        label: 'T1',
        name: 'Static Analysis',
        badge: '🔍',
        color: '#94a3b8',
        description: 'File-system presence checks only — no file reads, no execution. Fast and safe on any repo.',
        checks: [
            { icon: '📁', text: 'Repository structure & manifest files' },
            { icon: '🔒', text: 'Committed secrets & audit file scan' },
            { icon: '🐋', text: 'Dockerfile & CI configuration detection' },
            { icon: '📦', text: 'Dependency manifest, lock file & count analysis' },
            { icon: '📋', text: 'Governance file presence (LICENSE, README, CHANGELOG, CONTRIBUTING)' },
            { icon: '🧪', text: 'Test directory & coverage tooling detection' },
        ],
    },
    {
        tier: 2,
        label: 'T2',
        name: 'Content Quality',
        badge: '📖',
        color: '#4fa3e0',
        description: 'Reads file contents to assess documentation depth, governance quality, and AI context coverage.',
        note: 'Requires a completed Tier 1 scan for this repository.',
        checks: [
            { icon: '📄', text: 'README depth & coverage analysis' },
            { icon: '📋', text: 'CONTRIBUTING.md, CHANGELOG & governance quality' },
            { icon: '🔐', text: 'SECURITY.md vulnerability disclosure detail' },
            { icon: '🤖', text: 'LLM context files present and covering key modules' },
            { icon: '🧠', text: 'AI model configuration quality (CLAUDE.md / .cursorrules depth)' },
        ],
    },
    {
        tier: 3,
        label: 'T3',
        name: 'Active Validation',
        badge: '⚡',
        color: '#f0c040',
        description: 'Spawns an execution container and actively builds, tests, and runs analysis tooling.',
        note: 'Requires a completed Tier 2 scan. Slower — may install packages.',
        checks: [
            { icon: '🏗️', text: 'Build system validation (cargo build, npm install, etc.)' },
            { icon: '🧪', text: 'Test suite execution & pass rate' },
            { icon: '🔬', text: 'Linter & static analysis tool execution' },
            { icon: '🤖', text: 'LLM context quality scoring via AI' },
            { icon: '📊', text: 'Code quality metrics collection' },
        ],
    },
]

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
    const [tier, setTier] = useState<Tier>(1)
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
                    tier,
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

    const activeTierDef = TIER_DEFS.find(t => t.tier === tier)!

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

                {/* ── Tier selector ───────────────────────────────────────── */}
                <div className={styles.tierSection}>
                    <span className={styles.pipelineSectionLabel}>Analysis Tier</span>
                    <div className={styles.tierTabs}>
                        {TIER_DEFS.map(td => (
                            <button
                                key={td.tier}
                                type="button"
                                className={`${styles.tierTab} ${tier === td.tier ? styles.tierTabActive : ''}`}
                                style={{ '--tier-color': td.color } as React.CSSProperties}
                                onClick={() => setTier(td.tier)}
                                disabled={submitting}
                            >
                                <span className={styles.tierTabBadge}>{td.badge}</span>
                                <span className={styles.tierTabLabel}>{td.label}</span>
                                <span className={styles.tierTabName}>{td.name}</span>
                            </button>
                        ))}
                    </div>
                </div>

                {/* ── Tier panel ──────────────────────────────────────────── */}
                {tier === 1 ? (
                    /* T1: Full pipeline tree */
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
                ) : (
                    /* T2/T3: Tier info panel */
                    <div className={styles.tierPanel}>
                        {activeTierDef.note && (
                            <div className={styles.tierNote}>
                                <span className={styles.tierNoteIcon}>ℹ️</span>
                                {activeTierDef.note}
                            </div>
                        )}
                        <p className={styles.tierDesc}>{activeTierDef.description}</p>
                        <div className={styles.tierChecksLabel}>
                            <span className={styles.pipelineSectionLabel}>Checks included</span>
                            <span className={styles.tierChecksIncludes}>+ all T{tier - 1 as 1 | 2} checks</span>
                        </div>
                        <ul className={styles.tierCheckList}>
                            {activeTierDef.checks.map((c, i) => (
                                <li key={i} className={styles.tierCheckItem}>
                                    <span className={styles.tierCheckIcon}>{c.icon}</span>
                                    <span className={styles.tierCheckText}>{c.text}</span>
                                </li>
                            ))}
                        </ul>
                    </div>
                )}

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
                        ) : (
                            <>{activeTierDef.badge} Run {activeTierDef.label} Analysis</>
                        )}
                    </button>
                </div>

            </form>
        </Modal>
    )
}
