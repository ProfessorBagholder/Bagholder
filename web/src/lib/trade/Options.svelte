<script lang="ts">
  // The Options card on a US listing's page (SPEC.md markets-123 to markets-130,
  // brief 83): Puts · Calls, one button per expiry, and a row a strike with one
  // contract's Cost, Premium and Yield, every figure the server's; a row opens
  // the contract, its Cost, Premium and premium each day. A listing with no
  // options has no card at all.
  import type { Trade } from '../model'
  import type { OptionExpiry, OptionRow, OptionContract, OptionsAnswer } from '../generated/markets'
  import type { Dec } from '../dec'
  import { digits, plot } from '../dec'
  import { money0 } from '../fmt'
  import { read } from '../reads.svelte'
  import { watchDoc, type Holder } from '../live.svelte'
  import { tell } from '../ui.svelte'
  import { listingTicker } from './chart'
  import { untrack } from 'svelte'

  let { trade }: { trade: Trade } = $props()

  // the chain the server reads as the card shows and every fifteen minutes while it
  // shows (rust docs.rs), sent here as it changes
  const query = $derived({ symbol: String(listingTicker(trade) || trade.symbol || '').toUpperCase(), exchange: trade.exchange || '', currency: trade.currency || '', name: '' })

  let expiries = $state<OptionExpiry[]>([])
  let side = $state<'puts' | 'calls'>('puts')
  let picked = $state('')
  let open = $state<OptionRow | null>(null)
  let contract = $state<OptionContract | null>(null)
  let list = $state<HTMLDivElement | null>(null)

  // the card's document, by its key alone: the page handing over an equal listing
  // (the same address opened again) keeps the watch and what it holds
  const symbol = $derived(query.symbol)
  const key = $derived(query.symbol ? 'options:' + new URLSearchParams({ symbol: query.symbol, exchange: query.exchange, currency: query.currency }).toString() : '')

  $effect(() => {
    const k = key
    const sym = untrack(() => symbol)
    expiries = []
    open = null
    contract = null
    if (!k) return
    const holder: Holder<OptionsAnswer> = { data: null }
    return watchDoc(k, {}, holder, () => {
      const a = holder.data
      if (!a) return
      if (!a.ok) {
        tell('Could not read the options of ' + sym + ': ' + ('error' in a ? a.error : ''), 'err')
        return
      }
      if ('expiries' in a) expiries = structuredClone(a.expiries)
    })
  })

  const expiry = $derived(expiries.find((e) => e.day === picked) ?? expiries[0] ?? null)
  const rows = $derived(expiry ? expiry[side].rows : [])
  const near = $derived(expiry ? expiry[side].near : null)

  // the list opens with the strike nearest the price in view
  $effect(() => {
    const el = list
    const i = near
    void rows
    if (!el || i == null) return
    const row = el.querySelectorAll<HTMLElement>('.oc-row')[i]
    if (row) el.scrollTop = Math.max(0, row.offsetTop - el.offsetTop - (el.clientHeight - row.offsetHeight) / 2)
  })

  // a strike as the card writes it: whole, or to two decimals
  function strike(s: Dec): string {
    const [whole, part] = String(s).split('.')
    return part ? whole + '.' + (part + '00').slice(0, 2) : whole
  }
  const pctText = (v: Dec | null) => (v == null ? '—' : digits(v, 2) + '%')

  async function pick(r: OptionRow): Promise<void> {
    open = r
    contract = null
    const a = await read('GET /api/options/contract', { query: { ...query, occ: r.occ } })
    if (open !== r) return
    if (!a.ok) {
      tell('Could not read ' + r.title + ': ' + ('error' in a ? a.error : ''), 'err')
      return
    }
    if ('contract' in a) {
      contract = a.contract
      if (a.contract.chartFailed) tell(a.contract.chartFailed, 'err')
    }
  }

  function back(): void {
    open = null
    contract = null
  }

  // the chart's geometry, as the mockup draws it: 600 wide, from 140 up to 10
  const chart = $derived.by(() => {
    const c = contract?.chart
    if (!c || !c.points.length) return null
    const lo = plot(c.bottom)
    const hi = plot(c.top)
    const span = hi - lo || 1
    const y = (v: Dec) => Math.round((140 - ((plot(v) - lo) / span) * 130) * 10) / 10
    const n = Math.max(1, c.points.length - 1)
    const points = c.points.map((p, i) => Math.round(((i * 600) / n) * 10) / 10 + ',' + y(p.premium)).join(' ')
    return { points, lastY: y(c.points[c.points.length - 1].premium), top: money0(c.top), middle: money0(c.middle), bottom: money0(c.bottom), from: c.from }
  })
</script>

