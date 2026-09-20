<script lang="ts">
  import type { Markets } from './model'
  import MarketTiles from './markets/MarketTiles.svelte'
  import Heatmap from './heatmap/Heatmap.svelte'

  let { markets }: { markets: Markets } = $props()
</script>

<div class="mk">
  <MarketTiles tiles={markets.tiles} />
  <Heatmap {markets} />
</div>

<style>
  .mk { display: flex; flex-direction: column; gap: 16px; }
</style>
