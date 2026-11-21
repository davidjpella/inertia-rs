/**
 * A LiveTransport for inertia_rs's Server-Sent Events broadcaster
 * (`inertia_rs::axum::live::Broadcaster::sse`).
 *
 * Implements the official `LiveTransport` interface from @inertiajs/core:
 * one EventSource per channel, shared by every event listener on it. The
 * socket id is generated here and sent both with the subscription (so the
 * server can skip it for `broadcast_to_others`) and, by Inertia, in the
 * `X-Socket-Id` header of every request.
 */
export function sseTransport({ url = '/inertia/live' } = {}) {
  const socketId = globalThis.crypto?.randomUUID?.() ?? `${Date.now()}.${Math.random()}`
  const channels = new Map()
  const statusListeners = new Set()

  const notify = (connected) => statusListeners.forEach((callback) => callback(connected))

  const open = (channel) => {
    const params = new URLSearchParams({ channel: channel.name, type: channel.type, socket_id: socketId })
    const source = new EventSource(`${url}?${params}`, { withCredentials: true })
    source.addEventListener('inertia:subscribed', () => notify(true))
    source.onerror = () => notify(false)
    return { source, listeners: 0 }
  }

  return {
    socketId: () => socketId,

    onStatusChange(callback) {
      statusListeners.add(callback)
      return () => statusListeners.delete(callback)
    },

    subscribe(channel, event, handler) {
      const key = `${channel.type}:${channel.name}`
      let entry = channels.get(key)

      if (!entry) {
        entry = open(channel)
        channels.set(key, entry)
      }

      const listener = (message) => {
        let payload = null
        try {
          payload = JSON.parse(message.data)
        } catch {
          payload = null
        }
        handler(payload)
      }

      entry.source.addEventListener(event, listener)
      entry.listeners += 1

      return () => {
        entry.source.removeEventListener(event, listener)
        entry.listeners -= 1

        if (entry.listeners === 0) {
          entry.source.close()
          channels.delete(key)
        }
      }
    },
  }
}
