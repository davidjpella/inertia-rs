import { Head, useForm } from '@inertiajs/react'

// Rendered as a layer over the user page by clients that support the
// experimental Layers protocol, and as an ordinary page otherwise.
export default function UsersEdit({ user }) {
  const form = useForm({ name: user.name })

  return (
    <>
      <Head title={`Edit ${user.name}`} />
      <h2 data-testid="edit-heading">Edit {user.name}</h2>
      <form
        noValidate
        onSubmit={(event) => {
          event.preventDefault()
          form.put(`/users/${user.id}/edit`)
        }}
      >
        <label>
          Name
          <input name="name" value={form.data.name} onChange={(event) => form.setData('name', event.target.value)} />
        </label>
        {form.errors.name && (
          <p className="error" data-testid="edit-error-name">
            {form.errors.name}
          </p>
        )}
        <button type="submit" data-testid="edit-save">
          Save
        </button>
      </form>
    </>
  )
}
