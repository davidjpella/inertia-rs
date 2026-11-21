import { Deferred, Head, router } from '@inertiajs/react'

export default function UsersShow({ user, activity }) {
  return (
    <>
      <Head title={user.name} />
      <h1 data-testid="user-name">{user.name}</h1>
      <p>{user.email}</p>
      <Deferred data="activity" fallback={<p>Loading activity…</p>}>
        <ul data-testid="activity">{activity?.map((entry) => <li key={entry}>{entry}</li>)}</ul>
      </Deferred>
      <button type="button" data-testid="delete" onClick={() => router.delete(`/users/${user.id}`)}>
        Delete
      </button>
    </>
  )
}
