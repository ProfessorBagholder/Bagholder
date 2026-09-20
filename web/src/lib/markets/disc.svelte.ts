// Disclosures the News card reads: a per-scope feed (/api/filings/feed) and a
// per-listing payload (/api/filings), with titles filling in top-first from
// /api/filings/enrich. Reactive $state so the card fills in as documents are read.
//
// Note: the reference page's enrichment has an elaborate "waiting for the local
// summary model" retry state machine (discEnrichSettle/discSummaryReady). Here the
// sweep is simplified — each row is read once for its subject, top of the list
// first — which fills every title but does not re-poll a summary that is still
// being prepared. Titles (what the card shows) are faithful; the deferred summary
// re-polling is not reproduced.
import type { Filing } from '../model'
import { api } from './util'

export interface DiscRow extends Filing {
  symbol?: string
  exchange?: string
}
export interface Payload {
  filings?: Filing[]
  sources?: Record<string, { available?: boolean; filer?: boolean }>
}

export const discFeed = $state<Record<string, { rows: DiscRow[] | null; at: number; loading: boolean }>>({})
export const discBySym = $state<Record<string, { loading?: boolean; payload?: Payload; error?: string }>>({})

export const enrichLoading = $state<Record<string, boolean>>({})
const enrichTried: Record<string, boolean> = {}
const sweeping: Record<string, boolean> = {}

export function loadDiscFeed(scope: string): void {
  const f = discFeed[scope]
  if (f && (f.loading || Date.now() - f.at < 60000)) return
  discFeed[scope] = { rows: f ? f.rows : null, at: f ? f.at : 0, loading: true }
  api<{ ok: boolean; filings?: DiscRow[] }>('GET', '/api/filings/feed?scope=' + encodeURIComponent(scope)).then((r) => {
    discFeed[scope] = { rows: r && r.ok ? r.filings || [] : f ? f.rows : [], at: Date.now(), loading: false }
  })
}

export function ensureDisclosures(t: { symbol: string; exchange?: string; name?: string; currency?: string }): void {
  const sym = String(t.symbol || '').toUpperCase()
  if (!sym || discBySym[sym]) return
  discBySym[sym] = { loading: true }
  const q =
    'symbol=' + encodeURIComponent(sym) + (t.name ? '&name=' + encodeURIComponent(t.name) : '') + (t.exchange ? '&exchange=' + encodeURIComponent(t.exchange) : '') + (t.currency ? '&currency=' + encodeURIComponent(t.currency) : '')
  api<{ ok: boolean; filings?: Filing[]; sources?: Payload['sources']; error?: string }>('GET', '/api/filings?' + q).then((d) => {
    discBySym[sym] = d && d.ok ? { payload: { filings: d.filings, sources: d.sources } } : { error: (d && d.error) || 'Could not read disclosures.' }
  })
}

// whether a row's title is still on its way (ledger's discTitleComing, simplified)
export function discTitleComing(f: DiscRow): boolean {
  if (f.subject) return false
  return !!enrichLoading[f.id] || !enrichTried[f.id]
}

// fill each row's subject top-first, one at a time, while `isOpen()` holds
export function sweepEnrich(key: string, rows: DiscRow[], isOpen: () => boolean): void {
  if (sweeping[key]) return
  const need = (f: DiscRow) => !f.subject && !enrichTried[f.id]
  const pending = rows.filter(need).slice(0, 60)
  if (!pending.length) return
  sweeping[key] = true
  let i = 0
  const step = () => {
    if (!isOpen()) {
      sweeping[key] = false
      return
    }
    while (i < pending.length && !need(pending[i])) i++
    if (i >= pending.length) {
      sweeping[key] = false
      return
    }
    const f = pending[i++]
    enrichLoading[f.id] = true
    api<{ ok: boolean; subject?: string; summary?: string; summaryStatus?: string }>('GET', '/api/filings/enrich?symbol=' + encodeURIComponent(f.symbol || '') + '&id=' + encodeURIComponent(f.id)).then((d) => {
      delete enrichLoading[f.id]
      if (d && d.ok) {
        if (d.subject) f.subject = d.subject
        if (d.summary) f.summary = d.summary
        f.summaryStatus = d.summaryStatus || ''
      }
      enrichTried[f.id] = true
      setTimeout(step, 150)
    })
  }
  step()
}
