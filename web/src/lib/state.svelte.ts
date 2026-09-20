import type { Model } from './model'

// The reactive store. This is the whole point of the migration: state lives in
// one $state rune and the UI derives from it — no manual coreVersion/dataVersion
// bookkeeping, no reconciler. Components read store.model and Svelte tracks the
// dependency itself.
export const store = $state<{ model: Model | null; error: string | null; loading: boolean }>({
  model: null,
  error: null,
  loading: true,
})

// The data layer, behavior-equivalent to the legacy loadModel(): fetch the
// server-derived model and hand it to the store. (loadLive / status polling
// come in later phases; the spike proves the full-model path.)
export async function loadModel(): Promise<void> {
  store.loading = true
  try {
    const r = await fetch('/api/model')
    if (!r.ok) throw new Error('HTTP ' + r.status)
    store.model = (await r.json()) as Model
    store.error = null
  } catch (e) {
    store.error = e instanceof Error ? e.message : String(e)
  } finally {
    store.loading = false
  }
}

// A journal edit: mutate the one trade optimistically (so the UI reflects it at
// once and nothing else re-renders), then persist. On failure, surface it and
// reload the authoritative model. This is the "update exactly what changed,
// nothing else affected" pattern the reconciler could never guarantee.
export async function saveJournal(id: string, patch: { thesis?: string; grade?: string; tags?: string[] }): Promise<void> {
  const t = store.model?.trades.find((x) => x.id === id)
  if (!t) return
  Object.assign(t, patch)
  const body = { id, thesis: t.thesis ?? '', tags: t.tags ?? [], grade: t.grade ?? '' }
  try {
    const r = await fetch('/api/journal', { method: 'POST', headers: { 'content-type': 'application/json' }, body: JSON.stringify(body) })
    const d = await r.json()
    if (!d.ok) throw new Error('save failed')
  } catch {
    store.error = 'Could not save journal entry.'
    loadModel()
  }
}

// Remove a watchlist listing: drop it from the store at once (so the Watchlist
// card and the heatmap's watchlist universe update, and nothing else does), then
// persist. On failure, reload the authoritative model.
export async function removeWatch(symbol: string, exchange: string): Promise<void> {
  const mk = store.model?.markets
  if (!mk) return
  mk.watchlist = mk.watchlist.filter((w) => !(w.symbol === symbol && w.exchange === exchange))
  try {
    const r = await fetch('/api/watchlist/remove', { method: 'POST', headers: { 'content-type': 'application/json' }, body: JSON.stringify({ symbol, exchange }) })
    const d = await r.json()
    if (!d.ok) throw new Error('remove failed')
  } catch {
    loadModel()
  }
}
