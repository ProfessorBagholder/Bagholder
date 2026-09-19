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

export const route = $state<{ tab: Tab }>({ tab: tabFromHash(typeof location !== 'undefined' ? location.hash : '') })

// Wire hash changes to the store; returns a teardown for onMount.
export function startRouter(): () => void {
  const on = () => {
    route.tab = tabFromHash(location.hash)
  }
  on()
  window.addEventListener('hashchange', on)
  return () => window.removeEventListener('hashchange', on)
}

export function go(tab: Tab): void {
  location.hash = tab
}
