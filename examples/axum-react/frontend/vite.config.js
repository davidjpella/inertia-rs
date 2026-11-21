import inertia from '@inertiajs/vite'
import react from '@vitejs/plugin-react'
import { defineConfig } from 'vite'

const port = Number(process.env.VITE_PORT ?? 5173)

export default defineConfig(({ isSsrBuild }) => ({
  plugins: [inertia({ ssr: { entry: 'src/app.jsx', port: 13714 } }), react()],
  // HTML is served by Axum, so the dev server must accept cross-origin
  // module requests and emit absolute asset URLs.
  server: {
    port,
    strictPort: true,
    cors: true,
    origin: `http://localhost:${port}`,
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