{#if expiries.length}
  <section class="card elev-sm oc" aria-labelledby="oc-h" data-testid="options-card">
    {#if !open}
      <div class="oc-top">
        <h2 id="oc-h" class="lbl oc-h">Options</h2>
        <div class="mseg oc-sides" role="group" aria-label="Puts or calls">
          <button class="mseg-opt" class:on={side === 'puts'} type="button" aria-pressed={side === 'puts'} onclick={() => (side = 'puts')}>Puts</button>
          <button class="mseg-opt" class:on={side === 'calls'} type="button" aria-pressed={side === 'calls'} onclick={() => (side = 'calls')}>Calls</button>
        </div>
      </div>
      <div class="oc-expiries" role="group" aria-label="Expiry">
        {#each expiries as e (e.day)}
          <button class="oc-pill" class:on={e === expiry} type="button" aria-pressed={e === expiry} onclick={() => (picked = e.day)}>{e.label}</button>
        {/each}
      </div>
      <div>
        <div class="oc-head"><span>Strike</span><span class="r">Cost</span><span class="r">Premium</span><span class="r">Yield</span></div>
        <div class="scroll" style="max-height:436px" bind:this={list}>
          {#each rows as r (r.occ)}
            <button class="oc-row" type="button" onclick={() => pick(r)}><span>{strike(r.strike)}</span><span class="r">{money0(r.cost)}</span><span class="r prem">{money0(r.premium)}</span><span class="r">{pctText(r.yield)}</span></button>
          {/each}
        </div>
      </div>
    {:else}
      <div class="oc-title">
        <h2 id="oc-h" class="oc-name">{open.title}</h2>
        <button class="btn" type="button" onclick={back}>Back</button>
      </div>
      <div class="oc-figs">
        <div><span class="dim">Cost</span><span class="oc-big">{money0(open.cost)}</span></div>
        <div><span class="dim">Premium</span><span class="oc-big prem">{money0(open.premium)}</span></div>
      </div>
      {#if chart}
        <svg viewBox="0 0 660 164" role="img" aria-label={'Premium each day from ' + chart.from + ' to today'} class="oc-chart">
          <g stroke="var(--grid)" stroke-width="1">
            <line x1="0" y1="10" x2="600" y2="10"></line>
            <line x1="0" y1="75" x2="600" y2="75"></line>
            <line x1="0" y1="140" x2="600" y2="140"></line>
          </g>
          <polyline fill="none" stroke="var(--accent)" stroke-width="1.8" points={chart.points}></polyline>
          <circle cx="600" cy={chart.lastY} r="3.5" fill="var(--ink)"></circle>
          <g fill="var(--ink60)" font-size="10.5" font-family="var(--font)">
            <text x="608" y="13.5">{chart.top}</text>
            <text x="608" y="78.5">{chart.middle}</text>
            <text x="608" y="143.5">{chart.bottom}</text>
            <text x="0" y="158">{chart.from}</text>
            <text x="600" y="158" text-anchor="end">Today</text>
          </g>
        </svg>
      {/if}
    {/if}
  </section>
{/if}

<style>
  .oc{padding:16px 18px;display:flex;flex-direction:column;gap:16px}
  .oc-top{display:flex;flex-wrap:wrap;align-items:center;gap:12px}
  .oc-h{margin:0;font-weight:500;font-size:10.5px;line-height:18px}
  .oc-sides .mseg-opt{padding:6px 16px;font-size:13px}
  .oc-expiries{display:flex;flex-wrap:wrap;gap:6px}
  .oc-pill{border:0;cursor:pointer;font:500 12.5px var(--font);padding:7px 11px;border-radius:6px;background:rgba(var(--ink-rgb),.06);color:var(--ink75);white-space:nowrap}
  .oc-pill:hover{color:var(--ink)}
  .oc-pill.on{background:var(--chip-bg);color:var(--chip-fg)}
  .oc-head{display:grid;grid-template-columns:repeat(4,minmax(0,1fr));padding:0 12px 8px;font:500 10.5px/18px var(--font);letter-spacing:.09em;text-transform:uppercase;color:var(--ink60)}
  .oc-row{display:grid;grid-template-columns:repeat(4,minmax(0,1fr));width:100%;border:0;border-top:1px solid rgba(var(--ink-rgb),.06);background:transparent;color:var(--ink);font:400 14px var(--font);padding:11px 12px;cursor:pointer;text-align:left;font-variant-numeric:tabular-nums}
  .oc-row:hover{background:rgba(var(--ink-rgb),.08)}
  .r{text-align:right}
  .prem{color:var(--pos);font-weight:500}
  .oc-title{display:flex;align-items:center;justify-content:space-between;gap:12px}
  .oc-name{margin:0;font-size:16px;font-weight:500}
  .oc-figs{display:grid;grid-template-columns:repeat(2,minmax(0,1fr));gap:16px;font-variant-numeric:tabular-nums}
  .oc-figs > div{display:flex;flex-direction:column;gap:2px}
  .oc-figs .dim{font-size:12px}
  .oc-big{font-size:24px;font-weight:500}
  .oc-chart{width:100%;height:190px;display:block}
</style>
