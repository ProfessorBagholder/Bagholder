import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { flushSync } from 'svelte'
import { store } from './state.svelte'
import { connect, followConnect, ui } from './ui.svelte'
import type { Model } from './model'

// SPEC §4, Connecting: the wait for a sign-in ends on the server's clock, in the server's
// words. The page keeps no deadline of its own to race it.

let stop: () => void
beforeEach(() => {
  vi.useFakeTimers()
  vi.stubGlobal('fetch', vi.fn(async () => new Response(JSON.stringify({ ok: true }))))
  store.model = { status: { connected: false, capturing: false, error: '' } } as unknown as Model
  stop = followConnect()
})
afterEach(() => {
  stop()
  ui.connecting = false
  vi.useRealTimers()
  vi.unstubAllGlobals()
})

const started = async () => {
  connect()
  await vi.advanceTimersByTimeAsync(0) // the start is answered
  const st = store.model!.status as unknown as Record<string, unknown>
  st.capturing = true
  flushSync()
  return st
}

describe('waiting for a sign-in', () => {
  it('is never ended by a clock on the page, however long the server keeps waiting', async () => {
    const st = await started()
    await vi.advanceTimersByTimeAsync(60 * 60_000)
    flushSync()
    expect(ui.connecting).toBe(true)
    expect(st.error).toBe('')
  })

  it("ends when the server stops waiting, with the server's words and no others", async () => {
    const st = await started()
    const said = 'the server said ' + Math.random()
    st.error = said
    st.capturing = false
    flushSync()
    expect(ui.connecting).toBe(false)
    expect(st.error).toBe(said)
  })

  it('ends with nothing of its own to say where the server says nothing', async () => {
    const st = await started()
    st.capturing = false
    flushSync()
    expect(ui.connecting).toBe(false)
    expect(st.error).toBe('')
  })
})
