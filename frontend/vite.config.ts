import { defineConfig } from 'vite'
import react from '@vitejs/plugin-react'

export default defineConfig({
    plugins: [react()],
    server: {
        proxy: {
            '/analyze': 'http://localhost:8080',
            '/scans': 'http://localhost:8080',
            '/repos': 'http://localhost:8080',
            '/health': 'http://localhost:8080',
        },
    },
})
