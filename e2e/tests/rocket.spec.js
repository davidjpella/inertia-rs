// Smoke tests for the Rocket integration (examples/rocket-svelte) with the
// official @inertiajs/svelte 3.11.0 client.
import { expect, test } from '@playwright/test'
import { nextInertiaResponse } from './helpers.js'

test('first visit renders shared props and loads deferred props', async ({ page }) => {
  const deferred = nextInertiaResponse(page, '/hello', (response) =>
    (response.request().headers()['x-inertia-partial-data'] ?? '').includes('stats'),
  )
  await page.goto('/hello')
  await expect(page.getByRole('heading', { name: 'Hello world' })).toBeVisible()
  await expect(page.getByText('Ada Lovelace')).toBeVisible()

  const body = await (await deferred).json()
  expect(body.props.stats.adapter).toBe('Rocket')
  await expect(page.getByText('Generated at')).toBeVisible()
})

test('optional props load through a partial reload', async ({ page }) => {
  await page.goto('/hello')
  await expect(page.getByText('Not requested yet')).toBeVisible()

  const reload = nextInertiaResponse(page, '/hello', (response) =>
    response.request().headers()['x-inertia-partial-data'] === 'debug',
  )
  await page.getByRole('button', { name: 'Load debug' }).click()
  const body = await (await reload).json()
  expect(Object.keys(body.props).sort()).toEqual(['debug', 'errors'])
  await expect(page.getByText('X-Inertia-Partial-Data: debug')).toBeVisible()
})
