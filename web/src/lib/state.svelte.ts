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
