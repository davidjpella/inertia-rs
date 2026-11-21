import { Head, useForm } from '@inertiajs/react'

export default function UsersCreate({ roles }) {
  const form = useForm({ name: '', email: '' })

  const submit = (event) => {
    event.preventDefault()
    form.post('/users')
  }

  return (
    <>
      <Head title="New user" />
      <h1>New user</h1>
      <form onSubmit={submit} noValidate>
        <label>
          Name
          <input name="name" value={form.data.name} onChange={(event) => form.setData('name', event.target.value)} />
        </label>
        {form.errors.name && (
          <p className="error" data-testid="error-name">
            {form.errors.name}
          </p>
        )}
        <label>
          Email
          <input name="email" value={form.data.email} onChange={(event) => form.setData('email', event.target.value)} />
        </label>
        {form.errors.email && (
          <p className="error" data-testid="error-email">
            {form.errors.email}
          </p>
        )}
        <p>Roles: {roles.join(', ')}</p>
        <button type="submit" disabled={form.processing} data-testid="submit">
          Create
        </button>
      </form>
    </>
  )
}
