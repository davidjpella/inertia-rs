// EXPERIMENTAL: Layers (inertiajs/inertia#3236, client commit e57a7774).
// Runs examples/axum-react with `--features layers` against the client built
// by scripts/build-layers-client.sh. The upstream contract can change.
import { expect, test } from '@playwright/test'
import { expectSameDocument, markWindow, nextInertiaResponse } from './helpers.js'

test('a layer opens over its base and the browser back button closes it', async ({ page }) => {
  await page.goto('/users/3')
  await markWindow(page)

  const opened = nextInertiaResponse(page, '/users/3/edit')
  await page.getByTestId('edit-link').click()
  const body = await (await opened).json()
  expect(body.layer).toEqual({ base: '/users/3' })

  const dialog = page.getByRole('dialog')
  await expect(dialog.getByTestId('edit-heading')).toHaveText('Edit User 3')
  await expect(page.getByTestId('user-name')).toHaveText('User 3')
  await expect(page).toHaveURL(/\/users\/3\/edit$/)

  await page.goBack()
  await expect(page).toHaveURL(/\/users\/3$/)
  await expect(page.getByRole('dialog')).toHaveCount(0)
  await expect(page.getByTestId('user-name')).toHaveText('User 3')
  await expectSameDocument(page)
})

test('a cold visit renders the layer and fetches its base beneath', async ({ page, request }) => {
  const html = await (await request.get('/users/5/edit')).text()
  const script = html.match(/<script data-page="app" type="application\/json">(.*?)<\/script>/s)
  expect(JSON.parse(script[1]).layer).toEqual({ base: '/users/5' })
  // The SSR server renders the cold-opened layer inside its dialog shell.
  expect(html).toContain('data-server-rendered="true"')
  expect(html).toMatch(/<dialog open=""[^>]*data-layer-type="routed"/)

  const base = page.waitForResponse((response) => new URL(response.url()).pathname === '/users/5')
  await page.goto('/users/5/edit')
  await base
  await expect(page.getByRole('dialog').getByTestId('edit-heading')).toHaveText('Edit User 5')
  await expect(page.getByTestId('user-name')).toHaveText('User 5')
})

test('validation errors stay in the layer and close refreshes the base with flash', async ({ page }) => {
  await page.goto('/users/6')
  await page.getByTestId('edit-link').click()
  const dialog = page.getByRole('dialog')
  await expect(dialog.getByTestId('edit-heading')).toBeVisible()

  await dialog.getByLabel('Name').fill('')
  await dialog.getByTestId('edit-save').click()
  await expect(dialog.getByTestId('edit-error-name')).toHaveText('The name field is required.')

  const flashes = []
  page.on('response', async (response) => {
    if (new URL(response.url()).pathname !== '/users/6') return
    const body = await response.json().catch(() => null)
    if (body?.flash?.message) flashes.push(body.flash.message)
  })
  const close = page.waitForResponse((response) => response.request().method() === 'PUT')
  await dialog.getByLabel('Name').fill('Renamed Six')
  await dialog.getByTestId('edit-save').click()
  expect(await (await close).json()).toMatchObject({ component: '', close: true })

  await expect(page.getByRole('dialog')).toHaveCount(0)
  await expect(page.getByTestId('user-name')).toHaveText('Renamed Six')
  // The close response leaves the flash in the session; the next response
  // for the page beneath delivers it exactly once.
  await expect.poll(() => flashes).toEqual(['User updated.'])
  // Known limitation (client commit e57a7774): the client refreshes the
  // base's deferred props first and ignores flash on deferred-prop requests,
  // so the message is not displayed when the base page has deferred props.
  // See docs/protocol-support.md.
})

test('an interstitial detour opens the pending layer where it was requested', async ({ page }) => {
  await page.goto('/users/4')
  const detour = page.waitForResponse(
    (response) => new URL(response.url()).pathname === '/confirm-password' && response.request().method() === 'GET',
  )
  await page.getByTestId('secure-edit-link').click()
  expect((await (await detour).json()).interstitial).toBe(true)
  await expect(page.getByTestId('confirm-heading')).toBeVisible()

  const layer = page.waitForResponse(
    (response) => new URL(response.url()).pathname === '/users/4/secure-edit' && response.status() === 200,
  )
  await page.getByTestId('confirm').click()
  const body = await (await layer).json()
  expect(body.layer).toEqual({ base: '/users/4' })

  await expect(page.getByRole('dialog').getByTestId('edit-heading')).toHaveText('Edit User 4')
  await expect(page.getByTestId('user-name')).toHaveText('User 4')
})
