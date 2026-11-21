<script>
  import { Deferred, Link, page, router } from '@inertiajs/svelte'

  let { appName, message, debug } = $props()
</script>

<h1 data-testid="app-name">{appName}</h1>
<p data-testid="message">{message}</p>
{#if page.flash?.message}
  <p role="status" data-testid="flash">{page.flash.message}</p>
{/if}
<Deferred data="stats">
  {#snippet fallback()}
    <p data-testid="stats-loading">Loading…</p>
  {/snippet}
  <p data-testid="stats">{page.props.stats.visits} visits</p>
</Deferred>
<button type="button" data-testid="load-debug" onclick={() => router.reload({ only: ['debug'] })}>Load debug</button>
{#if debug}
  <p data-testid="debug">{debug}</p>
{/if}
<Link href="/smoke/form" data-testid="form-link">Form</Link>
