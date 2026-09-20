import { describe, it, expect, afterEach } from 'vitest'
import { tabFromHash, route, startRouter, go } from './router.svelte'

describe('hash router', () => {
  let stop: (() => void) | undefined
  afterEach(() => {
    stop?.()
    stop = undefined
    location.hash = ''
  })

  it('parses known tabs and falls back to dashboard', () => {
    expect(tabFromHash('#markets')).toBe('markets')
    expect(tabFromHash('#/cashflow')).toBe('cashflow')
    expect(tabFromHash('#trades/rt:123')).toBe('trades')
    expect(tabFromHash('')).toBe('dashboard')
    expect(tabFromHash('#nonsense')).toBe('dashboard')
  })

  it('go() sets the hash and the router reflects it in the reactive store', async () => {
    stop = startRouter()
    go('portfolio')
    // hashchange is async in jsdom; wait a tick
    await new Promise((r) => setTimeout(r, 0))
    expect(location.hash).toBe('#portfolio')
    expect(route.tab).toBe('portfolio')

    go('markets')
    await new Promise((r) => setTimeout(r, 0))
    expect(route.tab).toBe('markets')
  })
})
