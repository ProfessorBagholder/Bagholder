<script lang="ts">
  import type { Options } from './model'
  import {
    filters, resetFilters, FIELDS, PRESETS, dateLabel, listSummary, rangeSummary, clearField,
    type ListKey, type RangeKey, type Field,
  } from './filters.svelte'
  import { setFilters, refilter, store } from './state.svelte'
  import { openTicket } from './ticket/ticket.svelte'
  import { goSub } from './router.svelte'
  import { symText, bareSymbol } from './sym'
  import { ICONS } from './icons'
  import { searchSymbols } from './api'

  // `field` opens the popover straight at one field's editor (for a chip's Edit /
  // ledger chipEdit: 'search' → the fields view, any other key → that field). The
  // funnel opens with no `field`, i.e. the fields view.
  let { options, onclose, field }: { options: Options; onclose: () => void; field?: string } = $props()

  // Which field is open: 'fields' is the list-of-fields view (the funnel opens here),
  // otherwise the key of the field being edited. fieldQuery null means "use the
  // committed free-text search"; a string is the text typed in the fields search box.
  let picker = $state<string>(field && field !== 'search' ? field : 'fields')
  let fieldQuery = $state<string | null>(null)
  let valueQuery = $state('')
  let valueHi = $state(0)

  const OPTION_RE = /^\S+ (\d{2})([A-Z]{3})(\d{2}) /

  const active = $derived(picker !== 'fields' ? FIELDS.find((x) => x.key === picker) ?? null : null)

  // ---- close on click outside the popover + its funnel (no scrim, as ledger.html) ----
  let popEl = $state<HTMLDivElement | undefined>()
  function onPointerDown(e: PointerEvent) {
    const wrap = popEl?.parentElement // the position:relative div holding funnel + pop
    if (wrap && !wrap.contains(e.target as Node)) onclose()
  }

  // ---- the book's knowledge of a symbol, for the row's Buy/Sell (ledger tkLookup) ----
  function tkLookup(symbol: string): { securityId: string; kind: string; hasPosition: boolean } {
    const m = store.model
    const pos = (m?.positions ?? []).find((p) => p.symbol === symbol) ?? null
    const t = pos ?? (m?.trades ?? []).find((x) => x.symbol === symbol) ?? null
    return { securityId: (t?.securityId as string) || '', kind: (t?.kind as string) || '', hasPosition: !!pos }
  }

  // ---- external symbol search ----
  // When the typed text matches nothing in the book, the reference page looks the
  // ticker up at Yahoo (never at Wealthsimple) and lists what it finds. The lookup
  // is scheduled imperatively from the input handler (extSchedule, like ledger.html)
  // — deterministic per keystroke — and the results land in flat $state tagged with
  // the query they belong to, so the rows derived tracks them and shows the matches.
  let extResults = $state<{ q: string; rows: SymRow[] }>({ q: '', rows: [] })
  let extTimer: ReturnType<typeof setTimeout> | undefined
  function extSchedule(v: string) {
    clearTimeout(extTimer)
    const q = v.trim().toUpperCase()
    if (!q) {
      extResults = { q: '', rows: [] }
      return
    }
    // an answer already had comes back at once (api.ts keeps them); a new one waits for a pause in the typing
    extTimer = setTimeout(async () => {
      const rows = await searchSymbols(q)
      if (rows.length || q === v.trim().toUpperCase()) extResults = { q, rows }
    }, 300)
  }
  function extRows(q: string): SymRow[] {
    const Q = q.trim().toUpperCase()
    if (!Q || extResults.q !== Q) return []
    const held = new Set((options.symbols ?? []).map((v) => bareSymbol(v) + '|' + String(options.listings?.[v]?.exchange ?? '').toUpperCase()))
    return extResults.rows.filter((r) => !held.has(bareSymbol((r.symbol ?? r.sym) as string) + '|' + String(r.exchange ?? '').toUpperCase()))
  }

  interface SymRow { book?: boolean; sym?: string; symbol?: string; name?: string; exchange?: string; currency?: string; kind?: string; rank?: number; sub?: string }
  interface MergedSym { book: boolean; sym: string; name: string; exchange: string; currency: string; kind: string; rank: number; sub: string }

  function contractExpiry(sym: string): string {
    const m = OPTION_RE.exec(sym)
    if (!m) return ''
    return '20' + m[3] + String('JANFEBMARAPRMAYJUNJULAUGSEPOCTNOVDEC'.indexOf(m[2]) / 3 + 1).padStart(2, '0') + m[1]
  }
  // Book symbols + external matches, one order of relevance (ledger symbolRows()).
  function symbolRows(q: string): MergedSym[] {
    const Q = q.trim().toUpperCase()
    if (!Q) return []
    const B = bareSymbol(Q)
    const listings = options.listings ?? {}
    const rank = (sym: string, name?: string) => {
      const S = String(sym).toUpperCase()
      if (S === Q || bareSymbol(S) === Q || bareSymbol(S) === B) return 0
      if (S.startsWith(Q) || S.startsWith(B)) return 1
      return S.indexOf(Q) >= 0 || String(name ?? '').toUpperCase().indexOf(Q) >= 0 ? 2 : -1
    }
    const out: MergedSym[] = []
    ;(options.symbols ?? []).forEach((sym) => {
      const l = listings[sym] ?? {}
      if (l.kind === 'Options' || OPTION_RE.test(sym)) {
        const under = String(sym).split(' ')[0].toUpperCase()
        if (bareSymbol(under) === Q || under.startsWith(Q) || String(sym).toUpperCase().indexOf(Q) >= 0)
          out.push({ book: true, sym, name: '', exchange: l.exchange ?? '', currency: l.currency ?? '', kind: l.kind ?? '', rank: 3, sub: contractExpiry(sym) + sym })
        return
      }
      const r = rank(sym, l.name)
      if (r >= 0) out.push({ book: true, sym, name: l.name ?? '', exchange: l.exchange ?? '', currency: l.currency ?? '', kind: l.kind ?? '', rank: r, sub: bareSymbol(sym) })
    })
    extRows(q).forEach((r) => {
      const s = (r.symbol ?? r.sym) as string
      const k = r.rank != null ? r.rank : rank(s, r.name)
      out.push({ book: false, sym: s, name: r.name ?? '', exchange: r.exchange ?? '', currency: r.currency ?? '', kind: r.kind ?? '', rank: k < 0 ? 2 : k, sub: s })
    })
    out.sort((a, b) => a.rank - b.rank || (a.book === b.book ? 0 : a.book ? -1 : 1) || (a.sub < b.sub ? -1 : a.sub > b.sub ? 1 : 0))
    return out
  }
  // Values from every list field containing the typed text, symbols excluded here.
  interface OtherMatch { key: string; value: string; label: string }
  function fieldMatches(q: string): OtherMatch[] {
    const Q = q.trim().toUpperCase()
    if (!Q) return []
    const out: OtherMatch[] = []
    for (const x of FIELDS) {
      if (x.kind !== 'list' || x.key === 'symbol') continue
      const vals = ((options[x.opt as keyof Options] as string[]) ?? []).slice()
      if (x.key === 'tag') vals.push('untagged')
      for (const v of vals) if (String(v).toUpperCase().indexOf(Q) >= 0 && out.length < 40) out.push({ key: x.key, value: v, label: x.label })
    }
    return out
  }

  // ---- derived views ----
  // The book's symbols the text is a ticker of, then the values it names (an account, a
  // grade, a tag), then everything matched by a company's name: the book's own first, the
  // listings found outside it after. A name is the weakest match, so a holding whose name
  // merely contains the text never stands above the account the text spells, and a run of
  // same-named companies never pushes that account out of the scroll box.
  const fieldsQ = $derived(fieldQuery == null ? filters.search : fieldQuery)
  const syms = $derived(fieldsQ ? symbolRows(fieldsQ) : [])
  const byTicker = (r: MergedSym, q: string) => {
    const Q = q.trim().toUpperCase()
    const S = r.sym.toUpperCase()
    return r.rank !== 2 || S.indexOf(Q) >= 0 || S.indexOf(bareSymbol(Q)) >= 0
  }
  const bookSyms = $derived(syms.filter((r) => r.book && byTicker(r, fieldsQ)))
  const extSyms = $derived(syms.filter((r) => r.book && !byTicker(r, fieldsQ)).concat(syms.filter((r) => !r.book)))
  const others = $derived(fieldMatches(fieldsQ).filter((m) => m.key !== 'symbol'))
  const matchTotal = $derived(bookSyms.length + others.length + extSyms.length)
  const hasMatches = $derived(!!fieldsQ && matchTotal > 0)

  const valueOpts = $derived.by(() => {
    if (!active || active.kind !== 'list') return { opts: [] as string[], shown: [] as string[] }
    const opts = ((options[active.opt as keyof Options] as string[]) ?? []).slice()
    if (active.key === 'tag') opts.push('untagged')
    const q = valueQuery.trim().toUpperCase()
    const shown = q ? opts.filter((v) => String(v).toUpperCase().indexOf(q) >= 0) : opts
    return { opts, shown }
  })

  const yearsAvail = $derived((options.years ?? []).slice(0, 8))

  // ---- actions (ledger act() cases) ----
  function pickField(key: string) { picker = key; valueQuery = ''; valueHi = 0 }
  function openFields() { picker = 'fields'; fieldQuery = null; valueHi = 0 }
  function closeFilter() { onclose() }
  // ledger clearAll resets the filters and sets picker=null, which closes the popover.
  // resetFilters() keeps the benchmark, as the original does (benchmark is not a filter).
  function clearAll() { resetFilters(); refilter(); onclose() }
  function clearFieldAct(key: string) { clearField(key); refilter() }

  function toggleList(key: string, value: string) {
    const on = filters.lists[key as ListKey]
    const i = on.indexOf(value)
    if (i >= 0) on.splice(i, 1)
    else on.push(value)
    // a value picked from the fields search stays checked; the box clears so more can be picked
    if (hasMatches && filters.search) filters.search = ''
    refilter()
  }
  function toggleYear(y: string) {
    const i = filters.years.indexOf(y)
    if (i >= 0) filters.years.splice(i, 1)
    else filters.years.push(y)
    setFilters({ from: '', to: '', preset: 'all' })
  }
  function preset(p: string) { setFilters({ preset: filters.preset === p ? 'all' : p, years: [], from: '', to: '' }) }
  function rangeOp(key: RangeKey, op: string) { filters.ranges[key].op = op; if (filters.ranges[key].v != null) refilter() }
  function rangeStep(key: RangeKey, v: number) { filters.ranges[key].v = String(filters.ranges[key].v) === String(v) ? null : v; refilter() }

  let rangeTimer: ReturnType<typeof setTimeout> | undefined
  function rangeInput(key: RangeKey, raw: string) {
    clearTimeout(rangeTimer)
    const v = raw.trim()
    rangeTimer = setTimeout(() => {
      let n = v === '' ? null : Number(v.replace(/[$,]/g, ''))
      if (v !== '' && n != null && isNaN(n)) n = null
      filters.ranges[key].v = n
      refilter()
    }, 500)
  }

  function listingOpen(symbol: string, exchange: string) {
    onclose()
    goSub('markets', 'listing:' + bareSymbol(symbol) + '@' + String(exchange || '').toUpperCase())
  }
  function trade(symbol: string, side: 'BUY' | 'SELL', exchange: string, securityId: string) {
    openTicket(symbol, side, exchange, securityId)
    onclose()
  }

  // the fields search box: typing lists matches; Enter with no match commits free text
  function onFieldsInput(v: string) { fieldQuery = v; valueHi = 0; extSchedule(v) }
  function onFieldsEnter() {
    if (!matchTotal) { filters.search = (fieldQuery ?? '').trim(); fieldQuery = null; refilter(); return }
    const i = Math.min(valueHi, matchTotal - 1)
    if (i < bookSyms.length) {
      const r = bookSyms[i]
      if (OPTION_RE.test(r.sym)) toggleList('symbol', r.sym)
      else listingOpen(bareSymbol(r.sym), r.exchange)
    } else if (i < bookSyms.length + others.length) {
      const m = others[i - bookSyms.length]
      toggleList(m.key, m.value)
    } else {
      const r = extSyms[i - bookSyms.length - others.length]
      listingOpen(r.book ? bareSymbol(r.sym) : r.sym, r.exchange)
    }
  }
  function onFieldsKey(e: KeyboardEvent) {
    const total = matchTotal
    if ((e.key === 'ArrowDown' || e.key === 'ArrowUp') && total) {
      e.preventDefault()
      valueHi = (Math.min(valueHi, total - 1) + (e.key === 'ArrowDown' ? 1 : -1) + total) % total
    } else if (e.key === 'Enter') { e.preventDefault(); onFieldsEnter() }
    else if (e.key === 'Escape') onclose()
  }
  function onValueKey(e: KeyboardEvent) {
    const rows = valueOpts.shown
    if ((e.key === 'ArrowDown' || e.key === 'ArrowUp') && rows.length) {
      e.preventDefault()
      valueHi = (Math.min(valueHi, rows.length - 1) + (e.key === 'ArrowDown' ? 1 : -1) + rows.length) % rows.length
    } else if (e.key === 'Enter' && active) {
      e.preventDefault()
      const v = rows[Math.min(valueHi, rows.length - 1)]
      if (v == null) return
      if (active.key === 'symbol' && !OPTION_RE.test(v)) { const l = options.listings?.[v] ?? {}; listingOpen(bareSymbol(v), l.exchange ?? '') }
      else toggleList(active.key, v)
    } else if (e.key === 'Escape') onclose()
  }

  const summaryFor = (x: Field): string =>
    x.kind === 'date' ? (dateLabel() !== 'All time' ? dateLabel() : '') : x.kind === 'list' ? listSummary(x.key as ListKey) : rangeSummary(x.key as RangeKey)
  const subheadSummary = $derived.by(() => {
    if (!active) return ''
    return active.kind === 'date' ? dateLabel() : active.kind === 'list' ? (listSummary(active.key as ListKey) || 'Any') : (rangeSummary(active.key as RangeKey) || 'Any')
  })
  const hasValue = $derived.by(() => {
    if (!active) return false
    return active.kind === 'date' ? dateLabel() !== 'All time' : active.kind === 'list' ? filters.lists[active.key as ListKey].length > 0 : filters.ranges[active.key as RangeKey].v != null
  })
  const stepLabel = (x: Field, v: number) => (x.unit === '$' ? '$' + v.toLocaleString('en-US') : x.unit === 'd' ? v + 'd' : v.toLocaleString('en-US'))
