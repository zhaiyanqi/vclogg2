import { fileURLToPath, URL } from 'node:url'
import { defineConfig } from 'vite'
import vue from '@vitejs/plugin-vue'

export default defineConfig({
  root: fileURLToPath(new URL('.', import.meta.url)),
  base: '/admin/',
  plugins: [vue()],
  resolve: {
    alias: { '@': fileURLToPath(new URL('./src', import.meta.url)) }
  },
  build: {
    outDir: process.env.VCLOGG_ADMIN_OUT_DIR || fileURLToPath(new URL('../internal/app/admin', import.meta.url)),
    emptyOutDir: true,
    sourcemap: false,
    cssCodeSplit: true
  },
  server: {
    port: 5178,
    proxy: { '/admin/api': 'http://127.0.0.1:8787' }
  }
})
