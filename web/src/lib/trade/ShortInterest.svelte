<script lang="ts">
  import type { Trade, Shorts } from '../model'
  import { equityChart } from '../actions/equityChart'

  let { trade }: { trade: Trade } = $props()

  const REGULATOR: Record<string, string> = { us: 'FINRA', ca: 'CIRO' }
  // Short selling is reported only for a share listing; nothing for a coin, index,
  // contract or option.
  // svelte-ignore state_referenced_locally
  const HAS = trade.kind === 'Shares' || trade.kind === 'ETF'

  let data = $state<Shorts | null>(null)
  let loading = $state(true)

  async function load() {
    if (!HAS) { loading = false; return }
    try {
      const q = new URLSearchParams({ symbol: trade.symbol, exchange: trade.exchange || '', currency: trade.currency || '', kind: trade.kind || '' })
      const r = await fetch('/api/shorts?' + q.toString())
      const d = await r.json()
      if (d.ok && d.covered) data = d.shorts
    } catch {
      /* leave empty */
    }
    loading = false
  }
  $effect(() => { load() })

  const R = 42
  const C = 2 * Math.PI * R
  const compact = (n: number | null) => {
    if (n == null) return '—'
    const a = Math.abs(n)
    if (a >= 1e9) return (n / 1e9).toFixed(2) + 'B'
    if (a >= 1e6) return (n / 1e6).toFixed(1) + 'M'
    if (a >= 1e3) return (n / 1e3).toFixed(0) + 'K'
    return String(Math.round(n))
  }
  const signed = (n: number | null) => (n == null ? '—' : (n >= 0 ? '+' : '') + compact(n))
  const seriesPoints = $derived(data ? data.series.map((p) => ({ d: p.date, v: p.shares, dep: 0 })) : [])
</script>

{#snippet donut(frac: number, pct: string, legendA: string, valA: string, legendB: string, valB: string, foot: string, dateRight: string)}
  <div class="tile">
    <div class="thead">
      <div class="tlegend">
        <span><i class="sw a"></i>{legendA} {valA}</span>
        <span><i class="sw b"></i>{legendB} {valB}</span>
      </div>
      <span class="tdate">{dateRight}</span>
    </div>
    <svg viewBox="0 0 120 120" class="d">
      <g transform="rotate(-90 60 60)">
        <circle cx="60" cy="60" r={R} fill="none" stroke="#232a38" stroke-width="14" />
        <circle cx="60" cy="60" r={R} fill="none" stroke="#4b9fff" stroke-width="14" stroke-dasharray="{frac * C} {C}" />
      </g>
      <text x="60" y="57" text-anchor="middle" class="dpct">{pct}</text>
      <text x="60" y="70" text-anchor="middle" class="dlbl">Short</text>
    </svg>
    <div class="tfoot">{foot}</div>
  </div>
{/snippet}

{#if HAS}
  <div class="card">
    <div class="head"><h5>Short interest</h5>{#if data}<span class="reg">{REGULATOR[data.market] ?? ''}</span>{/if}</div>

    {#if loading}
      <p class="msg">Reading…</p>
    {:else if !data}
      <p class="msg">No short interest reported for this listing.</p>
    {:else}
      <div class="tiles">
        {@render donut(
          (data.volumePct ?? 0) / 100,
          (data.volumePct ?? 0).toFixed(1) + '%',
          'Short volume', compact(data.shortVolume),
          'Shares traded', compact(data.totalVolume),
          `${data.daysToCover ?? '—'} days to cover · ${compact(data.averageVolume)} daily volume`,
          `${data.volumeOf} · ${data.volumeSpan}`,
        )}
        {@render donut(
          (data.ofFloat ?? 0) / 100,
          (data.ofFloat ?? 0).toFixed(2) + '%',
          'Short interest', compact(data.shares),
          'Float', compact(data.float),
          `${compact(data.shares)} shares short · ${signed(data.change)} since last`,
          data.asOf,
        )}
      </div>

      {#if seriesPoints.length >= 3}
        <div class="overtime">
          <div class="ohead">Short interest over time</div>
          <div class="ochart" use:equityChart={seriesPoints}></div>
        </div>
      {/if}
    {/if}
  </div>
{/if}

<style>
  .card { background: #141924; border: 1px solid #1c2230; border-radius: 12px; padding: 16px 18px; }
  .head { display: flex; align-items: baseline; justify-content: space-between; margin-bottom: 12px; }
  .head h5 { margin: 0; font-size: 14px; font-weight: 600; }
  .reg { color: #8b93a7; font-size: 11px; }
  .msg { color: #8b93a7; font-size: 13px; }
  .tiles { display: grid; grid-template-columns: 1fr 1fr; gap: 16px; }
  .tile { background: #0f131c; border: 1px solid #1c2230; border-radius: 10px; padding: 12px 14px; display: flex; flex-direction: column; align-items: center; gap: 6px; }
  .thead { width: 100%; display: flex; justify-content: space-between; align-items: flex-start; gap: 8px; }
  .tlegend { display: flex; flex-direction: column; gap: 2px; font-size: 11px; color: #8b93a7; }
  .tlegend .sw { width: 9px; height: 9px; border-radius: 2px; display: inline-block; margin-right: 5px; }
  .sw.a { background: #4b9fff; } .sw.b { background: #232a38; }
  .tdate { color: #5b6474; font-size: 10px; white-space: nowrap; }
  .d { width: 130px; height: 130px; }
  .dpct { fill: #e6e9ef; font-size: 15px; font-weight: 600; }
  .dlbl { fill: #8b93a7; font-size: 9px; }
  .tfoot { color: #8b93a7; font-size: 11px; text-align: center; font-variant-numeric: tabular-nums; }
  .overtime { margin-top: 16px; }
  .ohead { color: #8b93a7; font-size: 12px; margin-bottom: 6px; }
  .ochart { width: 100%; height: 200px; }
  @media (max-width: 700px) { .tiles { grid-template-columns: 1fr; } }
</style>
