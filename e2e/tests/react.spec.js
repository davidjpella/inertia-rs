// End-to-end tests for examples/axum-react against the official
// @inertiajs/react 3.11.0 client. See docs/compatibility-target.md.
import { expect, test } from '@playwright/test'
import {
  collectErrors,
  expectNewDocument,
  expectSameDocument,
  markWindow,
  nextInertiaResponse,
} from './helpers.js'

test.describe('first visit and SSR', () => {
  test('server-renders the first visit and hydrates without errors', async ({ page, request }) => {
    const html = await (await request.get('/')).text()
    expect(html).toContain('data-server-rendered="true"')
    expect(html).toContain('<title data-inertia="">Dashboard · Inertia Rust</title>')
    expect(html).toContain('<script data-page="app" type="application/json">')

    const errors = collectErrors(page)
    await page.goto('/')
    await expect(page.getByTestId('greeting')).toHaveText('Hello from Axum')
    await expect(page).toHaveTitle('Dashboard · Inertia Rust')
    // Hydration attaches handlers: an Inertia visit works without a reload.
    await markWindow(page)
    await page.getByRole('link', { name: 'Users', exact: true }).click()
    await expect(page).toHaveURL(/\/users$/)
    await expectSameDocument(page)
    expect(errors.filter((error) => /hydrat/i.test(error))).toEqual([])
  })

  test('shared props are available on every page', async ({ page }) => {
    await page.goto('/users')
    await expect(page.getByTestId('app-name')).toHaveText('Inertia Rust')
    await expect(page.getByTestId('current-user')).toHaveText('Ada Lovelace')
  })
})

