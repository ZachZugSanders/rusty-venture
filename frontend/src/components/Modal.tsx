import { useEffect } from 'react'
import { createPortal } from 'react-dom'
import styles from './Modal.module.css'

interface ModalProps {
    title: string
    onClose: () => void
    children: React.ReactNode
    footer?: React.ReactNode
    size?: 'md' | 'lg'
}

export default function Modal({ title, onClose, children, footer, size = 'md' }: ModalProps) {
    useEffect(() => {
        const onKey = (e: KeyboardEvent) => { if (e.key === 'Escape') onClose() }
        document.addEventListener('keydown', onKey)
        return () => document.removeEventListener('keydown', onKey)
    }, [onClose])

    return createPortal(
        <div className={styles.overlay} onClick={onClose}>
            <div
                className={`${styles.dialog} ${size === 'lg' ? styles.dialogLg : ''}`}
                onClick={e => e.stopPropagation()}
            >
                <div className={styles.header}>
                    <h2 className={styles.title}>{title}</h2>
                    <button className={styles.closeBtn} onClick={onClose} aria-label="Close">✕</button>
                </div>
                <div className={styles.body}>{children}</div>
                {footer && <div className={styles.footer}>{footer}</div>}
            </div>
        </div>,
        document.body
    )
}
