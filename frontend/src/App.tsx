import { useState } from 'react'
import ReposView from './views/ReposView'
import MaturityGraphView from './views/MaturityGraphView'
import styles from './App.module.css'

type Tab = 'repos' | 'graph'

const NAV: { id: Tab; label: string; icon: JSX.Element }[] = [
    {
        id: 'repos',
        label: 'Repositories',
        icon: (
            <svg viewBox="0 0 16 16" fill="currentColor" width="15" height="15">
                <path d="M2 2.5A2.5 2.5 0 0 1 4.5 0h8.75a.75.75 0 0 1 .75.75v12.5a.75.75 0 0 1-.75.75h-2.5a.75.75 0 0 1 0-1.5h1.75v-2h-8a1 1 0 0 0-.714 1.7.75.75 0 1 1-1.072 1.05A2.495 2.495 0 0 1 2 11.5Zm10.5-1h-8a1 1 0 0 0-1 1v6.708A2.486 2.486 0 0 1 4.5 9h8Z" />
            </svg>
        ),
    },
    {
        id: 'graph',
        label: 'Maturity Graph',
        icon: (
            <svg viewBox="0 0 16 16" fill="currentColor" width="15" height="15">
                <path d="M7.5 1.75a.75.75 0 0 1 1.5 0v.581a6.003 6.003 0 0 1 4.688 5.168H14.5a.75.75 0 0 1 0 1.5h-.813a6.003 6.003 0 0 1-4.687 5.168V14.5a.75.75 0 0 1-1.5 0v-.833A6.003 6.003 0 0 1 2.813 9H2a.75.75 0 0 1 0-1.5h.813A6.003 6.003 0 0 1 7.5 2.331ZM8 3.5a4.5 4.5 0 1 0 0 9 4.5 4.5 0 0 0 0-9Zm0 2a2.5 2.5 0 1 1 0 5 2.5 2.5 0 0 1 0-5Z" />
            </svg>
        ),
    },
]

export default function App() {
    const [tab, setTab] = useState<Tab>('repos')

    return (
        <div className={styles.shell}>
            <aside className={styles.sidebar}>
                <div className={styles.logoWrap}>
                    <span className={styles.logoEmoji}>🦀</span>
                    <div>
                        <div className={styles.logoName}>Rusty Venture</div>
                        <div className={styles.logoTagline}>Repo Maturity</div>
                    </div>
                </div>

                <nav className={styles.navList}>
                    {NAV.map(item => (
                        <button
                            key={item.id}
                            className={`${styles.navItem} ${tab === item.id ? styles.navItemActive : ''}`}
                            onClick={() => setTab(item.id)}
                        >
                            <span className={styles.navIcon}>{item.icon}</span>
                            <span>{item.label}</span>
                        </button>
                    ))}
                </nav>
            </aside>

            <main className={styles.main}>
                {tab === 'repos' && <ReposView />}
                {tab === 'graph' && <MaturityGraphView />}
            </main>
        </div>
    )
}
