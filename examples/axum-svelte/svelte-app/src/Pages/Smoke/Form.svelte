<script>
  import { Link, useForm } from '@inertiajs/svelte'

  const form = useForm({ name: '' })

  function submit(event) {
    event.preventDefault()
    form.post('/smoke/form')
  }
</script>

<h1>Form</h1>
<form onsubmit={submit} novalidate>
  <label>Name <input name="name" bind:value={form.name} /></label>
  {#if form.errors.name}
    <p data-testid="error-name">{form.errors.name}</p>
  {/if}
  <button type="submit" data-testid="submit">Save</button>
</form>
<Link href="/" data-testid="home-link">Home</Link>
