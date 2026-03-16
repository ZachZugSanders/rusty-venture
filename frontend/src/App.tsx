import { useState } from 'react'
import ReposView from './views/ReposView'
import HistoryView from './views/HistoryView'
import AnalyzeView from './views/AnalyzeView'
import styles from './App.module.css'

type Tab = 'repos' | 'history' | 'analyze'

export default function App() {
    const [tab, setTab] = useState<Tab>('repos')

    return (
        <div className={styles.shell}>
            <nav className={styles.nav}>
                <span className={styles.logo}>🦀 Rusty Venture</span>
                <button
                    className={`${styles.navTab} ${tab === 'repos' ? styles.navTabActive : ''}`}
                    onClick={() => setTab('repos')}
                >
                    Repos
                </button>
                <button
                    className={`${styles.navTab} ${tab === 'history' ? styles.navTabActive : ''}`}
                    onClick={() => setTab('history')}
                >
                    History
                </button>
                <button
                    className={`${styles.navTab} ${tab === 'analyze' ? styles.navTabActive : ''}`}
                    onClick={() => setTab('analyze')}
                >
                    Analyze
                </button>
            </nav>
            <main className={styles.main}>
                {tab === 'repos' && <ReposView />}
                {tab === 'history' && <HistoryView />}
                {tab === 'analyze' && <AnalyzeView />}
            </main>
        </div>
    )
}
