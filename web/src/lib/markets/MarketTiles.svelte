<script lang="ts">
  import type { MarketTile } from '../model'

  let { tiles }: { tiles: MarketTile[] } = $props()

  let expanded = $state(false)
  const shown = $derived(expanded ? tiles : tiles.slice(0, 6))

  const fmtLast = (t: MarketTile) => (t.last == null ? '—' : t.last.toLocaleString('en-CA', { minimumFractionDigits: t.decimals, maximumFractionDigits: t.decimals }))
  const fmtChange = (t: MarketTile) => {
    if (t.change == null || t.percentChange == null) return ''
    const pts = (t.change >= 0 ? '+' : '') + t.change.toLocaleString('en-CA', { minimumFractionDigits: t.decimals, maximumFractionDigits: t.decimals })
    const pc = (t.percentChange >= 0 ? '+' : '') + t.percentChange.toFixed(2) + '%'
    return `${pts} (${pc})`
  }
</script>

<div class="wrap">
  <div class="tiles">
    {#each shown as t (t.symbol)}
      <div class="tile">
        <div class="lbl">{t.label}</div>
        <div class="last">{fmtLast(t)}</div>
        <div class="chg {t.change == null ? '' : t.change >= 0 ? 'pos' : 'neg'}">{fmtChange(t)}</div>
      </div>
    {/each}
  </div>
  {#if tiles.length > 6}
    <button class="more" onclick={() => (expanded = !expanded)}>{expanded ? 'Show fewer' : `Show ${tiles.length - 6} more`}</button>
  {/if}
</div>

<style>
  .wrap { display: flex; flex-direction: column; gap: 8px; }
  .tiles { display: grid; grid-template-columns: repeat(6, 1fr); gap: 12px; }
  .tile { background: #141924; border: 1px solid #1c2230; border-radius: 12px; padding: 10px 12px; }
  .lbl { color: #8b93a7; font-size: 11px; font-weight: 500; }
  .last { font-size: 17px; font-weight: 600; margin-top: 4px; font-variant-numeric: tabular-nums; }
  .chg { font-size: 11px; margin-top: 2px; font-variant-numeric: tabular-nums; color: #8b93a7; }
  .more { align-self: center; background: none; border: 0; color: #8b93a7; font: inherit; font-size: 12px; cursor: pointer; }
  .more:hover { color: #e6e9ef; }
  .pos { color: #3ecf8e; } .neg { color: #f0616d; }
  @media (max-width: 1100px) { .tiles { grid-template-columns: repeat(3, 1fr); } }
</style>
