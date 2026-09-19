<script lang="ts">
  import type { Model } from './model'
  import { equityChart } from './actions/equityChart'

  let { model }: { model: Model } = $props()

  const cad = (n: number, dp = 0) =>
    (n < 0 ? '-' : '') + '$' + Math.abs(n).toLocaleString('en-CA', { minimumFractionDigits: dp, maximumFractionDigits: dp })
  const pct = (n: number) => (n * 100).toFixed(1) + '%'

  // Derived tiles — Svelte tracks the dependency on `model`; no manual keying.
  const tiles = $derived([
    { label: 'Realized P&L', value: cad(model.kpi.realized), tone: model.kpi.realized >= 0 ? 'pos' : 'neg' },
    { label: 'Win rate', value: pct(model.kpi.winRate), sub: `${model.kpi.wins}/${model.kpi.count}` },
    { label: 'Profit factor', value: model.kpi.profitFactorInfinite ? '∞' : model.kpi.profitFactor.toFixed(2) },
    { label: 'Expectancy', value: cad(model.kpi.expectancy), tone: model.kpi.expectancy >= 0 ? 'pos' : 'neg' },
    { label: 'Avg win', value: cad(model.kpi.avgWin), tone: 'pos' },
    { label: 'Avg loss', value: cad(model.kpi.avgLoss), tone: 'neg' },
    { label: 'Avg hold', value: Math.round(model.kpi.avgHold) + 'd' },
    { label: 'Trades', value: String(model.kpi.count), sub: `${model.kpi.openCount} open` },
  ])

  const maxAbs = $derived(Math.max(1, ...model.monthly.map((m) => Math.abs(m.value))))
</script>

<section class="dash">
  <div class="tiles">
    {#each tiles as t (t.label)}
      <div class="tile">
        <div class="tile-label">{t.label}</div>
        <div class="tile-value {t.tone ?? ''}">{t.value}</div>
        {#if t.sub}<div class="tile-sub">{t.sub}</div>{/if}
      </div>
    {/each}
  </div>

  <div class="card">
    <h5>{model.equity.label ?? 'Equity'}</h5>
    <div class="chart" use:equityChart={model.equity.series}></div>
  </div>

  <div class="card">
    <h5>Monthly P&amp;L</h5>
    <div class="bars">
      {#each model.monthly as m (m.key)}
        <div class="bar-col" title={m.label}>
          <div class="bar-track">
            <div
              class="bar {m.value >= 0 ? 'pos' : 'neg'}"
              style="height: {(Math.abs(m.value) / maxAbs) * 100}%"
            ></div>
          </div>
          <div class="bar-label">{m.label}</div>
        </div>
      {/each}
    </div>
  </div>
</section>

<style>
  .dash { display: flex; flex-direction: column; gap: 16px; }
  .tiles { display: grid; grid-template-columns: repeat(4, 1fr); gap: 12px; }
  .tile { background: #141924; border: 1px solid #1c2230; border-radius: 12px; padding: 14px 16px; }
  .tile-label { color: #8b93a7; font-size: 11px; text-transform: uppercase; letter-spacing: 0.04em; }
  .tile-value { font-size: 26px; font-weight: 600; margin-top: 6px; font-variant-numeric: tabular-nums; }
  .tile-sub { color: #8b93a7; font-size: 12px; margin-top: 2px; }
  .pos { color: #3ecf8e; }
  .neg { color: #f0616d; }
  .card { background: #141924; border: 1px solid #1c2230; border-radius: 12px; padding: 16px 18px; }
  .card h5 { margin: 0 0 10px; font-size: 14px; font-weight: 600; }
  .chart { width: 100%; height: 280px; }
  .bars { display: flex; align-items: flex-end; gap: 6px; height: 180px; overflow-x: auto; }
  .bar-col { display: flex; flex-direction: column; align-items: center; gap: 6px; min-width: 26px; flex: 1; }
  .bar-track { display: flex; align-items: flex-end; height: 150px; width: 100%; }
  .bar { width: 100%; border-radius: 3px 3px 0 0; min-height: 2px; }
  .bar.pos { background: #3ecf8e; }
  .bar.neg { background: #f0616d; }
  .bar-label { color: #8b93a7; font-size: 9px; white-space: nowrap; }
</style>
