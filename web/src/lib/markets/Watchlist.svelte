<script lang="ts">
  // The watchlist card (watchlistCardHtml): ranked rows with an add row (the plus
  // toggles a symbol-search input with suggestions) and a per-row trash.
  import type { WatchItem, SymbolMatch } from '../model'
  import { ICONS } from '../icons'
  import { px } from '../fmt'
  import { signedPct } from './util'
  import { bareSymbol, symText } from '../sym'
  import { sort, toggleSort, sortRows } from '../sort.svelte'
  import { store, addWatch, removeWatch } from '../state.svelte'
  import { sugQuotes, sugKey, sugQuoteSchedule } from './quotes.svelte'
  import Icon from './Icon.svelte'
  import GridHead from './GridHead.svelte'
  import { request } from '../api'
  import { searchSymbols } from '../api'
  import { goSub } from '../router.svelte'
  import { rememberListing } from '../listing.svelte'

  let { watchlist }: { watchlist: WatchItem[] } = $props()

  const WATCH_COLS = [
    { key: 'symbol', label: 'Symbol' },
    { key: 'exchange', label: 'Exch', align: 'right' as const },
    { key: 'last', label: 'Last', align: 'right' as const },
    { key: 'chg', label: 'Chg', align: 'right' as const },
  ]
  const OPTION_RE = /^\S+ (\d{2})([A-Z]{3})(\d{2}) /

  let adding = $state(false)
  let query = $state('')
  let watchNew = $state<string | null>(null)
  let newTimer: ReturnType<typeof setTimeout> | undefined
  let input = $state<HTMLInputElement | null>(null)

  const watchedKeys = $derived(new Set((watchlist || []).map((w) => bareSymbol(w.symbol) + '@' + String(w.exchange || '').toUpperCase())))

  // rows: cached quote fills in a just-added row's price until the server's lands
  const rows = $derived.by(() => {
    const s = sort.watchlist
    const withQuote = (watchlist || []).map((w) => {
      const c = w.last == null ? sugQuotes[sugKey(w)] : null
      return c ? { ...w, ...c } : w
    })
    return sortRows(
      withQuote.map((w, i) => ({ added: i, ...w })),
      s.key,
      s.dir,
      (w, k) => (k === 'symbol' ? bareSymbol(w.symbol).toLowerCase() : k === 'exchange' ? String(w.exchange || '').toLowerCase() : k === 'last' ? w.last : k === 'chg' ? w.percentChange : w.added),
    )
  })

  // --- add row suggestions ---
  function watchAlready(q: string): boolean {
    const key = q.trim().toUpperCase()
    if (!key) return false
    return (watchlist || []).some((w) => bareSymbol(w.symbol).toUpperCase() === key)
  }
  // with nothing typed, the holdings not yet followed
  const fromHoldings = $derived.by<SymbolMatch[]>(() => {
    const out: SymbolMatch[] = []
    const seen = new Set<string>()
    ;(store.model?.positions || []).forEach((p) => {
      if (p.kind === 'Options' || OPTION_RE.test(p.symbol)) return
      const k = bareSymbol(p.symbol) + '@' + String(p.exchange || '').toUpperCase()
      if (watchedKeys.has(k) || seen.has(k)) return
      seen.add(k)
      out.push({ symbol: bareSymbol(p.symbol), exchange: p.exchange || '', name: p.name || '', currency: p.currency || '', last: p.last, percentChange: p.percentChange })
    })
    return out.slice(0, 4)
  })

  // typed matches come from /api/symbols/search, debounced
  let matches = $state<SymbolMatch[]>([])
  let searchTimer: ReturnType<typeof setTimeout> | undefined
  $effect(() => {
    const q = query.trim()
    if (!q) {
      matches = []
      return
    }
    clearTimeout(searchTimer)
    const key = q
    searchTimer = setTimeout(() => {
      searchSymbols(key).then((found) => {
        if (query.trim() !== key) return
        const out: SymbolMatch[] = []
        found.forEach((m) => {
          if (OPTION_RE.test(m.symbol) || m.kind) return
          const k = bareSymbol(m.symbol) + '@' + String(m.exchange || '').toUpperCase()
          if (watchedKeys.has(k) || out.some((x) => bareSymbol(x.symbol) + '@' + String(x.exchange || '').toUpperCase() === k)) return
          const p = (store.model?.positions || []).find((x) => x.symbol === m.symbol)
          out.push({ symbol: bareSymbol(m.symbol), exchange: m.exchange || '', name: m.name || '', currency: m.currency || (p ? p.currency : ''), last: p ? p.last : null, percentChange: p ? p.percentChange : null })
        })
        matches = out.slice(0, 4)
      })
    }, 300)
  })

  const suggestions = $derived.by(() => {
    const q = query.trim()
    const base = q ? matches : fromHoldings
    return base.map((w) => {
      const c = w.last == null ? sugQuotes[sugKey(w)] : null
      return c ? { ...w, ...c } : w
    })
  })
  $effect(() => {
    if (query.trim()) sugQuoteSchedule(suggestions)
  })

  function toggleAdd() {
    adding = !adding
    query = ''
    if (adding) setTimeout(() => input?.focus(), 0)
  }
  function pick(w: SymbolMatch) {
    const key = bareSymbol(w.symbol) + '@' + String(w.exchange || '').toUpperCase()
    query = ''
    watchNew = key
    addWatch({ symbol: w.symbol, exchange: w.exchange, name: w.name, currency: w.currency }).then(() => {
      clearTimeout(newTimer)
      newTimer = setTimeout(() => (watchNew = null), 1400)
    })
    input?.focus()
  }
  function openRow(w: WatchItem) {
    if (w.positionId) return goSub('portfolio', w.positionId)
    goSub('markets', rememberListing(w))
  }
  const chgColor = (v: number | null | undefined) => (v == null ? 'var(--ink55)' : v >= 0 ? 'var(--pos)' : 'var(--neg)')
