import { expect } from '@playwright/test'

/** Marks the window so a later full page load can be detected. */
export async function markWindow(page) {
  await page.evaluate(() => {
    window.__inertiaE2eMarker = true
  })
}

export async function expectSameDocument(page) {
  expect(await page.evaluate(() => window.__inertiaE2eMarker === true)).toBe(true)
}

export async function expectNewDocument(page) {
  expect(await page.evaluate(() => window.__inertiaE2eMarker === true)).toBe(false)
}

/** Collects console errors and uncaught exceptions. */
export function collectErrors(page) {
  const errors = []
  page.on('console', (message) => {
    if (message.type() === 'error') {
      errors.push(message.text())
    }
  })
  page.on('pageerror', (error) => errors.push(error.message))
  return errors
}

/** Waits for the next Inertia XHR whose URL path matches `path`. */
export function nextInertiaResponse(page, path, predicate = () => true) {
  return page.waitForResponse(
    (response) =>
      new URL(response.url()).pathname === path &&
      response.request().headers()['x-inertia'] === 'true' &&
      predicate(response),
  )
}

/**
 * Loads `url` and waits until React has hydrated the server-rendered page,
 * so interactions reach React's event handlers instead of the static markup.
 */
export async function gotoHydrated(page, url) {
  const response = await page.goto(url)
  await page.waitForFunction(() => {
    const root = document.getElementById('app')
    return Boolean(root && root.querySelector('*') && Object.keys(root.firstElementChild ?? {}).some((key) => key.startsWith('__react')))
  })
  return response
}
