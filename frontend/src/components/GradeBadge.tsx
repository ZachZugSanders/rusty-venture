import styles from './GradeBadge.module.css'

function gradeColour(grade: string): 'green' | 'yellow' | 'red' {
    switch (grade.toUpperCase()) {
        case 'DIAMOND':
        case 'PLATINUM':
        case 'EXEMPLARY':
        case 'ESTABLISHED':
            return 'green'
        case 'GOLD':
        case 'SILVER':
        case 'DEVELOPING':
        case 'EMERGING':
            return 'yellow'
        default:
            return 'red'
    }
}

interface GradeBadgeProps {
    grade: string
    'data-testid'?: string
}

export default function GradeBadge({ grade, 'data-testid': testId }: GradeBadgeProps) {
    const colour = gradeColour(grade)
    return (
        <span className={styles[colour]} data-testid={testId}>
            {grade}
        </span>
    )
}
