<script lang="ts">
  // The trade / holding / listing detail — the largest screen. A faithful port of
  // ledger.html's tradeDetailHtml + listingDetailHtml: the header card (symbol,
  // name·exchange, ticket buttons, the big P&L or a listing's price), the
  // timeframe pills + candlestick chart with the executions marked, a two-column
  // grid of the facts + executions table and the thesis / grade / tags card, then
  // the short-interest cards and the disclosures list.
  import type { Trade, Fill } from './model'
  import { money, money0, pct, px, qty, hold, color, localWhen, waiting } from './fmt'
  import { waits } from './dec'
  import { symText } from './sym'
  import { ICONS } from './icons'
  import { sort, toggleSort, sortRows } from './sort.svelte'
  import { store, saveJournal, server, detail } from './state.svelte'
  import { openTicket } from './ticket/ticket.svelte'
  import { chartColors, chartTfFor, setChartTf, listingTicker, loadHistory, historyKey, TIMEFRAMES, type Bar, type History } from './trade/chart'
  import { watchDoc } from './live'
  import { tradeChart } from './actions/tradeChart'
  import EventEntry from './EventEntry.svelte'
  import ShortInterest from './trade/ShortInterest.svelte'
  import Disclosures from './trade/Disclosures.svelte'

  let { trade }: { trade: Trade } = $props()

  // a corporate event this holding waits on, and that only the person can say what it did
  const waitingEvent = $derived((store.model?.waiting ?? []).find((w) => w.what === 'event' && w.instrument === trade.instrument && w.account === trade.accountId) ?? null)

  const signedPct = (v: number | null | undefined) => (v == null || !isFinite(v) ? '—' : (v < 0 ? '−' : '+') + Math.abs(v).toFixed(2) + '%')

  // ---- the chart: mount the wanted timeframe, falling back a step coarser when a
  // timeframe is not offered, and showing the daily chart while minute data loads.
  // a listing brings its own fills (none); a trade's or a holding's are asked for when it opens
  const fills = $derived(trade.fills ?? (detail.id === trade.id ? detail.fills : undefined))
  let loaded = $state<{ tf: string; hist: History; provisional: boolean } | null>(null)
  let wantedTf = $state('')

  $effect(() => {
    const t = trade
    if (fills === undefined) return // the trade's fills are still on their way
    void server.restarts // a server started again is asked again
    const wanted = wantedTf || chartTfFor(t)
    let cancelled = false
    const closing = new AbortController() // bars nobody is waiting for any more are not read
    let stopWatching: (() => void) | undefined
    const mount = async (want: string) => {
      const h = await loadHistory(t, want, closing.signal)
      if (cancelled) return
      const available = h.available
      const tf = chartTfFor(t, available)
      if (tf && tf !== want) {
        mount(tf)
        return
      }
      if (h.pending) {
        // the bars stored so far are drawn while newer ones are read; with none
        // stored for minute bars, the daily chart stands in
        if (h.bars.length) loaded = { tf, hist: h, provisional: true }
        else if (want !== '1d' && available.indexOf('1d') >= 0) {
          const d = await loadHistory(t, '1d', closing.signal)
          if (!cancelled) loaded = { tf: '1d', hist: d, provisional: true }
        }
        // the minute bars are being read: be told when they are in, and ask once more
        // then -- not every three seconds until they are
        let pending = true
        const flag = {
          get pending() { return pending },
          set pending(v: boolean) {
            pending = v
            if (v || cancelled) return
            stopWatching?.()
            if (chartTfFor(t) === want) mount(want)
          },
        }
        stopWatching?.()
        stopWatching = watchDoc(historyKey(t, want), {}, { data: flag })
        return
      }
      loaded = { tf, hist: h, provisional: false }
    }
    mount(wanted)
    return () => {
      cancelled = true
      closing.abort()
      stopWatching?.()
    }
  })

  const candles = $derived(!!loaded && loaded.hist.bars.length > 0 && loaded.hist.bars.every((b: Bar) => b.open != null && b.high != null && b.low != null))
  const chartFills = $derived(fills || [])
  const pillsAvailable = $derived(loaded ? loaded.hist.available : [])

  function pickTf(tf: string) {
    setChartTf(trade.id, tf)
    wantedTf = tf
  }

  // ---- executions ----
  function execSortValue(e: Fill, key: string): unknown {
    if (key === 'when') return e.when
    if (key === 'side') return e.side
    if (key === 'qty') return e.qty
    if (key === 'currency') return e.currency
    if (key === 'price') return e.price
    return e.amount
  }
  const ecols = [
    { key: 'when', label: 'When', padLeft: '0' },
    { key: 'side', label: 'Side' },
    { key: 'qty', label: 'Qty', align: 'right' },
    { key: 'currency', label: 'FX', align: 'center', padLeft: '18px', padRight: '18px' },
    { key: 'price', label: 'Price', align: 'right' },
    { key: 'amount', label: 'Amount', align: 'right', padRight: '0' },
  ] as { key: string; label: string; align?: string; padLeft?: string; padRight?: string }[]
  const execs = $derived(sortRows(fills || [], sort.execs.key, sort.execs.dir, execSortValue))

  // ---- ticket buttons (ticketButtonsHtml) ----
  interface TkBtn { side: 'BUY' | 'SELL'; on: boolean; open?: () => void }
  const tkButtons = $derived.by<TkBtn[] | null>(() => {
    if (String(trade.kind || 'Shares') !== 'Shares') return null
    if (trade.listing) {
      return [
        { side: 'BUY', on: true, open: () => openTicket(trade.symbol, 'BUY', trade.exchange || '') },
        { side: 'SELL', on: false },
      ]
    }
    // this page's own holding, by its id (the holding's page, or the holding an open trade
    // is): never the same symbol held in another account
    const pos = trade.position ? (store.model?.positions || []).find((p) => p.id === trade.position) ?? null : null
    const holding = pos?.id ?? ''
    return [
      { side: 'BUY', on: true, open: () => openTicket(trade.symbol, 'BUY', trade.exchange || '', trade.security, holding) },
      { side: 'SELL', on: !!pos, open: () => openTicket(trade.symbol, 'SELL', trade.exchange || '', trade.security, holding) },
    ]
  })

  // ---- journal: thesis, grade, tags ----
  // svelte-ignore state_referenced_locally
  let thesisDraft = $state(trade.thesis || '')
  let thesisTimer: ReturnType<typeof setTimeout> | undefined
  // svelte-ignore state_referenced_locally
  let lastId = trade.id
  $effect(() => {
    if (trade.id !== lastId) {
      lastId = trade.id
      thesisDraft = trade.thesis || ''
    }
  })
  function thesisInput() {
    clearTimeout(thesisTimer)
    thesisTimer = setTimeout(() => {
      if (trade.thesis !== thesisDraft) saveJournal(trade.journal ?? trade.id, { thesis: thesisDraft })
    }, 600)
  }
  function thesisBlur() {
    clearTimeout(thesisTimer)
    if (trade.thesis !== thesisDraft) saveJournal(trade.journal ?? trade.id, { thesis: thesisDraft })
  }
  function setGrade(g: string) {
    saveJournal(trade.journal ?? trade.id, { grade: trade.grade === g ? '' : g })
  }
  function removeTag(tag: string) {
    saveJournal(trade.journal ?? trade.id, { tags: (trade.tags || []).filter((x) => x !== tag) })
    tagEl?.focus()
  }

  let tagDraft = $state('')
  let tagHi = $state(0)
  let tagEl = $state<HTMLInputElement | null>(null)
  const tagMatches = $derived.by(() => {
    const draft = tagDraft.trim().toLowerCase()
    if (!draft) return [] as string[]
    return ((store.model?.options?.tags || []) as string[])
      .filter((x) => (trade.tags || []).indexOf(x) < 0 && x.toLowerCase().indexOf(draft) >= 0)
      .sort((a, b) => a.toLowerCase().indexOf(draft) - b.toLowerCase().indexOf(draft) || a.length - b.length)
      .slice(0, 6)
  })
  function addTag(tag: string) {
    if ((trade.tags || []).indexOf(tag) < 0) saveJournal(trade.journal ?? trade.id, { tags: [...(trade.tags || []), tag] })
    tagDraft = ''
    tagHi = 0
    tagEl?.focus()
  }
  function commitTag() {
    const v = tagDraft.trim().replace(/,$/, '')
    tagDraft = ''
    if (v && (trade.tags || []).indexOf(v) < 0) {
      saveJournal(trade.journal ?? trade.id, { tags: [...(trade.tags || []), v] })
      tagEl?.focus()
    }
  }
  function tagKey(e: KeyboardEvent) {
    const matches = tagMatches
    if (e.key === 'Backspace' && (e.target as HTMLInputElement).value === '') {
      const tags = trade.tags || []
      if (tags.length) {
        e.preventDefault()
        removeTag(tags[tags.length - 1])
      }
      return
    }
    if (e.key === 'ArrowDown' || e.key === 'ArrowUp') {
      if (!matches.length) return
      e.preventDefault()
      tagHi = (Math.min(tagHi, matches.length - 1) + (e.key === 'ArrowDown' ? 1 : -1) + matches.length) % matches.length
      return
    }
    if (e.key === 'Enter' || e.key === 'Tab' || e.key === ',') {
      if (matches.length && e.key !== ',') {
        e.preventDefault()
        addTag(matches[Math.min(tagHi, matches.length - 1)])
        return
      }
      if (e.key !== 'Tab') e.preventDefault()
      commitTag()
    }
  }

  const colors = $derived(chartColors())