test.describe('navigation', () => {
  test('links swap components without a full reload and back works', async ({ page }) => {
    await page.goto('/')
    await markWindow(page)

    const json = nextInertiaResponse(page, '/users')
    await page.getByRole('link', { name: 'Users', exact: true }).click()
    const response = await json
    expect(response.headers()['x-inertia']).toBe('true')
    expect(response.headers()['vary']).toContain('X-Inertia')
    await expect(page.getByTestId('user-row').first()).toBeVisible()
    await expectSameDocument(page)

    await page.goBack()
    await expect(page.getByTestId('greeting')).toBeVisible()
    await expectSameDocument(page)
  })

  test('fragment redirects become Inertia visits that keep the fragment', async ({ page }) => {
    await page.goto('/')
    await markWindow(page)
    await page.getByTestId('fragment-link').click()
    await expect(page).toHaveURL(/\/users#bottom$/)
    await expect(page.getByTestId('bottom')).toBeVisible()
    await expectSameDocument(page)
  })

  test('external locations trigger a full window.location visit', async ({ page }) => {
    await page.route('https://inertiajs.com/**', (route) =>
      route.fulfill({ contentType: 'text/html', body: '<h1>External</h1>' }),
    )
    await page.goto('/')
    await page.getByTestId('external-link').click()
    await expect(page).toHaveURL('https://inertiajs.com/')
    await expect(page.getByRole('heading', { name: 'External' })).toBeVisible()
  })

  test('unknown routes render the Inertia error page with a 404', async ({ page }) => {
    const response = await page.goto('/does-not-exist')
    expect(response.status()).toBe(404)
    await expect(page.getByTestId('error-status')).toHaveText('404')
  })
})

test.describe('props', () => {
  test('deferred props load in follow-up partial reloads per group', async ({ page }) => {
    const groups = []
    page.on('request', (request) => {
      const only = request.headers()['x-inertia-partial-data']
      if (only) groups.push(only)
    })

    await page.goto('/')
    await expect(page.getByTestId('stats')).toHaveText('1234 visits, 3.2% conversion')
    await expect(page.getByTestId('report')).toHaveText('Report loaded')
    expect(groups.sort()).toEqual(['report', 'stats'])
  })

  test('rescued deferred props render the rescue slot', async ({ page }) => {
    const reload = nextInertiaResponse(page, '/', (response) =>
      (response.request().headers()['x-inertia-partial-data'] ?? '').includes('report'),
    )
    await page.goto('/?fail=1')
    const response = await reload
    expect(response.status()).toBe(200)
    expect((await response.json()).rescuedProps).toEqual(['report'])
    await expect(page.getByTestId('report-failed')).toBeVisible()
    await expect(page.getByTestId('stats')).toBeVisible()
  })

  test('optional props load only when requested', async ({ page }) => {
    await page.goto('/')
    await expect(page.getByTestId('audit')).toHaveCount(0)

    const reload = nextInertiaResponse(page, '/')
    await page.getByTestId('load-audit').click()
    const body = await (await reload).json()
    expect(Object.keys(body.props).sort()).toEqual(['audit', 'errors'])
    await expect(page.getByTestId('audit')).toContainText('Viewed dashboard')
    await expect(page.getByTestId('greeting')).toHaveText('Hello from Axum')
  })

  test('shared once props are remembered across visits', async ({ page }) => {
    await page.goto('/')
    await expect(page.getByTestId('countries')).toContainText('Japan')
    await page.getByRole('link', { name: 'Users', exact: true }).click()
    await expect(page).toHaveURL(/\/users$/)

    const visit = nextInertiaResponse(page, '/', (response) => !response.request().headers()['x-inertia-partial-data'])
    await page.getByRole('link', { name: 'Dashboard' }).click()
    const response = await visit
    expect(response.request().headers()['x-inertia-except-once-props']).toContain('countries')
    const body = await response.json()
    expect(body.props.countries).toBeUndefined()
    expect(body.onceProps.countries.prop).toBe('countries')
    await expect(page.getByTestId('countries')).toContainText('Japan')
  })

  test('big integers survive as BigInt values', async ({ page }) => {
    await page.goto('/')
    await expect(page.getByTestId('big-id')).toHaveText('bigint:900719925474099988')
  })
})

test.describe('forms, validation, and flash', () => {
  test('validation errors redirect back and are shown once', async ({ page }) => {
    await page.goto('/users/create')
    await markWindow(page)

    const post = page.waitForResponse((response) => response.request().method() === 'POST')
    await page.getByTestId('submit').click()
    expect((await post).status()).toBe(303)
    await expect(page.getByTestId('error-name')).toHaveText('The name field is required.')
    await expect(page.getByTestId('error-email')).toHaveText('The email must be a valid email address.')
    await expect(page).toHaveURL(/\/users\/create$/)
    await expectSameDocument(page)

    await page.reload()
    await expect(page.getByTestId('error-name')).toHaveCount(0)
  })

  test('successful submissions flash once and survive the redirect', async ({ page }) => {
    await page.goto('/users/create')
    await page.getByLabel('Name').fill('Grace Hopper')
    await page.getByLabel('Email').fill('grace@example.com')
    await page.getByTestId('submit').click()

    await expect(page).toHaveURL(/\/users$/)
    await expect(page.getByTestId('flash')).toHaveText('User created.')
    await expect(page.getByTestId('new-user-id')).toBeVisible()
    await expect(page.getByTestId('user-row').first()).toContainText('Grace Hopper')

    await page.getByRole('link', { name: 'Dashboard' }).click()
    await expect(page.getByTestId('greeting')).toBeVisible()
    await expect(page.getByTestId('flash')).toHaveCount(0)

    // Flash data is not restored from history.
    await page.goBack()
    await expect(page).toHaveURL(/\/users$/)
    await expect(page.getByTestId('flash')).toHaveCount(0)
  })

  test('delete redirects with 303 and flashes', async ({ page }) => {
    await page.goto('/users/2')
    await expect(page.getByTestId('activity')).toContainText('User 2 signed in')
    const destroy = page.waitForResponse((response) => response.request().method() === 'DELETE')
    await page.getByTestId('delete').click()
    expect((await destroy).status()).toBe(303)
    await expect(page).toHaveURL(/\/users$/)
    await expect(page.getByTestId('flash')).toHaveText('User 2 deleted.')
  })
})

test.describe('infinite scroll', () => {
  test('loads further pages and merges them', async ({ page }) => {
    await page.goto('/users')
    await expect(page.getByTestId('user-row')).toHaveCount(15)

    const next = nextInertiaResponse(page, '/users')
    await page.getByTestId('load-more').click()
    const response = await next
    expect(new URL(response.url()).searchParams.get('page')).toBe('2')
    expect(response.request().headers()['x-inertia-infinite-scroll-merge-intent']).toBe('append')
    const body = await response.json()
    expect(body.mergeProps).toEqual(['users.data'])
    expect(body.scrollProps.users.currentPage).toBe(2)
    await expect(page.getByTestId('user-row')).toHaveCount(30)
  })
})

test.describe('asset versioning', () => {
  test('a version change forces a full reload on the next visit', async ({ page, request }) => {
    await page.goto('/')
    await markWindow(page)

    const conflict = page.waitForResponse((response) => response.status() === 409)
    expect((await request.post('/__e2e/bump-version')).status()).toBe(204)
    await page.getByRole('link', { name: 'Users', exact: true }).click()
    const response = await conflict
    expect(response.headers()['x-inertia-location']).toBe('/users')
    expect(response.headers()['x-inertia-version']).toBeTruthy()

    await expect(page.getByTestId('user-row').first()).toBeVisible()
    await expectNewDocument(page)
  })
})

test.describe('live props', () => {
  test('broadcast values update other clients without a request', async ({ browser }) => {
    const watcher = await browser.newPage()
    const actor = await browser.newPage()
    await watcher.goto('/')
    await actor.goto('/')
    const before = Number(await watcher.getByTestId('orders-count').textContent())

    const reloads = []
    watcher.on('request', (request) => {
      if (request.headers()['x-inertia-partial-data']?.includes('ordersCount')) reloads.push(request)
    })

    await actor.getByTestId('place-order').click()
    await expect(actor.getByTestId('flash')).toContainText('placed')
    await expect(watcher.getByTestId('orders-count')).toHaveText(String(before + 1))
    expect(reloads).toEqual([])
    await watcher.close()
    await actor.close()
  })

  test('reload events refresh other clients through partial reloads and skip the sender', async ({
    browser,
  }) => {
    const watcher = await browser.newPage()
    const actor = await browser.newPage()
    await watcher.goto('/')
    await actor.goto('/')
    const initial = await watcher.getByTestId('last-order-at').textContent()

    let actorReloads = 0
    actor.on('request', (request) => {
      if (request.headers()['x-inertia-partial-data']?.includes('lastOrderAt')) actorReloads += 1
    })
    const refresh = nextInertiaResponse(watcher, '/', (response) =>
      (response.request().headers()['x-inertia-partial-data'] ?? '').includes('lastOrderAt'),
    )

    await actor.getByTestId('place-order-reload').click()
    const response = await refresh
    expect(response.request().headers()['x-socket-id']).toBeTruthy()
    const body = await response.json()
    expect(Object.keys(body.liveProps)).toEqual(['lastOrderAt'])
    await expect(watcher.getByTestId('last-order-at')).not.toHaveText(initial)

    await actor.waitForTimeout(1500)
    expect(actorReloads).toBe(0)
    await watcher.close()
    await actor.close()
  })
})
