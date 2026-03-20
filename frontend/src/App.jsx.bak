import { useState } from 'react'
import AnalyzePage from './pages/AnalyzePage.jsx'
import ReposPage from './pages/ReposPage.jsx'
import HistoryPage from './pages/HistoryPage.jsx'
import styles from './App.module.css'

const TABS = [
  { id: 'repos', label: 'Repos' },
  { id: 'analyze', label: 'Analyze' },
  { id: 'history', label: 'History' },
]

export default function App() {
  const [page, setPage] = useState('repos')

  return (
    <div className={styles.shell}>
      <header className={styles.header}>
        <span className={styles.logo}>⚙ rusty-venture</span>
        <nav className={styles.nav}>
          {TABS.map(t => (
            <button
              key={t.id}
              className={page === t.id ? styles.active : ''}
              onClick={() => setPage(t.id)}
            >
              {t.label}
            </button>
          ))}
        </nav>
      </header>

      <main className={styles.main}>
        {page === 'repos' && <ReposPage />}
        {page === 'analyze' && <AnalyzePage />}
        {page === 'history' && <HistoryPage />}
      </main>
    </div>
  )
}
