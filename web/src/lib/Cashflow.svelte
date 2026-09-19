<script lang="ts">
  import type { Cashflow } from './model'
  import { cad } from './fmt'
  import CashflowTiles from './cashflow/CashflowTiles.svelte'
  import CashflowChart from './cashflow/CashflowChart.svelte'
  import CashflowPositions from './cashflow/CashflowPositions.svelte'
  import Donut from './Donut.svelte'
  import DistributionHistory from './cashflow/DistributionHistory.svelte'

  let { cashflow }: { cashflow: Cashflow } = $props()

  // Allocation is by projected monthly income (yob) per SPEC.
  const allocItems = $derived(cashflow.holdings.map((h) => ({ label: h.symbol, value: h.yob })))
  const allocTotal = $derived(allocItems.reduce((s, i) => s + i.value, 0))
</script>

<div class="cf">
  <CashflowTiles tiles={cashflow.tiles} />
  <CashflowChart months={cashflow.months} other={cashflow.other} />
  <CashflowPositions holdings={cashflow.holdings} />
  <div class="split">
    <div class="card donutcard">
      <h5>Allocation</h5>
      <Donut items={allocItems} centerLabel="Projected" centerTotal={cad(allocTotal)} showValue valueFmt={(n) => cad(n)} />
    </div>
    <DistributionHistory rows={cashflow.rows} />
  </div>
</div>

<style>
  .cf { display: flex; flex-direction: column; gap: 16px; }
  .split { display: grid; grid-template-columns: 1fr 1fr; gap: 16px; }
  .card { background: #141924; border: 1px solid #1c2230; border-radius: 12px; padding: 16px 18px; }
  .donutcard { height: 380px; display: flex; flex-direction: column; }
  .donutcard h5 { margin: 0 0 10px; font-size: 14px; font-weight: 600; }
  @media (max-width: 900px) { .split { grid-template-columns: 1fr; } }
</style>
