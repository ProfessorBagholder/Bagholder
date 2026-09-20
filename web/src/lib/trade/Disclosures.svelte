<script lang="ts">
  // The Disclosures card, a faithful port of ledger.html's disclosuresCardHtml:
  // a filings table narrowed by category (mseg) and source (chip), sorted by
  // column, with each row's title and summary read in the background. Columns and
  // controls appear only when they carry more than one value.
  import type { Trade, Filing } from '../model'
  import { relTime } from '../fmt'
  import { ICONS } from '../icons'
  import { sort, toggleSort, sortRows } from '../sort.svelte'
  import {
    DISC_ORDER,
    discStore,
    discVersion,
    discSymbol,
    ensureDisclosures,
    refreshDisclosures,
    sweepDiscEnrich,
    markDiscOpen,
    markDiscClosed,
    enrichLoading,
    sweepActive,
    preparing,
    titleComing,
    discDate,
    discSortValue,
  } from './discStore.svelte'

  let { trade }: { trade: Trade } = $props()

  const sym = $derived(discSymbol(trade))
  const rec = $derived(discStore[sym])

  // local view state (the originals live on the global `state`)
  let discCat = $state('all')
  let discSource = $state<string | null>(null)
  let discExpanded = $state<string | null>(null)
  let discOpening = $state<string | null>(null)
  let openTimer: ReturnType<typeof setTimeout> | undefined

  $effect(() => {
    ensureDisclosures(trade)
  })
  $effect(() => {
    markDiscOpen(sym)
    return () => markDiscClosed(sym)
  })

  interface Col {
    key: string
    label: string
    w: string
    plain?: boolean
    align?: string
  }

  const view = $derived.by(() => {
    // reference the enrich counter so titles/summaries re-render as they arrive
    void discVersion.n
    if (!rec || rec.loading) return { state: 'loading' as const }
    if (rec.error) return { state: 'error' as const, error: rec.error }
    const p = rec.payload || { ok: true }
    const all = p.filings || []
    const srcStatus = p.sources || {}
    const availNames = Object.keys(srcStatus).filter((k) => srcStatus[k].available)
    const anyFiler = Object.keys(srcStatus).some((k) => srcStatus[k].filer)
    // a source that was tried and could not be reached (an outage, a maintenance page)
    // carries an error; it is unavailable, not proof the listing has no filer
    const downNames = Object.keys(srcStatus).filter((k) => srcStatus[k].error)
    if (!all.length) {
      const msg = anyFiler ? 'Nothing filed.'
        : downNames.length ? downNames.join(' · ') + (downNames.length > 1 ? ' are unavailable.' : ' is unavailable.')
        : 'No regulatory filer for this listing.'
      const names = anyFiler ? availNames.join(' · ') : ''
      const right = [names, p.fetchedAt ? 'read ' + relTime(p.fetchedAt) : ''].filter(Boolean).join(' · ')
      return { state: 'empty' as const, right, msg }
    }
    const catsAll = DISC_ORDER.filter((c) => all.some((f) => f.category === c))
    const effCat = catsAll.indexOf(discCat) >= 0 ? discCat : 'all'
    const multiCat = catsAll.length > 1
    const multiSource = !discSource && new Set(all.map((f) => f.source)).size > 1
    const anySize = all.some((f) => f.size)
    const oneSource = discSource || (new Set(all.map((f) => f.source)).size === 1 ? all[0].source : '')

    let rows = all.slice()
    if (discSource) rows = rows.filter((f) => f.source === discSource)
    if (effCat !== 'all') rows = rows.filter((f) => f.category === effCat)
    const s = sort.disc
    rows = sortRows(rows, s.key, s.dir, discSortValue)

    const cols: Col[] = [
      { key: 'date', label: 'Date', w: '88px' },
      { key: 'document', label: 'Document', w: 'minmax(120px,0.6fr)' },
      { key: 'title', label: 'Title', w: 'minmax(0,1.3fr)', plain: true },
      { key: 'summary', label: 'Summary', w: 'minmax(0,2fr)', plain: true },
    ]
    if (multiSource) cols.push({ key: 'source', label: 'Source', w: '84px' })
    if (anySize) cols.push({ key: 'size', label: 'Size', w: '66px', align: 'right' })
    const tmpl = cols.map((c) => c.w).join(' ') + ' 22px'
    const rightText = [!multiSource && !discSource && oneSource ? oneSource : '', p.fetchedAt ? 'read ' + relTime(p.fetchedAt) : ''].filter(Boolean).join(' · ')
    return { state: 'list' as const, all, rows, cols, tmpl, effCat, catsAll, multiCat, multiSource, anySize, rightText }
  })

  // the background read of each row's title and summary
  $effect(() => {
    if (view.state === 'list') sweepDiscEnrich(sym, view.rows)
  })

  function toggleExpand(id: string) {
    discExpanded = discExpanded === id ? null : id
  }
  function pickSource(source: string) {
    discSource = source
  }
  function clearSource() {
    discSource = null
  }
  function openDoc(f: Filing) {
    discOpening = f.id
    const durl = f.source === 'SEC' && f.url ? f.url : '/api/filings/doc?symbol=' + encodeURIComponent(sym) + '&id=' + encodeURIComponent(f.id)
    window.open(durl, '_blank', 'noopener')
    clearTimeout(openTimer)
    openTimer = setTimeout(() => (discOpening = null), 4000)
  }
  function reread() {
    refreshDisclosures(sym, trade)
  }

  const catOpts = $derived(view.state === 'list' ? ([['all', 'All']] as [string, string][]).concat(view.catsAll.map((c) => [c, c] as [string, string])) : [])
