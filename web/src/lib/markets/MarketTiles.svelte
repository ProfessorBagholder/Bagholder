<script lang="ts">
  import { roll } from '../actions/roll'
  // The market-tiles row (marketTilesHtml): draggable KPI tiles, a plus cell that
  // opens the instrument picker, and a "more" expander. Order is persisted through
  // /api/tiles/set exactly as the reference page does.
  import type { MarketTile, MarketInstrument } from '../model'
  import { ICONS } from '../icons'
  import Icon from './Icon.svelte'
  import { n2, signedPct } from './util'
  import { call } from '../api'
  import { focusOnMount } from '../actions/focus'
  import { escapable } from '../escape'

  let { tiles: propTiles, instruments }: { tiles: MarketTile[]; instruments: MarketInstrument[] } = $props()

  const TILES_MAX = 12
  const TILES_ROW = 6

  // an optimistic order held while a drag or a save is in flight; otherwise the
  // model's own tiles (tileOrder())
  let order = $state<MarketTile[] | null>(null)
  const tiles = $derived(order ?? propTiles ?? [])

  let tilesOpen = $state((() => { try { return localStorage.getItem('bh2.tilesOpen') === '1' } catch { return false } })())
  let tileAdd = $state(false)
  // Escape closes the picker wherever the focus is
  $effect(() => {
    if (tileAdd) return escapable(() => { tileAdd = false; return true })
  })
  let tileQuery = $state('')
  let dragSym = $state<string | null>(null)

  const shown = $derived(tilesOpen ? tiles : tiles.slice(0, TILES_ROW))
  const room = $derived(tilesOpen ? TILES_MAX : TILES_ROW)
  const plus = $derived(shown.length < room)
  const beyond = $derived(tiles.length - TILES_ROW)
  const showMore = $derived(tiles.length >= TILES_ROW && !(tilesOpen && tiles.length >= TILES_MAX))

  $effect(() => {
    if (!plus && tileAdd) tileAdd = false // the plus cell is gone: its box goes with it
  })

  async function postTiles(next: MarketTile[]) {
    order = next.slice()
    const r = await call('POST /api/tiles/set', { body: { tiles: next.map((t) => ({ symbol: t.symbol, exchange: t.exchange })) } })
    if (!r || !r.ok) order = null // refused: back to what the server has
  }
  // The saved row reaches the page as a change to the tiles, and each tile's price as
  // a change to that tile when the server has read it. The optimistic order steps
  // aside the moment the server's row says the same thing.
  $effect(() => {
    if (!order || !propTiles) return
    const sent = order.map((t) => t.symbol + '@' + t.exchange).join()
    if (sent === propTiles.map((t) => t.symbol + '@' + t.exchange).join()) order = null
  })

  function tileRemove(sym: string) {
    postTiles(tiles.filter((t) => t.symbol !== sym))
  }
  function tileToggle(inst: MarketInstrument) {
    const has = tiles.some((t) => t.symbol === inst.symbol)
    if (!has && tiles.length >= TILES_MAX) return
    const next = has
      ? tiles.filter((t) => t.symbol !== inst.symbol)
      : tiles.concat([{ symbol: inst.symbol, exchange: inst.exchange, label: inst.label, name: inst.name, kind: inst.kind, last: null, change: null, percentChange: null, decimals: 2 }])
    if (!has && next.length > TILES_ROW && !tilesOpen) {
      tilesOpen = true
      try { localStorage.setItem('bh2.tilesOpen', '1') } catch { /* ignore */ }
    }
    if (next.length >= TILES_MAX) tileAdd = false
    postTiles(next)
  }
  function tilesMore() {
    tilesOpen = !tilesOpen
    if (!tilesOpen) tileAdd = false
    try { localStorage.setItem('bh2.tilesOpen', tilesOpen ? '1' : '0') } catch { /* ignore */ }
  }
  function openPicker() {
    tileAdd = !tileAdd
    tileQuery = ''
  }

  // The picker opens under the app's header at the right, where ⌘K opens, so the box
  // stays put as tiles are added: placed against the header's popover anchor, measured
  // from this row (both scroll with the page), and again when the window is resized.
  let row = $state<HTMLElement | null>(null)
  let pickAt = $state<{ top: number; right: number } | null>(null)
  $effect(() => {
    if (!tileAdd || !row) return
    const el = row
    const place = () => {
      const a = document.getElementById('popAnchor')
      if (!a) return (pickAt = null)
      const ab = a.getBoundingClientRect()
      const rb = el.getBoundingClientRect()
      pickAt = { top: ab.top - rb.top + 42, right: rb.right - ab.right }
    }
    place()
    window.addEventListener('resize', place)
    return () => window.removeEventListener('resize', place)
  })

  // --- the picker ---
  const on = $derived(new Set(tiles.map((t) => t.symbol)))
  const pickerRows = $derived.by(() => {
    const q = tileQuery.trim().toUpperCase()
    return (instruments || []).filter((r) => !q || r.symbol.indexOf(q) >= 0 || r.label.indexOf(q) >= 0 || r.name.toUpperCase().indexOf(q) >= 0 || (r.aliases || []).some((a) => a.toUpperCase().indexOf(q) >= 0))
  })

  // --- drag reorder (pointer events) ---
  let grid = $state<HTMLElement | null>(null)
  let td: { sym: string; x: number; y: number; ox: number; oy: number; on: boolean; order: MarketTile[] } | null = null

  function onPointerDown(e: PointerEvent, sym: string) {
    if (e.button !== 0 || (e.target as HTMLElement).closest('.mt-x')) return
    const r = (e.currentTarget as HTMLElement).getBoundingClientRect()
    td = { sym, x: e.clientX, y: e.clientY, ox: e.clientX - r.left, oy: e.clientY - r.top, on: false, order: tiles.slice() }
    window.addEventListener('pointermove', onPointerMove)
    window.addEventListener('pointerup', onPointerUp)
    window.addEventListener('pointercancel', onPointerUp)
  }
  function onPointerMove(e: PointerEvent) {
    if (!td) return
    if (!td.on) {
      if (Math.hypot(e.clientX - td.x, e.clientY - td.y) < 4) return
      td.on = true
      order = td.order.slice()
      dragSym = td.sym
    }
    const cells = grid ? ([...grid.querySelectorAll('.mt-cell[data-sym]')] as HTMLElement[]) : []
    const over = cells.findIndex((c) => { const b = c.getBoundingClientRect(); return e.clientX >= b.left && e.clientX <= b.right && e.clientY >= b.top && e.clientY <= b.bottom })
    const from = td.order.findIndex((t) => t.symbol === td!.sym)
    if (over >= 0 && over !== from) {
      const [m] = td.order.splice(from, 1)
      td.order.splice(over, 0, m)
      order = td.order.slice()
    }
    // the pressed tile follows the pointer: measure its untransformed layout
    // position, then place it under the pointer (imperative, as ledger does)
    const el = grid?.querySelector('.mt-tile[data-sym="' + CSS.escape(td.sym) + '"]') as HTMLElement | null
    if (el) {
      el.style.transform = ''
      const b = el.getBoundingClientRect()
      el.style.transform = 'translate(' + (e.clientX - td.ox - b.left) + 'px,' + (e.clientY - td.oy - b.top) + 'px) scale(1.015)'
    }
  }
  function onPointerUp() {
    window.removeEventListener('pointermove', onPointerMove)
    window.removeEventListener('pointerup', onPointerUp)
    window.removeEventListener('pointercancel', onPointerUp)
    const d = td
    td = null
    // the tile settles into its cell on release, before the save
    const el = d && grid?.querySelector('.mt-tile[data-sym="' + CSS.escape(d.sym) + '"]') as HTMLElement | null
    if (el) el.style.transform = ''
    dragSym = null
    if (!d || !d.on) return
    postTiles(d.order)
  }
