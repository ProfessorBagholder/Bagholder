<script lang="ts">
  import { addWatch } from './state.svelte'
  import { store } from './state.svelte'

  let { onclose }: { onclose: () => void } = $props()

  interface Match { symbol: string; name: string; exchange: string; currency: string; kind?: string }

  let query = $state('')
  let matches = $state<Match[]>([])
  let hi = $state(0)
  let added = $state<Set<string>>(new Set())
  let reqId = 0
  let timer: ReturnType<typeof setTimeout> | undefined

  const watched = $derived(new Set((store.model?.markets.watchlist ?? []).map((w) => w.symbol + '|' + w.exchange)))
  const keyOf = (m: Match) => m.symbol + '|' + m.exchange

  function onInput() {
    clearTimeout(timer)
    const q = query.trim()
    if (!q) { matches = []; return }
    timer = setTimeout(async () => {
      const my = ++reqId
      try {
        const r = await fetch('/api/symbols/search?q=' + encodeURIComponent(q))
        const d = await r.json()
        if (my !== reqId) return
        matches = d.ok ? (d.matches ?? []) : []
        hi = 0
      } catch { if (my === reqId) matches = [] }
    }, 180)
  }

  function add(m: Match) {
    addWatch({ symbol: m.symbol, exchange: m.exchange, name: m.name, currency: m.currency })
    added = new Set(added).add(keyOf(m))
  }
  function onKey(e: KeyboardEvent) {
    if (e.key === 'Escape') { onclose(); return }
    if (e.key === 'ArrowDown') { e.preventDefault(); hi = Math.min(hi + 1, matches.length - 1) }
    else if (e.key === 'ArrowUp') { e.preventDefault(); hi = Math.max(hi - 1, 0) }
    else if (e.key === 'Enter' && matches[hi]) { e.preventDefault(); add(matches[hi]) }
  }
</script>

<svelte:window onkeydown={onKey} />
<div class="scrim" role="presentation" onclick={onclose}></div>
<div class="palette" role="dialog" aria-label="Search">
  <!-- svelte-ignore a11y_autofocus -->
  <input
    class="q"
    placeholder="Search symbol, name…"
    bind:value={query}
    oninput={onInput}
    autocomplete="off"
    aria-label="Search"
    autofocus
  />
  {#if matches.length}
    <div class="results">
      {#each matches as m, i (keyOf(m))}
        <button class="row" class:hi={i === hi} onmouseenter={() => (hi = i)} onclick={() => add(m)}>
          <span class="sym">{m.symbol}</span>
          <span class="name">{m.name}</span>
          <span class="exch">{m.exchange}</span>
          <span class="act">
            {#if watched.has(keyOf(m)) || added.has(keyOf(m))}<span class="on">✓ Watching</span>{:else}<span class="add">+ Watch</span>{/if}
          </span>
        </button>
      {/each}
    </div>
  {:else if query.trim()}
    <div class="empty">No listing by that name.</div>
  {:else}
    <div class="empty">Type a symbol or company name.</div>
  {/if}
</div>

<style>
  .scrim { position: fixed; inset: 0; background: rgba(0,0,0,0.5); z-index: 50; }
  .palette { position: fixed; top: 90px; left: 50%; transform: translateX(-50%); width: min(560px, 92vw); background: #141924; border: 1px solid #2a3242; border-radius: 12px; z-index: 51; box-shadow: 0 20px 60px rgba(0,0,0,0.6); overflow: hidden; }
  .q { width: 100%; box-sizing: border-box; background: #0b0e14; border: 0; border-bottom: 1px solid #1c2230; color: #e6e9ef; font: inherit; font-size: 15px; padding: 14px 16px; outline: none; }
  .results { max-height: 50vh; overflow-y: auto; }
  .row { width: 100%; display: grid; grid-template-columns: auto 1fr auto auto; gap: 12px; align-items: center; text-align: left; background: none; border: 0; color: inherit; font: inherit; padding: 9px 16px; cursor: pointer; }
  .row.hi { background: #1c2534; }
  .sym { font-weight: 600; font-size: 13px; }
  .name { color: #8b93a7; font-size: 12px; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .exch { color: #8b93a7; font-size: 11px; }
  .act { font-size: 11px; }
  .add { color: #4b9fff; } .on { color: #3ecf8e; }
  .empty { padding: 18px 16px; color: #8b93a7; font-size: 13px; }
</style>