</script>

{#snippet gth(table: string, c: Col)}
  {@const on = sort[table].key === c.key}
  {@const right = c.align === 'right'}
  {#if c.plain}
    <div class="gth" style="text-align:{c.align || 'left'}"><span class="th-in">{c.label}</span></div>
  {:else}
    <!-- svelte-ignore a11y_click_events_have_key_events a11y_no_static_element_interactions -->
    <div class="gth" class:on onclick={() => toggleSort(table, c.key)} style="text-align:{c.align || 'left'}">
      <span class="th-in" style="flex-direction:{right ? 'row-reverse' : 'row'}">{c.label}<span class="arrow" style="color:{on ? 'var(--accent)' : 'transparent'}">{on && sort[table].dir === 'asc' ? '▲' : '▼'}</span></span>
    </div>
  {/if}
{/snippet}

{#snippet readBtn()}
  <button onclick={reread} aria-label="Re-read disclosures" style="display:grid;place-items:center;width:24px;height:24px;padding:0;border:0;border-radius:6px;background:transparent;color:var(--ink55);cursor:pointer"><svg width="14" height="14" viewBox="0 0 256 256" fill="currentColor"><path d={ICONS.sync} /></svg></button>
{/snippet}

<div class="card elev-sm" style="padding:14px 16px 8px">
  {#if view.state === 'loading'}
    <div style="display:flex;align-items:center;gap:12px;min-height:24px;margin-bottom:8px"><h5>Disclosures</h5><span style="font-size:11px;color:var(--ink55)">Reading disclosures…</span></div>
    <div class="dc-sweep"></div>
    <div class="dc-list">
      {#each Array(6) as _, i (i)}
        <div class="dc-row" style="grid-template-columns:92px minmax(0,0.9fr) minmax(0,1.25fr) minmax(0,1.7fr) 22px;pointer-events:none">
          {#each ['55%', '60%', '78%', '90%'] as w (w)}<div class="dc-skel" style="width:{w}"></div>{/each}
          <div></div>
        </div>
      {/each}
    </div>
  {:else if view.state === 'error'}
    <div style="display:flex;align-items:center;gap:12px;min-height:24px;margin-bottom:8px"><h5>Disclosures</h5><span style="margin-left:auto;display:flex;align-items:center;gap:8px;font-size:11px;color:var(--ink55)">{@render readBtn()}</span></div>
    <div class="muted" style="font-size:12px;padding:6px 0 10px">{view.error}</div>
  {:else if view.state === 'empty'}
    <div style="display:flex;align-items:center;gap:12px;min-height:24px;margin-bottom:8px"><h5>Disclosures</h5><span style="margin-left:auto;display:flex;align-items:center;gap:8px;font-size:11px;color:var(--ink55)">{#if view.right}<span>{view.right}</span>{/if}{@render readBtn()}</span></div>
    <div class="muted" style="font-size:12px;padding:8px 0 10px">{view.msg}</div>
  {:else if view.state === 'list'}
    <div style="display:flex;align-items:center;gap:12px;min-height:24px;margin-bottom:8px">
      <h5>Disclosures</h5>
      {#if view.multiCat}
        <span class="mseg">
          {#each catOpts as o (o[0])}
            <button class="mseg-opt" class:on={o[0] === view.effCat} onclick={() => (discCat = o[0])}>{o[1]}</button>
          {/each}
        </span>
      {/if}
      {#if discSource}
        <span class="chip"><span class="cf">Source</span><span class="cv" style="cursor:default">{discSource}</span><button class="cx" onclick={clearSource} aria-label="Show every source">×</button></span>
      {/if}
      <span style="margin-left:auto;display:flex;align-items:center;gap:8px;font-size:11px;color:var(--ink55)">{#if view.rightText}<span>{view.rightText}</span>{/if}{@render readBtn()}</span>
    </div>
    <div class="dc-head" style="grid-template-columns:{view.tmpl}">
      {#each view.cols as c (c.key)}{@render gth('disc', c)}{/each}
      <div></div>
    </div>
    <div class="scroll dc-list" style="max-height:330px">
      {#each view.rows as f (f.id)}
        {@const enriching = enrichLoading(f.id) || sweepActive(sym, f)}
        <!-- svelte-ignore a11y_click_events_have_key_events a11y_no_static_element_interactions -->
        <div class="dc-row" class:on={discExpanded === f.id} onclick={() => toggleExpand(f.id)} style="grid-template-columns:{view.tmpl}">
          <div class="dc-date">{discDate(f)}</div>
          <div class="dc-doc"><b>{f.type}</b></div>
          <div class="dc-title">{#if f.subject}{f.subject}{:else if enriching || titleComing(f, sym)}<span class="dc-skel" style="width:70%"></span>{/if}</div>
          <div class="dc-sumcell">{#if f.summary}{f.summary}{:else if enriching || preparing(f)}<span class="dc-skel" style="width:90%"></span>{/if}</div>
          {#if view.multiSource}<button class="dc-src" onclick={(e) => { e.stopPropagation(); pickSource(f.source) }}>{f.source}</button>{/if}
          {#if view.anySize}<div style="font-size:11.5px;color:var(--ink55);text-align:right;font-variant-numeric:tabular-nums;padding-top:1px">{f.size || ''}</div>{/if}
          <button class="dc-open" class:lit={discOpening === f.id} onclick={(e) => { e.stopPropagation(); openDoc(f) }} aria-label="Open document">{discOpening === f.id ? '…' : '↗'}</button>
        </div>
      {/each}
    </div>
  {/if}
</div>