</script>

{#snippet tkbtns()}
  {#if tkButtons}
    <span class="tk-rowbtns">
      {#each tkButtons as b (b.side)}
        {#if b.on}
          <button class="tk-rowbtn {b.side.toLowerCase()}" aria-label="{b.side === 'BUY' ? 'Buy' : 'Sell'} {symText(trade.symbol)}" onclick={b.open}><svg width="12" height="12" viewBox="0 0 256 256" fill="currentColor"><path d={b.side === 'BUY' ? ICONS.plus : ICONS.minus} /></svg></button>
        {:else}
          <span class="tk-rowbtn off" aria-hidden="true"><svg width="12" height="12" viewBox="0 0 256 256" fill="currentColor"><path d={b.side === 'BUY' ? ICONS.plus : ICONS.minus} /></svg></span>
        {/if}
      {/each}
    </span>
  {/if}
{/snippet}

{#snippet fact(l: string, v: string)}
  <div><div class="lbl">{l}</div><div class="tab" style="font-size:13px">{v}</div></div>
{/snippet}

<div style="padding:20px;min-height:380px;display:flex;flex-direction:column;gap:14px">
  <!-- header + chart card -->
  <div class="card elev-sm" style="padding:16px 18px 12px">
    <div style="display:flex;align-items:flex-start;gap:14px;margin:0 0 14px">
      <div style="min-width:0">
        <h4>{symText(trade.symbol)}</h4>
        <div class="dim" style="font-size:11.5px;margin-top:2px">{trade.name || trade.symbol}{trade.exchange ? ' · ' + trade.exchange + ': ' + listingTicker(trade) : ''}{trade.flags?.includes('entered') ? ' · Entered by you' : ''}</div>
      </div>
      <div style="margin-left:auto;display:flex;align-items:center;gap:12px">
        {@render tkbtns()}
        {#if trade.listing}
          <div style="text-align:right">
            <div class="tab" style="font-size:24px;font-weight:500;line-height:1.1">{trade.last == null ? '—' : px(trade.last)}</div>
            <div class="tab" style="font-size:12px;margin-top:2px;color:{trade.percentChange == null ? 'var(--ink55)' : color(trade.percentChange)}">{signedPct(trade.percentChange)}</div>
          </div>
        {:else}
          <div style="text-align:right">
            <div class="tab" style="font-size:24px;font-weight:500;line-height:1.1;color:{color(trade.pnl)}">{money(trade.pnl, trade.currency)}</div>
            <div class="tab" style="font-size:12px;color:{color(trade.pnl)};margin-top:2px">{pct(trade.pnlPct)}</div>
          </div>
        {/if}
      </div>
    </div>
    <div style="display:flex;gap:4px;justify-content:flex-end;margin:0 0 8px">
      {#if pillsAvailable.length}
        {#each TIMEFRAMES as [k, label] (k)}
          {#if pillsAvailable.indexOf(k) >= 0}
            <button class="pill" class:on={loaded?.tf === k} onclick={() => pickTf(k)} style="padding:2px 8px;font-size:11px;width:auto">{label}</button>
          {/if}
        {/each}
      {/if}
    </div>
    {#if loaded && candles}
      <div use:tradeChart={{ bars: loaded.hist.bars, fills: chartFills, tf: loaded.tf, colors, rangeKey: trade.id + '|' + loaded.tf, provisional: loaded.provisional, open: trade.exitDate == null }} style="position:relative;height:300px"></div>
    {:else}
      <div style="position:relative;height:300px">
        {#if loaded}<div class="muted empty" style="display:flex;align-items:center;justify-content:center;height:100%;font-size:12px;text-align:center;padding:0 24px">{loaded.hist.reason || 'No price history for this span.'}</div>{/if}
      </div>
    {/if}
  </div>

  {#if !trade.listing}
    <!-- facts + executions | thesis/grade/tags -->
    <div style="display:grid;grid-template-columns:minmax(0,1fr) minmax(0,1fr);gap:14px;align-items:stretch;height:372px">
      <div class="card elev-sm" style="padding:16px 18px;display:flex;flex-direction:column;min-height:0">
        <div class="rule-b" style="display:grid;grid-template-columns:repeat(6,minmax(0,1fr));gap:8px;padding:0 0 14px">
          {#if trade.holding}
            {@render fact('Qty', qty(trade.qty))}
            {@render fact('Avg', px(trade.avg ?? null))}
            {@render fact('Book', money0(trade.cost ?? null, trade.currency))}
            {@render fact('Market', money0(trade.mv ?? null, trade.currency))}
          {:else}
            {@render fact('Open', trade.entryDate)}
            {@render fact('Close', trade.exitDate ?? 'Open')}
            {@render fact('Entry', px(trade.entry))}
            {@render fact('Exit', px(trade.exit))}
          {/if}
          {@render fact('Hold', trade.held != null && waits(trade.held) ? waiting(trade.held) : hold(trade.holdDays))}
          {@render fact('Account', trade.account)}
        </div>
        <div style="display:flex;align-items:baseline;gap:8px;margin:14px 0 6px"><span class="lbl">Executions{fills ? ' (' + execs.length + ')' : ''}</span></div>
        <div class="scroll" style="flex:1;min-height:0">
          <table class="table" style="font-size:12.5px;width:100%">
            <thead><tr>
              {#each ecols as c (c.key)}
                {@const on = sort.execs.key === c.key}
                {@const right = c.align === 'right'}
                {@const center = c.align === 'center'}
                <th onclick={() => toggleSort('execs', c.key)} style="white-space:nowrap;text-align:{c.align || 'left'};cursor:pointer;position:sticky;top:0;z-index:1;color:{on ? 'var(--ink)' : 'rgba(var(--ink-rgb),.6)'}{c.padLeft ? ';padding-left:' + c.padLeft : ''}{c.padRight ? ';padding-right:' + c.padRight : ''}">
                  <span class="th-in" style="flex-direction:{right ? 'row-reverse' : 'row'}">{c.label}<span class="arrow" style="color:{on ? 'var(--accent)' : 'transparent'}{center ? ';position:absolute;left:100%;margin-left:4px' : ''}">{on && sort.execs.dir === 'asc' ? '▲' : '▼'}</span></span>
                </th>
              {/each}
            </tr></thead>
            <tbody>
              {#each execs as e (e.id)}
                <tr class="tab">
                  <td style="padding-left:0;color:var(--ink75);white-space:nowrap">{localWhen(e.when, e.date).day} <span class="dim">{localWhen(e.when, e.date).time}</span></td>
                  <td style="padding-inline:6px;font-size:11.5px;letter-spacing:.03em;color:var(--ink60)">{e.sub || e.side}</td>
                  <td style="text-align:right;padding-inline:6px">{qty(e.qty)}</td>
                  <td class="dim" style="text-align:center;padding:7px 18px;font-variant-numeric:normal">{e.currency}</td>
                  <td style="text-align:right;padding-inline:6px;color:var(--ink75)">{px(e.price)}</td>
                  <td style="text-align:right;padding-right:0;color:rgba(var(--ink-rgb),.85)">{money(e.amount, e.currency)}</td>
                </tr>
              {/each}
            </tbody>
          </table>
          {#if detail.id === trade.id && detail.error}<div class="neg" style="padding:12px 0;font-size:12px">{detail.error}</div>{/if}
        </div>
      </div>

      <!-- the row is a fixed height: while an event waits its form takes room from the notes box, and the card scrolls rather than spill onto the page -->
      <div class="card elev-sm" style="padding:16px 18px;display:flex;flex-direction:column;min-height:0;overflow-y:auto">
        <div class="lbl" style="margin:0 0 6px">Thesis / notes</div>
        <textarea class="input" bind:value={thesisDraft} oninput={thesisInput} onblur={thesisBlur} placeholder="Why did you take this trade?" style="flex:1;min-height:{waitingEvent ? 48 : 96}px;height:auto;font-size:13px"></textarea>
        <div style="display:flex;gap:10px;margin-top:16px;align-items:center">
          <div style="flex:1">
            <div class="lbl" style="margin-bottom:5px">Grade</div>
            <div class="seg">
              {#each ['A', 'B', 'C', 'F'] as g (g)}
                <!-- svelte-ignore a11y_click_events_have_key_events a11y_no_noninteractive_element_interactions a11y_label_has_associated_control -->
                <label class="seg-opt" class:on={trade.grade === g} onclick={() => setGrade(g)}>{g}</label>
              {/each}
            </div>
          </div>
        </div>
        <div style="margin-top:14px;position:relative">
          <div class="lbl" style="margin-bottom:6px">Tags</div>
          <!-- svelte-ignore a11y_click_events_have_key_events a11y_no_static_element_interactions -->
          <div class="input" onclick={() => tagEl?.focus()} style="display:flex;flex-wrap:wrap;gap:6px;align-items:center;min-height:36px;height:auto;padding:6px 8px;cursor:text">
            {#each trade.tags || [] as tg (tg)}
              <span style="display:inline-flex;align-items:center;gap:6px;font-size:11px;padding:3px 5px 3px 9px;border-radius:6px;background:var(--chip-bg);color:var(--chip-fg)">{tg}<button onclick={(e) => { e.stopPropagation(); removeTag(tg) }} aria-label="Remove tag" style="display:grid;place-items:center;width:15px;height:15px;padding:0;border:0;border-radius:4px;background:rgba(var(--ink-rgb),.1);color:var(--chip-fg);cursor:pointer;font-size:11px;line-height:1">×</button></span>
            {/each}
            <input id="tagInput" bind:this={tagEl} bind:value={tagDraft} oninput={() => (tagHi = 0)} onkeydown={tagKey} placeholder={(trade.tags || []).length ? 'Add another…' : 'Add a tag…'} aria-label="Add tag" style="flex:1;min-width:90px;border:0;background:transparent;font:400 12.5px var(--font);color:var(--ink);outline:none" autocomplete="off" />
          </div>
          {#if tagDraft.trim() && tagMatches.length}
            {@const hiI = Math.min(tagHi, tagMatches.length - 1)}
            <div style="position:absolute;left:0;right:0;bottom:100%;margin-bottom:4px;z-index:5;border-radius:8px;background:var(--n900);box-shadow:var(--shadow-md);padding:5px;display:flex;flex-direction:column;gap:1px">
              {#each tagMatches as tg, i (tg)}
                <button onmousedown={(e) => { e.preventDefault(); addTag(tg) }} style="display:flex;align-items:center;gap:8px;width:100%;text-align:left;cursor:pointer;border:0;font:400 12.5px var(--font);padding:6px 8px;border-radius:6px;background:{i === hiI ? 'var(--chip-hi)' : 'transparent'};color:{i === hiI ? 'var(--chip-fg)' : 'rgba(var(--ink-rgb),.85)'}">{tg}<span class="muted" style="margin-left:auto;font-size:10px">{i === hiI ? 'Tab ↵' : ''}</span></button>
              {/each}
            </div>
          {/if}
        </div>
        {#if waitingEvent && store.model}<EventEntry event={waitingEvent} model={store.model} />{/if}
      </div>
    </div>
  {/if}

  <ShortInterest {trade} />
  <Disclosures {trade} />
</div>