</script>

<svelte:window onpointerdown={onPointerDown} />

<!-- svelte-ignore a11y_no_static_element_interactions -->
<div class="pop elev-md" bind:this={popEl}>
  {#if !active}
    <!-- fields view: search box + matches, or the list of fields -->
    <div style="display:flex;align-items:center;gap:7px;padding:5px 7px;margin-bottom:8px;border-radius:6px;background:var(--n900);box-shadow:inset 0 0 0 1px rgba(var(--ink-rgb),.1)">
      <!-- svelte-ignore a11y_autofocus -->
      <input value={fieldsQ} oninput={(e) => onFieldsInput((e.target as HTMLInputElement).value)} onkeydown={onFieldsKey} placeholder="Search symbol, account, tag…" aria-label="Search" autocomplete="off" autofocus style="flex:1;min-width:0;border:0;background:transparent;outline:none;font:400 12.5px Inter,system-ui;color:var(--ink)" />
    </div>
    <div>
      {#if hasMatches}
        <div class="scroll" style="max-height:300px;display:flex;flex-direction:column;gap:1px">
          {#each bookSyms as r, i (r.sym + '|' + r.exchange + '#' + i)}
            {@const on = filters.lists.symbol.indexOf(r.sym) >= 0}
            {@const cls = (on ? ' on' : '') + (i === valueHi ? ' hi' : '')}
            {@const contract = OPTION_RE.test(r.sym)}
            {@render symbolRow(contract ? r.sym : bareSymbol(r.sym), r.name, r.exchange, r.sym, cls, contract ? undefined : () => listingOpen(bareSymbol(r.sym), r.exchange), contract ? () => toggleList('symbol', r.sym) : undefined, true)}
          {/each}
          {#each others as m, i (m.key + '|' + m.value)}
            {@const on = filters.lists[m.key as ListKey].indexOf(m.value) >= 0}
            <button class="pop-row{(on ? ' on' : '') + (bookSyms.length + i === valueHi ? ' hi' : '')}" tabindex="-1" onclick={() => toggleList(m.key, m.value)}>{m.value}<span style="margin-left:auto;font-size:11px;color:var(--ink55)">{m.label}</span></button>
          {/each}
          {#each extSyms as r, i (r.sym + '|' + r.exchange + '#' + i)}
            {@const hi = bookSyms.length + others.length + i === valueHi ? ' hi' : ''}
            {#if r.book}
              {@render symbolRow(bareSymbol(r.sym), r.name, r.exchange, r.sym, (filters.lists.symbol.indexOf(r.sym) >= 0 ? ' on' : '') + hi, () => listingOpen(bareSymbol(r.sym), r.exchange), undefined, true)}
            {:else}
              {@render extRow(r, hi)}
            {/if}
          {/each}
        </div>
      {:else}
        {#if fieldsQ}<div class="muted" style="padding:2px 7px 8px;font-size:12px">No match.</div>{/if}
        <div class="lbl" style="padding:2px 7px 7px">Filter by</div>
        <div class="scroll" style="max-height:300px;display:flex;flex-direction:column;gap:1px">
          {#each FIELDS as x (x.key)}
            {@const sum = summaryFor(x)}
            <button class="pop-row" onclick={() => pickField(x.key)}>{x.label}{#if sum}<span style="margin-left:auto;font-size:11px;color:var(--accent-300)">{sum}</span>{/if}</button>
          {/each}
        </div>
      {/if}
    </div>
  {:else}
    <!-- one field's editor -->
    <div style="display:flex;align-items:center;gap:6px;margin-bottom:8px">
      <button onclick={openFields} aria-label="Back" style="cursor:pointer;border:0;background:transparent;color:var(--ink60);font:400 13px Inter,system-ui;padding:2px 5px;border-radius:5px">←</button>
      <span class="lbl">{active.label}</span>
      <span style="margin-left:auto;font-size:11px;color:var(--ink55)">{subheadSummary}</span>
      {#if hasValue}<button onclick={() => clearFieldAct(active.key)} style="cursor:pointer;border:0;background:transparent;color:var(--accent);font:400 11px Inter,system-ui;padding:2px 6px;border-radius:5px">Clear</button>{/if}
    </div>

    {#if active.kind === 'list'}
      {#if active.search}
        <div style="display:flex;align-items:center;gap:7px;padding:5px 7px;margin-bottom:6px;border-radius:6px;background:var(--n900);box-shadow:inset 0 0 0 1px rgba(var(--ink-rgb),.1)">
          <!-- svelte-ignore a11y_autofocus -->
          <input value={valueQuery} oninput={(e) => { valueQuery = (e.target as HTMLInputElement).value; valueHi = 0 }} onkeydown={onValueKey} placeholder="Search" aria-label="Search values" autocomplete="off" autofocus style="flex:1;min-width:0;border:0;background:transparent;outline:none;font:400 12.5px Inter,system-ui;color:var(--ink)" />
        </div>
      {/if}
      <div class="scroll" style="max-height:260px;display:flex;flex-direction:column;gap:1px">
        {#if !valueOpts.opts.length}
          <div class="muted" style="padding:6px 7px;font-size:12px">Nothing to choose from yet.</div>
        {:else if !valueOpts.shown.length}
          <div class="muted" style="padding:6px 7px;font-size:12px">No match.</div>
        {:else}
          {#each valueOpts.shown as v, i (v)}
            {@const on = filters.lists[active.key as ListKey].indexOf(v) >= 0}
            {@const cls = (on ? ' on' : '') + (i === valueHi ? ' hi' : '')}
            {#if active.key === 'symbol'}
              {@const contract = OPTION_RE.test(v)}
              {@const l = options.listings?.[v] ?? {}}
              {@render symbolRow(contract ? v : bareSymbol(v), l.name ?? '', l.exchange ?? '', v, cls, contract ? undefined : () => listingOpen(bareSymbol(v), l.exchange ?? ''), contract ? () => toggleList('symbol', v) : undefined, true)}
            {:else}
              <button class="pop-row{cls}" tabindex="-1" onclick={() => toggleList(active.key, v)}>{v}</button>
            {/if}
          {/each}
        {/if}
      </div>
    {:else if active.kind === 'date'}
      <div style="display:flex;gap:4px;margin-bottom:8px">
        {#each PRESETS as p (p[0])}
          <button class="pill{!filters.years.length && !filters.from && !filters.to && filters.preset === p[0] ? ' on' : ''}" style="flex:1" onclick={() => preset(p[0])}>{p[1]}</button>
        {/each}
      </div>
      <div style="display:grid;grid-template-columns:repeat(4,1fr);gap:4px">
        {#each yearsAvail as y (y)}
          <button class="pill{filters.years.indexOf(y) >= 0 ? ' on' : ''}" onclick={() => toggleYear(y)}>{y}</button>
        {/each}
      </div>
      <div style="display:grid;grid-template-columns:minmax(0,1fr) auto minmax(0,1fr);align-items:center;gap:6px;margin-top:7px">
        <input class="input" type="date" value={filters.from} onchange={(e) => { filters.from = (e.target as HTMLInputElement).value; setFilters({ years: [], preset: 'all' }) }} aria-label="From" style="min-height:30px;font-size:12px;padding:4px 8px" />
        <span class="muted" style="font-size:11px">to</span>
        <input class="input" type="date" value={filters.to} onchange={(e) => { filters.to = (e.target as HTMLInputElement).value; setFilters({ years: [], preset: 'all' }) }} aria-label="To" style="min-height:30px;font-size:12px;padding:4px 8px" />
      </div>
    {:else}
      {@const r = filters.ranges[active.key as RangeKey]}
      <div style="display:flex;gap:5px;margin-bottom:7px">
        {#each [['>', 'More than'], ['<', 'Less than']] as o (o[0])}
          <button class="pill{r.op === o[0] ? ' on' : ''}" style="flex:1;font-size:12px;padding:5px 0" onclick={() => rangeOp(active.key as RangeKey, o[0])}>{o[1]}</button>
        {/each}
      </div>
      <div style="display:grid;grid-template-columns:repeat(4,1fr);gap:5px">
        {#each active.steps ?? [] as v (v)}
          <button class="pill{String(r.v) === String(v) ? ' on' : ''}" style="font-size:12px;padding:5px 0" onclick={() => rangeStep(active.key as RangeKey, v)}>{stepLabel(active, v)}</button>
        {/each}
      </div>
      <input class="input" value={r.v == null ? '' : r.v} oninput={(e) => rangeInput(active.key as RangeKey, (e.target as HTMLInputElement).value)} placeholder={active.ph} aria-label="Custom value" style="margin-top:7px;min-height:30px;font-size:12px" inputmode="decimal" />
    {/if}
  {/if}

  <div class="rule-t" style="display:flex;align-items:center;gap:8px;margin-top:10px;padding-top:9px">
    <button class="btn btn-secondary" onclick={clearAll} style="font-size:12px;padding:4px 10px">Clear all</button>
    <button class="btn btn-primary" onclick={closeFilter} style="font-size:12px;padding:4px 10px;margin-left:auto">Done</button>
  </div>
</div>

{#snippet iconSvg(path: string)}<svg width="12" height="12" viewBox="0 0 256 256" fill="currentColor"><path d={path} /></svg>{/snippet}

<!-- The three-slot symbol row (ledger symbolRowHtml + symbolRowButtons): funnel, Buy, Sell.
     onOpen (the row itself) opens the listing; onToggle (a contract) toggles its filter. -->
{#snippet symbolRow(sym: string, name: string, exchange: string, value: string, cls: string, onOpen: (() => void) | undefined, onToggle: (() => void) | undefined, book: boolean)}
  {@const info = book ? tkLookup(value) : { securityId: '', kind: '', hasPosition: false }}
  {@const share = info.kind === 'Shares'}
  {@const filtered = filters.lists.symbol.indexOf(value) >= 0}
  <div class="pop-row{cls}" role="button" tabindex="-1" onclick={() => (onToggle ? onToggle() : onOpen?.())} onkeydown={(e) => { if (e.key === 'Enter' || e.key === ' ') { e.preventDefault(); onToggle ? onToggle() : onOpen?.() } }}>
    <span style="flex:none">{sym}</span>
    {#if name}<span style="min-width:0;overflow:hidden;text-overflow:ellipsis;white-space:nowrap;color:var(--ink55)">{name}</span>{/if}
    <span style="margin-left:auto;font-size:11px;color:var(--ink55);white-space:nowrap">{exchange || ''}</span>
    <span style="flex:none;width:70px;display:inline-flex;justify-content:flex-end">
      <span class="tk-rowbtns">
        <button class="tk-rowbtn funnel{filtered ? ' on' : ''}" aria-label={(filtered ? 'Stop filtering by ' : 'Filter by ') + symText(value)} aria-pressed={filtered} onclick={(e) => { e.stopPropagation(); toggleList('symbol', value) }}>{@render iconSvg(ICONS.funnel)}</button>
        {#if share}<button class="tk-rowbtn buy" aria-label="Buy {symText(value)}" onclick={(e) => { e.stopPropagation(); trade(value, 'BUY', exchange, info.securityId) }}>{@render iconSvg(ICONS.plus)}</button>{:else}<span class="tk-rowbtn off" aria-hidden="true">{@render iconSvg(ICONS.plus)}</span>{/if}
        {#if share && info.hasPosition}<button class="tk-rowbtn sell" aria-label="Sell {symText(value)}" onclick={(e) => { e.stopPropagation(); trade(value, 'SELL', exchange, info.securityId) }}>{@render iconSvg(ICONS.minus)}</button>{:else}<span class="tk-rowbtn off" aria-hidden="true">{@render iconSvg(ICONS.minus)}</span>{/if}
      </span>
    </span>
  </div>
{/snippet}

<!-- An external listing (not in the book): funnel dimmed, Buy live, Sell dimmed;
     an instrument/crypto (r.kind set) has both dimmed. ledger extRowHtml. -->
{#snippet extRow(r: MergedSym, cls: string)}
  <div class="pop-row{cls}" role="button" tabindex="-1" onclick={() => (r.kind ? undefined : listingOpen(r.sym, r.exchange))} onkeydown={(e) => { if ((e.key === 'Enter' || e.key === ' ') && !r.kind) { e.preventDefault(); listingOpen(r.sym, r.exchange) } }}>
    <span style="flex:none">{r.sym}</span>
    {#if r.name}<span style="min-width:0;overflow:hidden;text-overflow:ellipsis;white-space:nowrap;color:var(--ink55)">{r.name}</span>{/if}
    <span style="margin-left:auto;font-size:11px;color:var(--ink55);white-space:nowrap">{r.exchange || ''}</span>
    <span style="flex:none;width:70px;display:inline-flex;justify-content:flex-end">
      <span class="tk-rowbtns">
        {#if r.kind}
          <span class="tk-rowbtn off" aria-hidden="true">{@render iconSvg(ICONS.plus)}</span>
          <span class="tk-rowbtn off" aria-hidden="true">{@render iconSvg(ICONS.minus)}</span>
        {:else}
          <span class="tk-rowbtn off" aria-hidden="true">{@render iconSvg(ICONS.funnel)}</span>
          <button class="tk-rowbtn buy" aria-label="Buy {symText(r.sym)}" onclick={(e) => { e.stopPropagation(); trade(r.sym, 'BUY', r.exchange, '') }}>{@render iconSvg(ICONS.plus)}</button>
          <span class="tk-rowbtn off" aria-hidden="true">{@render iconSvg(ICONS.minus)}</span>
        {/if}
      </span>
    </span>
  </div>
{/snippet}
