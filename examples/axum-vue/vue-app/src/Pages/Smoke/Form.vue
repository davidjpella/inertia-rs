<script setup>
import { Link, useForm } from '@inertiajs/vue3'

const form = useForm({ name: '' })
</script>

<template>
  <h1>Form</h1>
  <form novalidate @submit.prevent="form.post('/smoke/form')">
    <label>Name <input v-model="form.name" name="name" /></label>
    <p v-if="form.errors.name" data-testid="error-name">{{ form.errors.name }}</p>
    <button type="submit" data-testid="submit">Save</button>
  </form>
  <Link href="/" data-testid="home-link">Home</Link>
</template>
