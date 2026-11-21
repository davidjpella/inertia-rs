// Builds the shared sources (`src` is a symlink to ../frontend/src) against
// the Layers client installed in this directory. `preserveSymlinks` makes
// imports resolve from here rather than from ../frontend/node_modules.
import inertia from '@inertiajs/vite'
import react from '@vitejs/plugin-react'
import { defineConfig } from 'vite'

export default defineConfig(({ isSsrBuild }) => ({
  plugins: [inertia({ ssr: { entry: 'src/app.jsx', port: 13715 } }), react()],
  resolve: { preserveSymlinks: true },
  build: isSsrBuild
    ? { outDir: 'bootstrap/ssr', emptyOutDir: true }
    : {
        outDir: '../public/build-layers',
        emptyOutDir: true,
        manifest: true,
        rollupOptions: { input: 'src/app.jsx' },
      },
}))
