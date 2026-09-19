<script lang="ts">
  import type { CashflowMonth, CashflowRow } from '../model'
  import { cad } from '../fmt'

  let { months, other }: { months: CashflowMonth[]; other: CashflowRow[] } = $props()

  // Margin interest per month: group the Interest charge rows by YYYY-MM. This is
  // presentation grouping of amounts the server already computed, not money math.
  const interestByMonth = $derived.by(() => {
    const m: Record<string, number> = {}
    for (const r of other) {
      if (r.kind !== 'Interest charge') continue
      const key = r.date.slice(0, 7)
      m[key] = (m[key] ?? 0) + Math.abs(r.amountCad)
    }
    return m
  })

  const bars = $derived(
    months.map((mo) => ({ ...mo, dist: mo.value, interest: interestByMonth[mo.key] ?? 0 })),
  )
  const maxVal = $derived(Math.max(1, ...bars.map((b) => Math.max(b.dist, b.interest))))
  // six evenly spaced axis labels
  const axisIdx = $derived.by(() => {
    const n = bars.length
    if (n <= 6) return bars.map((_, i) => i)
    return [0, 1, 2, 3, 4, 5].map((k) => Math.round((k * (n - 1)) / 5))
  })

  let hover = $state<number | null>(null)
</script>

<div class="card">
  <div class="head">
    <h5>Cashflow</h5>
    <div class="legend">
      <span class="key"><i class="sw dist"></i>Distributions</span>
      <span class="key"><i class="sw int"></i>Margin interest</span>
    </div>
  </div>

  <div class="plot">
    <div class="bars">
      {#each bars as b, i (b.key)}
        <div
          class="col"
          role="presentation"
          onmouseenter={() => (hover = i)}
          onmouseleave={() => (hover = null)}
        >
          <div class="track">
            <div class="bar dist" style="height: {(b.dist / maxVal) * 100}%"></div>
            <div class="bar int" style="height: {(b.interest / maxVal) * 100}%"></div>
          </div>
        </div>
      {/each}
    </div>
    <div class="axis">
      {#each axisIdx as i}
        <span style="left: {((i + 0.5) / bars.length) * 100}%">{bars[i]?.label}</span>
      {/each}
    </div>

    {#if hover != null && bars[hover]}
      {@const b = bars[hover]}
      <div class="tip" style="left: {((hover + 0.5) / bars.length) * 100}%">
        <div class="tip-m">{b.label}</div>
        <div class="tip-r"><span>Distributions</span><span class="pos">{cad(b.dist, 2)}</span></div>
        <div class="tip-r"><span>Margin interest</span><span class="neg">-{cad(b.interest, 2)}</span></div>
        <div class="tip-r net"><span>Net cashflow</span><span class={b.dist - b.interest >= 0 ? 'pos' : 'neg'}>{cad(b.dist - b.interest, 2)}</span></div>
      </div>
    {/if}
  </div>
</div>

<style>
  .card { background: #141924; border: 1px solid #1c2230; border-radius: 12px; padding: 16px 18px; }
  .head { display: flex; align-items: baseline; justify-content: space-between; margin-bottom: 12px; }
  .head h5 { margin: 0; font-size: 14px; font-weight: 600; }
  .legend { display: flex; gap: 14px; }
  .key { color: #8b93a7; font-size: 12px; display: inline-flex; align-items: center; gap: 6px; }
  .sw { width: 10px; height: 10px; border-radius: 2px; display: inline-block; }
  .sw.dist { background: #3ecf8e; }
  .sw.int { background: #f0616d; }
  .plot { position: relative; }
  .bars { display: flex; align-items: flex-end; gap: 3px; height: 240px; }
  .col { flex: 1; height: 100%; display: flex; align-items: flex-end; min-width: 0; }
  .track { position: relative; width: 100%; height: 100%; display: flex; align-items: flex-end; }
  .bar { position: absolute; bottom: 0; width: 100%; border-radius: 2px 2px 0 0; }
  .bar.dist { background: #3ecf8e; }
  .bar.int { background: #f0616d; }
  .col:hover .track { filter: brightness(1.15); }
  .axis { position: relative; height: 18px; margin-top: 6px; }
  .axis span { position: absolute; transform: translateX(-50%); color: #8b93a7; font-size: 10px; white-space: nowrap; }
  .tip {
    position: absolute; bottom: 30px; transform: translateX(-50%); pointer-events: none;
    background: #0b0e14; border: 1px solid #2a3242; border-radius: 8px; padding: 8px 10px;
    font-size: 12px; min-width: 150px; z-index: 5; box-shadow: 0 6px 20px rgba(0,0,0,0.4);
  }
  .tip-m { font-weight: 600; margin-bottom: 4px; }
  .tip-r { display: flex; justify-content: space-between; gap: 12px; color: #8b93a7; }
  .tip-r.net { margin-top: 4px; padding-top: 4px; border-top: 1px solid #2a3242; color: #e6e9ef; }
  .pos { color: #3ecf8e; }
  .neg { color: #f0616d; }
</style>
