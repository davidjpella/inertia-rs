<script setup>
import { Deferred, Link, router, usePage } from '@inertiajs/vue3'

defineProps({ appName: String, message: String, stats: Object, debug: String })
const page = usePage()
</script>

<template>
  <h1 data-testid="app-name">{{ appName }}</h1>
  <p data-testid="message">{{ message }}</p>
  <p v-if="page.flash?.message" role="status" data-testid="flash">{{ page.flash.message }}</p>
  <Deferred data="stats">
    <template #fallback><p data-testid="stats-loading">Loading…</p></template>
    <p data-testid="stats">{{ stats.visits }} visits</p>
  </Deferred>
  <button type="button" data-testid="load-debug" @click="router.reload({ only: ['debug'] })">Load debug</button>
  <p v-if="debug" data-testid="debug">{{ debug }}</p>
  <Link href="/smoke/form" data-testid="form-link">Form</Link>
</template>
