import { Head, InfiniteScroll, Link, usePage } from '@inertiajs/react'

export default function UsersIndex({ users, total }) {
  const page = usePage()

  return (
    <>
      <Head title="Users" />
      <h1>
        Users (<span data-testid="total">{total}</span>)
      </h1>
      {page.flash?.newUserId && <p data-testid="new-user-id">New user #{page.flash.newUserId}</p>}
      <InfiniteScroll
        data="users"
        manual
        next={({ fetch, hasMore, loading }) =>
          hasMore && (
            <button type="button" data-testid="load-more" disabled={loading} onClick={fetch}>
              {loading ? 'Loading…' : 'Load more'}
            </button>
          )
        }
      >
        <ul className="users" data-testid="users">
          {users.data.map((user) => (
            <li key={user.id} data-testid="user-row">
              <Link href={`/users/${user.id}`}>{user.name}</Link> <small>{user.email}</small>
            </li>
          ))}
        </ul>
      </InfiniteScroll>
      <p id="bottom" data-testid="bottom">
        End of list
      </p>
    </>
  )
}