</script>

<div class="card elev-sm" style="padding:14px 16px 12px;display:flex;flex-direction:column;min-height:0">
  <div style="display:flex;align-items:center;min-height:28px;margin-bottom:8px">
    <h5>Watchlist</h5>
    <button class="wl-add" class:on={adding} style="margin-left:auto" aria-label={adding ? 'Close' : 'Add to the watchlist'} onclick={toggleAdd}><Icon d={ICONS.plus} /></button>
  </div>

  {#if adding}
    <div class="wl-field">
      <Icon d={ICONS.search} />
      <input bind:this={input} bind:value={query} placeholder="Search symbol..." aria-label="Search symbol" onkeydown={(e) => { if (e.key === 'Escape') toggleAdd() }} />
      <span class="esc">ESC</span>
    </div>
    <div class="lbl" style="padding:0 0 6px">{query.trim() ? 'Matches' : 'From Holdings'}</div>
    {#if suggestions.length}
      <div class="wl-list">
        {#each suggestions as w, i (sugKey(w))}
          <div class="wl-row wl-sug" class:hi={query.trim() && i === 0} role="button" tabindex="-1" onclick={() => pick(w)} onkeydown={(e) => { if (e.key === 'Enter') pick(w) }}>
            <div><div style="font-size:13px;font-weight:500;overflow:hidden;text-overflow:ellipsis">{w.symbol}</div><div style="font-size:11px;color:rgba(var(--ink-rgb),.45);overflow:hidden;text-overflow:ellipsis">{w.name || ''}</div></div>
            <div style="font-size:11px;color:rgba(var(--ink-rgb),.45);text-align:right">{w.exchange || ''}</div>
            <div class="tab" style="text-align:right;font-size:13px">{w.last == null ? '—' : px(w.last)}</div>
            <div class="tab" style="text-align:right;font-size:13px;color:{w.percentChange == null ? 'rgba(var(--ink-rgb),.45)' : w.percentChange >= 0 ? 'var(--pos)' : 'var(--neg)'}">{signedPct(w.percentChange)}</div>
            <div></div>
          </div>
        {/each}
      </div>
    {:else}
      <div class="muted" style="font-size:12px;padding:2px 0 6px">{query.trim() ? (watchAlready(query) ? 'Already on the watchlist.' : 'No listing by that name.') : 'Every holding is followed.'}</div>
    {/if}
    <div class="wl-rule"></div>
  {/if}

  {#if rows.length}
    <div class="wl-head"><GridHead table="watchlist" cols={WATCH_COLS} /><div></div></div>
    <div class="scroll wl-list" style="flex:1;min-height:0;max-height:436px">
      {#each rows as w (bareSymbol(w.symbol) + '@' + String(w.exchange || '').toUpperCase())}
        <div class="wl-row go" class:wl-new={watchNew === bareSymbol(w.symbol) + '@' + String(w.exchange || '').toUpperCase()} role="button" tabindex="-1" onclick={() => openRow(w)} onkeydown={(e) => { if (e.key === 'Enter') openRow(w) }}>
          <div><div style="font-size:13px;font-weight:500;overflow:hidden;text-overflow:ellipsis">{bareSymbol(w.symbol)}</div><div style="font-size:11px;color:var(--ink55);overflow:hidden;text-overflow:ellipsis">{w.name || ''}</div></div>
          <div style="font-size:12px;color:var(--ink55);text-align:right">{w.exchange || ''}</div>
          <div class="tab" style="text-align:right;font-size:13px">{w.last == null ? '—' : px(w.last)}</div>
          <div class="tab" style="text-align:right;font-size:13px;color:{chgColor(w.percentChange)}">{signedPct(w.percentChange)}</div>
          <button class="wl-trash" aria-label="Remove {symText(w.symbol)} from the watchlist" onclick={(e) => { e.stopPropagation(); removeWatch(w.symbol, w.exchange) }}><Icon d={ICONS.trash} /></button>
        </div>
      {/each}
    </div>
  {:else}
    <div class="muted" style="font-size:12px">Nothing watched.</div>
  {/if}
</div>
