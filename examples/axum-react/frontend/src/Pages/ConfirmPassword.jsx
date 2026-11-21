import { Head, router } from '@inertiajs/react'

export default function ConfirmPassword() {
  return (
    <>
      <Head title="Confirm password" />
      <h1 data-testid="confirm-heading">Confirm your password</h1>
      <button type="button" data-testid="confirm" onClick={() => router.post('/confirm-password')}>
        Confirm
      </button>
    </>
  )
}
