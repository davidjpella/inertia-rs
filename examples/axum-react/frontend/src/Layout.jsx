import { Link, usePage } from '@inertiajs/react'

export default function Layout({ children }) {
  const page = usePage()
  const { appName, auth } = page.props

  return (
    <div className="shell">
      <header>
        <strong data-testid="app-name">{appName}</strong>
        <nav>
          <Link href="/" prefetch>
            Dashboard
          </Link>
          <Link href="/users" prefetch>
            Users
          </Link>
          <Link href="/users/create">New user</Link>
          <Link href="/fragment" data-testid="fragment-link">
            Jump to bottom
          </Link>
          <Link href="/external" data-testid="external-link">
            Inertia docs
          </Link>
        </nav>
        <span data-testid="current-user">{auth?.user?.name}</span>
      </header>
      {page.flash?.message && (
        <p role="status" data-testid="flash">
          {page.flash.message}
        </p>
      )}
      <main>{children}</main>
    </div>
  )
}