</script>

<div id="mtRow" bind:this={row} style={tileAdd ? 'position:relative;z-index:30' : 'position:relative'}>
  <div id="mtGrid" bind:this={grid} style="display:grid;grid-template-columns:repeat(6,minmax(0,1fr));gap:14px">
    {#each shown as t (t.symbol)}
      <div class="mt-cell" data-sym={t.symbol}>
        <div class="card elev-sm kpi mt-tile" class:mt-drag={dragSym === t.symbol} data-sym={t.symbol} data-ex={t.exchange} onpointerdown={(e) => onPointerDown(e, t.symbol)}>
          <div class="lbl">{t.label}</div>
          <div class="v" use:roll={t.rate != null ? n2(t.rate, 2) + '%' : t.last == null ? '—' : n2(t.last, t.decimals)}></div>
          {#if t.rate != null}
            {@const m = t.rateChange}
            <div class="s" style="color:var(--ink55)">{t.last == null ? '—' : n2(t.last, t.decimals)}{#if m != null} <span style="color:var(--{m < 0 ? 'neg' : 'pos'})">({m < 0 ? '−' : '+'}{n2(Math.abs(m), t.decimals)})</span>{/if}</div>
          {:else if t.change == null || t.percentChange == null}
            <div class="s">—</div>
          {:else}
            <div class="s {t.change < 0 ? 'neg' : 'pos'}" style="color:var(--{t.change < 0 ? 'neg' : 'pos'})">{t.change < 0 ? '−' : '+'}{n2(Math.abs(t.change), t.decimals)} ({signedPct(t.percentChange)})</div>
          {/if}
          <button class="mt-x" aria-label="Remove {t.label}" onclick={() => tileRemove(t.symbol)}><Icon d={ICONS.x} /></button>
        </div>
      </div>
    {/each}
    {#if plus}
      <div class="mt-cell">
        <button class="mt-plus" class:on={tileAdd} aria-label="Add a tile" onclick={openPicker}><Icon d={ICONS.plus} /></button>
      </div>
    {/if}
  </div>
  {#if showMore}
    <div class="mt-more" class:open={tilesOpen}>
      <button onclick={tilesMore}>{tilesOpen ? 'Show fewer' : beyond > 0 ? 'Show ' + beyond + ' more' : 'Show more'}<Icon d={ICONS.caretDown} /></button>
    </div>
  {/if}

  {#if tileAdd}
    <div class="mt-scrim" onclick={() => (tileAdd = false)} role="presentation"></div>
    <div class="pop elev-md" style={pickAt ? `top:${pickAt.top}px;right:${pickAt.right}px` : ''}>
      <div style="display:flex;align-items:center;gap:7px;padding:5px 7px;margin-bottom:8px;border-radius:6px;background:var(--n900);box-shadow:inset 0 0 0 1px rgba(var(--ink-rgb),.1)">
        <Icon d={ICONS.search} />
        <input bind:value={tileQuery} placeholder="Index, future, commodity, rate or pair" aria-label="Search instruments" use:focusOnMount style="flex:1;min-width:0;border:0;background:transparent;color:var(--ink);font:400 12.5px var(--font);outline:none" />
        <span style="font-size:10px;color:rgba(var(--ink-rgb),.4)">ESC</span>
      </div>
      <div class="lbl" style="padding:2px 7px 7px">{tileQuery.trim() ? 'Matches' : 'Market instruments'}</div>
      {#if pickerRows.length}
        <div class="scroll" style="max-height:264px;display:flex;flex-direction:column;gap:1px">
          {#each pickerRows as r (r.symbol)}
            <div class="pop-row mt-row" class:on={on.has(r.symbol)} class:off={!on.has(r.symbol) && tiles.length >= TILES_MAX} role="button" tabindex="-1" onclick={() => tileToggle(r)} onkeydown={(e) => { if (e.key === 'Enter') tileToggle(r) }}>
              <span style="min-width:0"><span class="mt-sym" style="display:block;font-weight:500">{r.symbol}</span><span style="display:block;font-size:11px;color:rgba(var(--ink-rgb),.45);overflow:hidden;text-overflow:ellipsis;white-space:nowrap">{r.name}</span></span>
              <span style="margin-left:auto;font-size:11px;color:rgba(var(--ink-rgb),.4);white-space:nowrap">{r.exchange}</span>
              <span class="mt-bm"><Icon d={on.has(r.symbol) ? ICONS.bookmarkFill : ICONS.bookmark} /></span>
            </div>
          {/each}
        </div>
      {:else}
        <div class="muted" style="padding:2px 7px 8px;font-size:12px">No match.</div>
      {/if}
      <div style="font-size:11px;color:rgba(var(--ink-rgb),.4);padding:8px 7px 0">{tiles.length} of {TILES_MAX} tiles used</div>
    </div>
  {/if}
</div>
