<script lang="ts">
  import type { Trade } from './model'
  import { tradeChart, type Bar, type Fill } from './actions/tradeChart'
  import { price, money, pct, num } from './fmt'
  import { go } from './router.svelte'
  import TradeJournal from './trade/TradeJournal.svelte'
  import Disclosures from './trade/Disclosures.svelte'

  let { trade }: { trade: Trade } = $props()

  interface DetailFill { id: string; date: string; time: string; side: string; sub: string; qty: number; price: number | null; amount: number; currency: string }

  const TIMEFRAMES: [string, string][] = [['1h', '1H'], ['4h', '4H'], ['1d', '1D'], ['1w', '1W'], ['1M', '1M']]
  function defaultTf(days: number): string {
    if (days <= 2) return '1h'
    if (days <= 10) return '4h'
    if (days <= 180) return '1d'
    if (days <= 1095) return '1w'
    return '1M'
  }

  let fills = $state<DetailFill[]>([])
  let bars = $state<Bar[]>([])
  let available = $state<string[]>([])
  // Default timeframe from the trade's length, computed once at mount (the
  // component is keyed per trade id, so it remounts for a different trade).
  // svelte-ignore state_referenced_locally
  let tf = $state(defaultTf(trade.holdDays))
  let reason = $state('')
  let loadingBars = $state(true)

  const day = 86400000
  function span() {
    const from = new Date(Date.parse(trade.entryDate) - 10 * day).toISOString().slice(0, 10)
    const end = trade.exitDate ? Math.min(Date.now(), Date.parse(trade.exitDate) + 10 * day) : Date.now()
    return { from, to: new Date(end).toISOString().slice(0, 10) }
  }

  async function loadFills() {
    try {
      const r = await fetch('/api/trade?id=' + encodeURIComponent(trade.id))
      const d = await r.json()
      if (d.ok) fills = d.fills ?? []
    } catch {
      /* leave empty */
    }
  }
  async function loadBars(which: string) {
    loadingBars = true
    const sp = span()
    const q = new URLSearchParams({ symbol: trade.symbol, exchange: trade.exchange || '', currency: trade.currency || '', kind: trade.kind || '', from: sp.from, to: sp.to, tf: which })
    try {
      const r = await fetch('/api/history?' + q.toString())
      const d = await r.json()
      if (d.ok) {
        bars = d.bars ?? []
        available = d.available ?? []
        reason = d.reason ?? ''
      } else {
        bars = []
        reason = d.error ?? ''
      }
    } catch {
      bars = []
      reason = 'Could not reach the history source.'
    }
    loadingBars = false
  }

  $effect(() => {
    loadFills()
  })
  $effect(() => {
    loadBars(tf)
  })

  const chartFills = $derived(fills.map((f) => ({ date: f.date, qty: f.qty, price: f.price })) as Fill[])
  const listingTicker = $derived((trade.symbol || '').replace(/\.(TO|V|CN|NE)$/i, ''))
  const sideText = (f: DetailFill) => (f.sub && f.sub !== f.side ? f.sub.replace(/([A-Z])(TO)([A-Z])/, '$1 $2 $3').replace(/_/g, ' ') : f.side)
</script>

