<script lang="ts">
  // The Markets tab (marketsHtml): the tile row, Fear & Greed, the heatmap, then a
  // row of News and Watchlist, then ranked short interest. The listing-detail
  // branch of marketsHtml is handled elsewhere (TradeDetail), not here. The tile
  // row and the watchlist are the `markets` document; the heatmap and the news
  // subscribe their own.
  import type { MarketsDoc } from './model'
  import MarketTiles from './markets/MarketTiles.svelte'
  import FearGreed from './markets/FearGreed.svelte'
  import Heatmap from './heatmap/Heatmap.svelte'
  import News from './markets/News.svelte'
  import Watchlist from './markets/Watchlist.svelte'
  import Shorts from './markets/Shorts.svelte'

  let { markets }: { markets: MarketsDoc } = $props()
</script>

<div style="padding:20px;display:flex;flex-direction:column;gap:14px">
  <MarketTiles tiles={markets.tiles} directory={markets.directory} />
  <FearGreed />
  <Heatmap />
  <div style="display:grid;grid-template-columns:minmax(0,1fr) 420px;gap:14px;align-items:stretch">
    <News />
    <Watchlist watchlist={markets.watchlist} />
  </div>
  <Shorts />
</div>
