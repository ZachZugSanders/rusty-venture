/// <reference types="vitest" />
import { defineConfig } from 'vite'
import react from '@vitejs/plugin-react'

export default defineConfig({
    plugins: [react()], test: {
        globals: true,
        environment: 'node',
        include: ['src/**/*.test.ts'],
    },
    server: {
        proxy: {
            '/analyze': 'http://127.0.0.1:3002',
            '/scans': 'http://127.0.0.1:3002',
            '/repos': 'http://127.0.0.1:3002',
            '/health': 'http://127.0.0.1:3002',
        },
    },
})
