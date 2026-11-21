import { defineConfig, devices } from '@playwright/test'

const examples = new URL('../examples/', import.meta.url).pathname
const frameworks = (process.env.E2E_FRAMEWORKS ?? 'react,vue,svelte').split(',')

const apps = {
  react: { port: 34100, example: 'axum-react', ssrPort: 13714 },
  vue: { port: 34101, example: 'axum-vue' },
  svelte: { port: 34102, example: 'axum-svelte' },
  // Experimental: requires scripts/build-layers-client.sh and a build of
  // examples/axum-react/frontend-layers. Not part of the default run.
  layers: {
    port: 34103,
    example: 'axum-react',
    ssrPort: 13715,
    ssrDir: 'frontend-layers',
    cargoArgs: '--features layers',
    env: { FRONTEND_BUILD_DIR: 'public/build-layers' },
  },
}

const servers = frameworks.flatMap((framework) => {
  const app = apps[framework]
  const entries = []

  if (app.ssrPort) {
    entries.push({
      command: 'node bootstrap/ssr/app.js',
      cwd: `${examples}${app.example}/${app.ssrDir ?? 'frontend'}`,
      url: `http://127.0.0.1:${app.ssrPort}/health`,
      reuseExistingServer: !process.env.CI,
    })
  }

  entries.push({
    command: `cargo run --quiet -p ${app.example} ${app.cargoArgs ?? ''}`,
    cwd: examples,
    url: `http://127.0.0.1:${app.port}/`,
    timeout: 300_000,
    reuseExistingServer: !process.env.CI,
    env: {
      ADDR: `127.0.0.1:${app.port}`,
      CARGO_TARGET_DIR: '../target',
      INERTIA_E2E: '1',
      ...(app.env ?? {}),
      ...(app.ssrPort ? { INERTIA_SSR_URL: `http://127.0.0.1:${app.ssrPort}` } : {}),
    },
  })

  return entries
})

export default defineConfig({
  testDir: './tests',
  // The example apps keep in-memory state, so tests run one at a time.
  workers: 1,
  retries: process.env.CI ? 1 : 0,
  reporter: process.env.CI ? [['list'], ['html', { open: 'never' }]] : 'list',
  use: { ...devices['Desktop Chrome'], trace: 'retain-on-failure' },
  projects: frameworks.map((framework) => ({
    name: framework,
    testMatch: { react: /react\.spec\.js/, layers: /layers\.spec\.js/ }[framework] ?? /smoke\.spec\.js/,
    use: { baseURL: `http://127.0.0.1:${apps[framework].port}` },
  })),
  webServer: servers,
})