<div class="detail">
  <button class="back" onclick={() => go('trades')}>← Trades</button>

  <div class="header">
    <div>
      <div class="sym">{trade.symbol}</div>
      <div class="name">{trade.name} · {trade.exchange}: {listingTicker}</div>
    </div>
    <div class="pnl {trade.pnl >= 0 ? 'pos' : 'neg'}">
      {money(trade.pnl, trade.currency)} ({pct(trade.pnlPct)})
    </div>
  </div>

  <div class="card chartcard">
    <div class="tfrow">
      {#each TIMEFRAMES as [key, label] (key)}
        {#if available.includes(key) || key === tf}
          <button class="pill" class:on={tf === key} onclick={() => (tf = key)}>{label}</button>
        {/if}
      {/each}
    </div>
    {#if bars.length}
      {#key tf}
        <div class="chart" use:tradeChart={{ bars, fills: chartFills }}></div>
      {/key}
    {:else}
      <div class="empty">{loadingBars ? 'Loading bars…' : reason || 'No bars for this span.'}</div>
    {/if}
  </div>

  <div class="facts">
    <div><span>Open</span>{trade.entryDate}</div>
    <div><span>Close</span>{trade.exitDate || '—'}</div>
    <div><span>Entry</span>{price(trade.entry)}</div>
    <div><span>Exit</span>{price(trade.exit)}</div>
    <div><span>Hold</span>{trade.holdDays}d</div>
    <div><span>Account</span>{trade.account ?? '—'}</div>
  </div>

  <div class="card">
    <h5>Executions</h5>
    <div class="scroll">
      <table>
        <thead><tr><th class="l">When</th><th class="l">Side</th><th>Qty</th><th class="c">FX</th><th>Price</th><th>Amount</th></tr></thead>
        <tbody>
          {#each fills as f (f.id)}
            <tr>
              <td class="l dim">{f.date} {f.time}</td>
              <td class="l">{sideText(f)}</td>
              <td class="r">{num(f.qty, 0)}</td>
              <td class="c dim">{f.currency}</td>
              <td class="r">{f.price != null ? price(f.price) : '—'}</td>
              <td class="r">{money(f.amount, f.currency)}</td>
            </tr>
          {/each}
        </tbody>
      </table>
    </div>
  </div>

  <div class="two">
    <TradeJournal {trade} />
    <Disclosures {trade} />
  </div>
</div>

<style>
  .detail { display: flex; flex-direction: column; gap: 16px; }
  .two { display: grid; grid-template-columns: 1fr 1fr; gap: 16px; align-items: start; }
  @media (max-width: 900px) { .two { grid-template-columns: 1fr; } }
  .back { align-self: flex-start; background: none; border: 0; color: #8b93a7; font: inherit; font-size: 13px; cursor: pointer; padding: 0; }
  .back:hover { color: #e6e9ef; }
  .header { display: flex; align-items: flex-start; justify-content: space-between; gap: 16px; }
  .sym { font-size: 22px; font-weight: 700; }
  .name { color: #8b93a7; font-size: 13px; margin-top: 2px; }
  .pnl { font-size: 20px; font-weight: 600; font-variant-numeric: tabular-nums; }
  .card { background: #141924; border: 1px solid #1c2230; border-radius: 12px; padding: 16px 18px; }
  .card h5 { margin: 0 0 10px; font-size: 14px; font-weight: 600; }
  .chartcard { display: flex; flex-direction: column; gap: 10px; }
  .tfrow { display: flex; gap: 6px; }
  .pill { background: #1c2230; border: 0; color: #8b93a7; border-radius: 6px; padding: 3px 10px; font-size: 11px; cursor: pointer; }
  .pill.on { background: #2a3242; color: #e6e9ef; }
  .chart { width: 100%; height: 340px; }
  .empty { height: 340px; display: flex; align-items: center; justify-content: center; color: #8b93a7; font-size: 13px; }
  .facts { display: grid; grid-template-columns: repeat(6, 1fr); gap: 12px; background: #141924; border: 1px solid #1c2230; border-radius: 12px; padding: 14px 18px; }
  .facts div { display: flex; flex-direction: column; gap: 3px; font-size: 13px; font-variant-numeric: tabular-nums; }
  .facts span { color: #8b93a7; font-size: 11px; text-transform: uppercase; letter-spacing: 0.03em; }
  .scroll { overflow: auto; max-height: 320px; }
  table { width: 100%; border-collapse: collapse; font-size: 12px; font-variant-numeric: tabular-nums; }
  th { position: sticky; top: 0; background: #141924; color: #8b93a7; font-weight: 500; text-align: right; padding: 6px 10px; border-bottom: 1px solid #1c2230; }
  td { text-align: right; padding: 6px 10px; white-space: nowrap; border-bottom: 1px solid #12161f; }
  th.l, td.l { text-align: left; } th.c, td.c { text-align: center; }
  .dim { color: #8b93a7; }
  .r { text-align: right; }
  .pos { color: #3ecf8e; } .neg { color: #f0616d; }
</style>
