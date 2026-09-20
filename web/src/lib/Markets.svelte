<script lang="ts">
  import type { Markets } from './model'
  import MarketTiles from './markets/MarketTiles.svelte'
  import FearGreed from './markets/FearGreed.svelte'
  import Heatmap from './heatmap/Heatmap.svelte'
  import News from './markets/News.svelte'
  import Watchlist from './markets/Watchlist.svelte'

  let { markets }: { markets: Markets } = $props()
</script>

<div class="mk">
  <MarketTiles tiles={markets.tiles} />
  <FearGreed />
  <Heatmap {markets} />
  <div class="split">
    <News news={markets.news} />
    <Watchlist watchlist={markets.watchlist} />
  </div>
</div>

<style>
  .mk { display: flex; flex-direction: column; gap: 16px; }
  .split { display: grid; grid-template-columns: 2fr 1fr; gap: 16px; align-items: start; }
  @media (max-width: 900px) { .split { grid-template-columns: 1fr; } }
</style>
