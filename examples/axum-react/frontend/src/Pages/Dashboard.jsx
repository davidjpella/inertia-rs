import { Deferred, Head, router, usePage, useProp } from '@inertiajs/react'

function Stats() {
  const { stats } = usePage().props
  return (
    <p data-testid="stats">
      {stats.visits} visits, {stats.conversion}% conversion
    </p>
  )
}

export default function Dashboard({ greeting, countries, audit, lastOrderAt }) {
  const page = usePage()
  const orders = useProp('ordersCount')

  const placeOrder = (mode) =>
    router.post('/orders', { mode }, { preserveScroll: true, preserveState: true })

  return (
    <>
      <Head title="Dashboard" />
      <h1 data-testid="greeting">{greeting}</h1>

      <section>
        <h2>Deferred props</h2>
        <Deferred data="stats" fallback={<p data-testid="stats-loading">Loading stats…</p>}>
          <Stats />
        </Deferred>
        <Deferred
          data="report"
          fallback={<p data-testid="report-loading">Loading report…</p>}
          rescue={<p data-testid="report-failed">Report unavailable</p>}
        >
          <p data-testid="report">Report loaded</p>
        </Deferred>
      </section>

      <section>
        <h2>Once and optional props</h2>
        <ul data-testid="countries">
          {countries?.map((country) => (
            <li key={country}>{country}</li>
          ))}
        </ul>
        <button type="button" data-testid="load-audit" onClick={() => router.reload({ only: ['audit'] })}>
          Load audit log
        </button>
        {audit && (
          <ul data-testid="audit">
            {audit.map((entry) => (
              <li key={entry}>{entry}</li>
            ))}
          </ul>
        )}
      </section>

      <section>
        <h2>Live props</h2>
        <p>
          Orders: <span data-testid="orders-count">{orders.value}</span>
          {orders.loading && <span data-testid="orders-loading"> (refreshing)</span>}
        </p>
        <p>
          Last refresh: <span data-testid="last-order-at">{lastOrderAt}</span>
        </p>
        <button type="button" data-testid="place-order" onClick={() => placeOrder('props')}>
          Place order (broadcast value)
        </button>
        <button type="button" data-testid="place-order-reload" onClick={() => placeOrder('reload')}>
          Place order (broadcast reload)
        </button>
      </section>

      <section>
        <h2>Big integers</h2>
        <p data-testid="big-id">
          {typeof page.flash?.bigId}:{String(page.flash?.bigId ?? '')}
        </p>
      </section>
    </>
  )
}
