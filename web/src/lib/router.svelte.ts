// Hash router as reactive state. The active tab is derived from location.hash;
// components read `route.tab` and Svelte re-renders the affected parts only —
// no whole-page redraw, no manual dispatch table.

export const TABS = ['dashboard', 'trades', 'portfolio', 'markets', 'cashflow'] as const
export type Tab = (typeof TABS)[number]

export const TAB_LABEL: Record<Tab, string> = {
  dashboard: 'Dashboard',
  trades: 'Trades',
  portfolio: 'Portfolio',
  markets: 'Markets',
  cashflow: 'Cashflow',
}

export function tabFromHash(hash: string): Tab {
  const h = hash.replace(/^#\/?/, '').split('/')[0]
  return (TABS as readonly string[]).includes(h) ? (h as Tab) : 'dashboard'
}

// The second path segment — a drill-down id (e.g. a selected trade), or null.
export function subFromHash(hash: string): string | null {
  const parts = hash.replace(/^#\/?/, '').split('/')
  return parts.length > 1 && parts[1] ? decodeURIComponent(parts.slice(1).join('/')) : null
}

const initHash = typeof location !== 'undefined' ? location.hash : ''
export const route = $state<{ tab: Tab; sub: string | null }>({ tab: tabFromHash(initHash), sub: subFromHash(initHash) })

// Wire hash changes to the store; returns a teardown for onMount.
export function startRouter(): () => void {
  const on = () => {
    route.tab = tabFromHash(location.hash)
    route.sub = subFromHash(location.hash)
  }
  on()
  window.addEventListener('hashchange', on)
  return () => window.removeEventListener('hashchange', on)
}

export function go(tab: Tab): void {
  location.hash = tab
}

export function goSub(tab: Tab, sub: string): void {
  location.hash = tab + '/' + encodeURIComponent(sub)
}
