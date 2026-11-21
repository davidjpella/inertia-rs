import { Head, Link } from '@inertiajs/react'

export default function Error({ status }) {
  return (
    <>
      <Head title={`Error ${status}`} />
      <h1 data-testid="error-status">{status}</h1>
      <Link href="/">Back to the dashboard</Link>
    </>
  )
}
