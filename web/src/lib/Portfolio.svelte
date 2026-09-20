<script lang="ts">
  import type { Model } from './model'
  import { cad } from './fmt'
  import PortfolioTiles from './portfolio/PortfolioTiles.svelte'
  import PortfolioHoldings from './portfolio/PortfolioHoldings.svelte'
  import Donut from './Donut.svelte'

  let { model }: { model: Model } = $props()
  const pf = $derived(model.portfolio)

  const allocItems = $derived(pf.allocation.map((a) => ({ label: a.symbol, value: a.value })))
  const sectorItems = $derived(pf.sectors.map((s) => ({ label: s.name, value: s.value })))
  const regionItems = $derived(pf.regions.map((r) => ({ label: r.name, value: r.value })))
  const spanCount = (arr: { name: string; value: number }[]) =>
    String(arr.filter((x) => x.value > 0 && x.name !== 'Not classified').length)
</script>

<div class="pf">
  <PortfolioTiles {pf} />

  <div class="split">
    <div class="card donutcard">
      <h5>Allocation</h5>
      <Donut items={allocItems} cap={10} centerLabel="Market value" centerTotal={cad(pf.marketValue)} showValue valueFmt={(n) => cad(n)} />
    </div>
    <PortfolioHoldings positions={model.positions} />
  </div>

  <div class="card sr">
    <div class="sr-head"><h5>Sectors</h5><h5>Regions</h5></div>
    <div class="sr-body">
      <Donut items={sectorItems} centerLabel="Sectors" centerTotal={spanCount(pf.sectors)} />
      <Donut items={regionItems} cap={10} centerLabel="Regions" centerTotal={spanCount(pf.regions)} />
    </div>
  </div>
</div>

<style>
  .pf { display: flex; flex-direction: column; gap: 16px; }
  .split { display: grid; grid-template-columns: 5fr 7fr; gap: 16px; align-items: stretch; }
  .card { background: #141924; border: 1px solid #1c2230; border-radius: 12px; padding: 16px 18px; }
  .donutcard { display: flex; flex-direction: column; }
  .donutcard h5, .sr-head h5 { margin: 0 0 10px; font-size: 14px; font-weight: 600; }
  .sr-head { display: flex; justify-content: space-between; }
  .sr-body { display: grid; grid-template-columns: 1fr 1fr; gap: 24px; }
  @media (max-width: 900px) { .split, .sr-body { grid-template-columns: 1fr; } }
</style>
