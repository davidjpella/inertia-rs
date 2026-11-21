import inertia from '@inertiajs/vite'
import react from '@vitejs/plugin-react'
import { defineConfig } from 'vite'

export default defineConfig(({ isSsrBuild }) => ({
  plugins: [inertia({ ssr: { entry: 'src/app.jsx', port: 13714 } }), react()],
  // HTML is served by Axum, so the dev server must accept cross-origin
  // module requests and emit absolute asset URLs.
  server: {
    port: 5173,
    strictPort: true,
    cors: true,
    origin: 'http://localhost:5173',
  },
  build: isSsrBuild
    ? { outDir: 'bootstrap/ssr', emptyOutDir: true }
    : {
        outDir: '../public/build',
        emptyOutDir: true,
        manifest: true,
        rollupOptions: { input: 'src/app.jsx' },
      },
}))
