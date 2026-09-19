<script lang="ts">
  import type { Cashflow } from './model'
  import CashflowTiles from './cashflow/CashflowTiles.svelte'
  import CashflowChart from './cashflow/CashflowChart.svelte'
  import CashflowPositions from './cashflow/CashflowPositions.svelte'
  import AllocationDonut from './cashflow/AllocationDonut.svelte'
  import DistributionHistory from './cashflow/DistributionHistory.svelte'

  let { cashflow }: { cashflow: Cashflow } = $props()

  // Allocation is by projected monthly income (yob) per SPEC.
  const allocItems = $derived(cashflow.holdings.map((h) => ({ symbol: h.symbol, value: h.yob })))
</script>

<div class="cf">
  <CashflowTiles tiles={cashflow.tiles} />
  <CashflowChart months={cashflow.months} other={cashflow.other} />
  <CashflowPositions holdings={cashflow.holdings} />
  <div class="split">
    <AllocationDonut items={allocItems} />
    <DistributionHistory rows={cashflow.rows} />
  </div>
</div>

<style>
  .cf { display: flex; flex-direction: column; gap: 16px; }
  .split { display: grid; grid-template-columns: 1fr 1fr; gap: 16px; }
  @media (max-width: 900px) { .split { grid-template-columns: 1fr; } }
</style>
