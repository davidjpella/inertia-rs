import { createInertiaApp } from '@inertiajs/react'
import Layout from './Layout'
import { sseTransport } from './liveTransport'
import './app.css'

createInertiaApp({
  pages: './Pages',
  title: (title) => (title ? `${title} · Inertia Rust` : 'Inertia Rust'),
  layout: () => Layout,
  // Live props only run in the browser.
  live: typeof window === 'undefined' ? undefined : sseTransport({ url: '/inertia/live' }),
})
