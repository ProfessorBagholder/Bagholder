<script lang="ts">
  import type { Tab } from './router.svelte'

  // The page that is coming, in silhouette. Each tab has its own, so nothing
  // changes shape when the data lands: the page resolves rather than reflowing.
  // Bar widths and stagger vary on purpose; a field of identical bars reads as a
  // loading graphic rather than as content.
  let { tab }: { tab: Tab } = $props()

  const TILE = ['58%', '40%', '52%', '46%', '62%', '44%']
  const COLS = ['18%', '11%', '9%', '12%', '10%', '13%', '9%']
  const LEGEND = ['70%', '55%', '62%', '48%', '58%']
  const SPLIT = 'display:grid;grid-template-columns:calc((100% - 70px) / 6 * 4 + 42px) minmax(0,1fr);gap:14px'
  const range = (n: number) => Array.from({ length: n }, (_, i) => i)
</script>

{#snippet bar(w: string, h: number, delay = 0, extra = '')}
  <div class="bhsk" style="width:{w};height:{h}px;animation-delay:{delay}ms{extra}"></div>
{/snippet}

{#snippet tiles(height: number)}
  <div style="display:grid;grid-template-columns:repeat(6,minmax(0,1fr));gap:14px">
    {#each range(6) as i (i)}
      <div class="card elev-sm" style="padding:14px 16px;height:{height}px;display:flex;flex-direction:column;gap:10px">
        {@render bar('46%', 8, i * 70)}{@render bar(TILE[i], 15, i * 70 + 40)}
      </div>
    {/each}
  </div>
{/snippet}

{#snippet rows(n: number)}
  {#each range(n) as i (i)}
    <div style="display:flex;align-items:center;gap:12px;padding:7px 0">
      {@render bar('22%', 10, i * 90)}{@render bar('14%', 10, i * 90 + 30, ';margin-left:auto')}{@render bar('12%', 10, i * 90 + 60)}
    </div>
  {/each}
{/snippet}

{#snippet listCard(title: string, n: number)}
  <div class="card elev-sm sk-card" style="height:302px">{@render bar(title, 13)}{@render rows(n)}</div>
{/snippet}

{#snippet chartCard(delay: number)}
  <div class="card elev-sm sk-card" style="height:302px">
    {@render bar('90px', 13)}
    <div class="bhsk" style="width:100%;flex:1;border-radius:6px;animation-delay:{delay}ms"></div>
  </div>
{/snippet}

<div id="pageSkel" aria-hidden="true" style="padding:20px;display:flex;flex-direction:column;gap:14px">
  {#if tab === 'trades'}
    <div style="display:flex;align-items:center;gap:14px">
      {@render bar('120px', 15)}{@render bar('60px', 11)}{@render bar('180px', 24, 40, ';margin-left:auto;border-radius:6px')}
    </div>
    <div class="card elev-sm" style="padding:14px 18px;display:flex;flex-direction:column;gap:6px">
      <div style="display:flex;gap:14px;padding-bottom:8px">
        {#each COLS as w, i (i)}{@render bar(w, 8, i * 24)}{/each}
      </div>
      {#each range(14) as r (r)}
        <div style="display:flex;align-items:center;gap:14px;padding:8px 0">
          {#each COLS as w, c (c)}{@render bar(w, 10, ((r + c) % 8) * 24)}{/each}
        </div>
      {/each}
    </div>
  {:else if tab === 'markets'}
    {@render tiles(76)}
    <div class="card elev-sm sk-card" style="height:470px">
      <div style="display:flex;gap:14px;align-items:center">
        {@render bar('90px', 13)}{@render bar('220px', 22, 40, ';margin-left:0;border-radius:6px')}
      </div>
      <div class="bhsk" style="width:100%;height:430px;border-radius:5px;animation-delay:120ms"></div>
    </div>
    <div style={SPLIT}>{@render listCard('90px', 7)}{@render listCard('80px', 6)}</div>
  {:else if tab === 'portfolio'}
    {@render tiles(97)}
    <div style={SPLIT}>
      {@render listCard('90px', 8)}
      <div class="card elev-sm sk-card" style="height:302px">
        <div style="display:flex;align-items:center;gap:16px;padding-top:10px">
          <div class="bhsk" style="width:188px;height:188px;border-radius:50%"></div>
          <div style="flex:1;display:flex;flex-direction:column;gap:9px">
            {#each LEGEND as w, i (i)}{@render bar(w, 10, i * 90)}{/each}
          </div>
        </div>
      </div>
    </div>
  {:else}
    <!-- dashboard and cashflow share a silhouette: tiles, then a chart card beside a list card -->
    {@render tiles(97)}
    <div style={SPLIT}>{@render chartCard(110)}{@render listCard('80px', 7)}</div>
    <div style={SPLIT}>{@render chartCard(140)}{@render listCard('80px', 7)}</div>
  {/if}
</div>

<style>
  .sk-card {
    padding: 16px 18px;
    display: flex;
    flex-direction: column;
    gap: 10px;
    overflow: hidden;
  }
</style>
