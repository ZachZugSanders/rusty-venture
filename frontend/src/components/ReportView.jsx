import styles from './ReportView.module.css'

const RISK_LABEL = score =>
  score <= 20 ? ['LOW', styles.green] :
    score <= 50 ? ['MEDIUM', styles.yellow] :
      score <= 80 ? ['HIGH', styles.red] :
        ['CRITICAL', styles.red]

const GRADE_CLASS = g => {
  const tier = (g ?? '').toUpperCase()

  if (['DIAMOND', 'PLATINUM', 'EXEMPLARY', 'ESTABLISHED'].includes(tier)) {
    return styles.green
  }

  if (['GOLD', 'SILVER', 'DEVELOPING', 'EMERGING'].includes(tier)) {
    return styles.yellow
  }

  return styles.red
}

function ScoreBar({ score }) {
  const filled = Math.min(10, Math.round(score / 10))
  return (
    <div className={styles.barWrap}>
      {Array.from({ length: 10 }, (_, i) => (
        <div key={i} className={`${styles.bar} ${i < filled ? styles.barFilled : ''}`} />
      ))}
      <span className={styles.barScore}>{score}</span>
    </div>
  )
}

function Section({ title, items }) {
  if (!items?.length) return null
  return (
    <div className={styles.section}>
      <h3 className={styles.sectionTitle}>{title}</h3>
      <ul className={styles.list}>
        {items.map((item, i) => <li key={i}>{item}</li>)}
      </ul>
    </div>
  )
}

export default function ReportView({ result }) {
  const { report, maturity, duration_ms } = result
  const [riskLabel, riskClass] = RISK_LABEL(report.risk_score)
  const gradeClass = GRADE_CLASS(maturity.grade)

  const failed = maturity.dimensions
    .flatMap(d => d.signals.filter(s => !s.passed).map(s => ({ dim: d.dimension, ...s })))

  return (
    <div className={styles.root}>
      {/* ── Score banner ── */}
      <div className={styles.banner}>
        <div className={styles.bannerItem}>
          <span className={styles.bannerLabel}>Risk</span>
          <span className={`${styles.bannerValue} ${riskClass}`}>
            {report.risk_score}/100 <small>{riskLabel}</small>
          </span>
        </div>
        <div className={styles.bannerDivider} />
        <div className={styles.bannerItem}>
          <span className={styles.bannerLabel}>Maturity</span>
          <span className={`${styles.bannerValue} ${gradeClass}`}>
            {maturity.composite}/100
          </span>
        </div>
        <div className={styles.bannerDivider} />
        <div className={styles.bannerItem}>
          <span className={styles.bannerLabel}>Grade</span>
          <span className={`${styles.bannerValue} ${gradeClass}`}>{maturity.grade}</span>
        </div>
        <div className={styles.bannerDivider} />
        <div className={styles.bannerItem}>
          <span className={styles.bannerLabel}>Duration</span>
          <span className={styles.bannerValue}>{(duration_ms / 1000).toFixed(1)}s</span>
        </div>
      </div>

      {/* ── Summary ── */}
      <div className={styles.section}>
        <h3 className={styles.sectionTitle}>Summary</h3>
        <p className={styles.summary}>{report.summary}</p>
      </div>

      {/* ── Maturity dimensions ── */}
      <div className={styles.section}>
        <h3 className={styles.sectionTitle}>Maturity Breakdown</h3>
        <div className={styles.dimensions}>
          {maturity.dimensions.map(d => (
            <div key={d.dimension} className={styles.dimRow}>
              <span className={styles.dimLabel}>{d.dimension}</span>
              <ScoreBar score={d.score} />
            </div>
          ))}
        </div>
      </div>

      {/* ── Failed signals ── */}
      {failed.length > 0 && (
        <div className={styles.section}>
          <h3 className={styles.sectionTitle}>Improvement Actions</h3>
          <ul className={styles.actionList}>
            {failed.map((s, i) => (
              <li key={i} className={styles.actionItem}>
                <span className={styles.actionName}>{s.name}</span>
                <span className={styles.actionDetail}>{s.detail ?? s.description}</span>
              </li>
            ))}
          </ul>
        </div>
      )}

      <Section title="Language Insights" items={report.language_insights} />
      <Section title="Dependency Recommendations" items={report.dependency_recommendations} />
      <Section title="Dockerfile Findings" items={report.dockerfile_findings} />
      <Section title="Security Violations" items={report.security_violations} />
      <Section title="General Recommendations" items={report.general_recommendations} />
    </div>
  )
}
