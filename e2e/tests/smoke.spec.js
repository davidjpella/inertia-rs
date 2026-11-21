// Smoke tests for the official Vue and Svelte adapters (3.11.0) against
// examples/smoke-server. They cover the server paths every adapter relies on.
import { expect, test } from '@playwright/test'
import { expectNewDocument, expectSameDocument, markWindow, nextInertiaResponse } from './helpers.js'

test('first visit renders and deferred props load', async ({ page }) => {
  await page.goto('/')
  await expect(page.getByTestId('app-name')).toHaveText('Inertia smoke')
  await expect(page.getByTestId('message')).toHaveText('Hello from the smoke server')
  await expect(page.getByTestId('stats')).toHaveText('42 visits')
})

test('optional props load through partial reloads', async ({ page }) => {
  await page.goto('/')
  await expect(page.getByTestId('debug')).toHaveCount(0)
  const reload = nextInertiaResponse(page, '/')
  await page.getByTestId('load-debug').click()
  const body = await (await reload).json()
  expect(Object.keys(body.props).sort()).toEqual(['debug', 'errors'])
  await expect(page.getByTestId('debug')).toHaveText('loaded on demand')
  await expect(page.getByTestId('message')).toBeVisible()
})

test('validation errors and flash survive redirects', async ({ page }) => {
  await page.goto('/')
  await markWindow(page)
  await page.getByTestId('form-link').click()
  await expect(page).toHaveURL(/\/smoke\/form$/)

  await page.getByTestId('submit').click()
  await expect(page.getByTestId('error-name')).toHaveText('The name field is required.')

  await page.getByLabel('Name').fill('Ada')
  await page.getByTestId('submit').click()
  await expect(page).toHaveURL(/\/$/)
  await expect(page.getByTestId('flash')).toHaveText('Saved Ada')
  await expectSameDocument(page)
})

test('asset version changes force a full reload', async ({ page, request }) => {
  await page.goto('/')
  await markWindow(page)
  expect((await request.post('/__e2e/bump-version')).status()).toBe(204)
  await page.getByTestId('form-link').click()
  await expect(page).toHaveURL(/\/smoke\/form$/)
  await expect(page.getByTestId('submit')).toBeVisible()
  await expectNewDocument(page)
})
