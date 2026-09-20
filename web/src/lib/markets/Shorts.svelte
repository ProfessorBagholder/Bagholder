<script lang="ts">
  // Ranked short interest (shortsListCardHtml): the listings the book holds and
  // watches, ranked, from what the sweep has stored (/api/shorts/feed); the search
  // box reaches any ticker (/api/shorts?symbol=).
  import type { ShortsFeedRow } from '../model'
  import { qty } from '../fmt'
  import { n2, shortDay } from './util'
  import { sort, sortRows } from '../sort.svelte'
  import { bareSymbol } from '../sym'
  import { watchDoc } from '../live'
  import Mseg from './Mseg.svelte'
  import GridHead from './GridHead.svelte'
  import { request } from '../api'

  const SCOPE_OPTS = [['all', 'All'], ['holdings', 'Holdings'], ['watchlist', 'Watchlist']] as const
  const SHORTS_COLS = [
    { key: 'symbol', label: 'Symbol' },
    { key: 'exchange', label: 'Exchange', align: 'right' as const },
    { key: 'shares', label: 'Short shares', align: 'right' as const },
    { key: 'ofFloat', label: 'Of float', align: 'right' as const },
    { key: 'daysToCover', label: 'Days to cover', align: 'right' as const },
    { key: 'volumePct', label: 'Short volume', align: 'right' as const },
    { key: 'asOf', label: 'As of', align: 'right' as const },
  ]

  let scope = $state((() => { try { return localStorage.getItem('bh2.shorts') || 'all' } catch { return 'all' } })())
  let query = $state('')

  // The table is shown only while this card is, so it is sent only then: whole once,
  // and after that each listing's row as the sweep reads it, and the word that a read
  // is under way -- pushed, not asked for every four seconds.
  const feedDoc = $state<{ data: { ok: boolean; rows: ShortsFeedRow[]; reading: boolean } | null }>({ data: null })
  const feed = {
    get rows() { return feedDoc.data ? feedDoc.data.rows : null },
    get reading() { return !!feedDoc.data?.reading },
    get loading() { return feedDoc.data == null },
  }
  const found = $state<Record<string, { loading?: boolean; missing?: boolean; row?: ShortsFeedRow }>>({})
  $effect(() => watchDoc('shorts', {}, feedDoc))

  let lookupTimer: ReturnType<typeof setTimeout> | undefined
  function shortsLookup(text: string) {
    const key = text.trim().toUpperCase()
    clearTimeout(lookupTimer)
    if (!key || key.length > 12 || /[^A-Z0-9.\-]/.test(key) || found[key]) return
    lookupTimer = setTimeout(() => {
      if (query.trim().toUpperCase() !== key) return
      if ((feed.rows || []).some((r) => String(r.symbol).toUpperCase() === key)) return
      found[key] = { loading: true }
      request<{ ok: boolean; covered?: boolean; shorts?: ShortsFeedRow }>('GET', '/api/shorts?symbol=' + encodeURIComponent(key)).then((d) => {
        found[key] = d && d.ok && d.covered ? { row: { name: '', ...(d.shorts as ShortsFeedRow) } } : { missing: true }
      })
    }, 450)
  }
  $effect(() => {
    shortsLookup(query)
  })

  const rows = $derived.by(() => {
    const q = query.trim().toUpperCase()
    const s = sort.shorts
    let pool = (feed.rows || []).filter((r) => scope === 'all' || (scope === 'holdings' ? r.held : r.watched))
    if (q) {
      pool = pool.filter((r) => String(r.symbol).toUpperCase().indexOf(q) === 0 || String(r.name || '').toUpperCase().indexOf(q) >= 0)
      const f = found[q]
      if (!pool.length && f && f.row) pool = [f.row]
    }
    return sortRows(pool, s.key, s.dir, (r, k) =>
      k === 'symbol' ? String(r.symbol || '').toLowerCase() : k === 'exchange' ? String(r.exchange || '').toLowerCase() : k === 'asOf' ? String(r.asOf || '') : ((r as unknown as Record<string, number | null>)[k] == null ? -Infinity : (r as unknown as Record<string, number>)[k]),
    )
  })

  const emptyText = $derived.by(() => {
    const q = query.trim().toUpperCase()
    const f = q ? found[q] : null
    if (feed.loading) return 'Reading…'
    if (q) return f && f.loading ? 'Reading ' + q + '…' : f && f.missing ? 'No short interest is reported for ' + q + '.' : 'No listing by that name.'
    return 'Nothing reported yet.'
  })

  function pickScope(v: string) {
    scope = v
    try { localStorage.setItem('bh2.shorts', v) } catch { /* ignore */ }
  }
  function openRow(r: ShortsFeedRow) {
    if (r.positionId) return void (location.hash = 'portfolio/' + encodeURIComponent(r.positionId))
    location.hash = 'markets/' + encodeURIComponent('listing:' + bareSymbol(r.symbol).toUpperCase() + '@' + String(r.exchange || '').toUpperCase())
  }
</script>

<div class="card elev-sm" style="padding:14px 16px 12px">
  <div style="display:flex;align-items:center;gap:14px;min-height:28px;margin-bottom:8px">
    <h5>Short interest</h5>
    <Mseg options={SCOPE_OPTS} cur={scope} onpick={pickScope} />
    <div style="margin-left:auto;display:flex;align-items:center;gap:7px;padding:4px 9px;width:200px;border-radius:6px;background:var(--field);box-shadow:inset 0 0 0 1px rgba(var(--ink-rgb),.1)">
      <input bind:value={query} placeholder="Search any symbol" aria-label="Search short interest" autocomplete="off" style="flex:1;min-width:0;border:0;background:transparent;outline:none;font:400 12.5px var(--font);color:var(--ink)" />
    </div>
  </div>
  {#if rows.length}
    <div class="si-head"><GridHead table="shorts" cols={SHORTS_COLS} /></div>
    <div class="scroll si-list" style="max-height:340px">
      {#each rows as r (r.symbol + '@' + String(r.exchange || ''))}
        <div class="si-row go" role="button" tabindex="-1" onclick={() => openRow(r)} onkeydown={(e) => { if (e.key === 'Enter') openRow(r) }}>
          <div><div style="font-size:13px;font-weight:500;overflow:hidden;text-overflow:ellipsis">{r.symbol}</div><div style="font-size:11px;color:var(--ink55);overflow:hidden;text-overflow:ellipsis">{r.name || ''}</div></div>
          <div style="font-size:12px;color:var(--ink55);text-align:right">{r.exchange || ''}</div>
          <div class="tab" style="text-align:right;font-size:13px">{qty(r.shares)}</div>
          <div class="tab" style="text-align:right;font-size:13px">{r.ofFloat == null ? '—' : n2(r.ofFloat, 2) + '%'}</div>
          <div class="tab" style="text-align:right;font-size:13px;color:var(--ink55)">{r.daysToCover == null ? '—' : n2(r.daysToCover, 1)}</div>
          <div class="tab" style="text-align:right;font-size:13px;color:var(--ink55)">{r.volumePct == null ? '—' : n2(r.volumePct, 1) + '%'}</div>
          <div class="tab" style="text-align:right;font-size:11.5px;color:var(--ink55)">{shortDay(r.asOf)}</div>
        </div>
      {/each}
    </div>
  {:else}
    <div class="muted" style="font-size:12px">{emptyText}</div>
  {/if}
</div>
