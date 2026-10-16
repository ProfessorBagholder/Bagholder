<script lang="ts">
  // The listings a symbol given with no venue may be (`/api/listings`), one row each
  // with its venue, for the person to pick: two listings of one ticker are two
  // companies or two markets, and none is chosen for them (plan
  // `market-data-keyed-by-listing`, Approach, 5).
  import type { ResolvedListing } from '../generated/markets'
  import { symText } from '../sym'

  let { listings, onpick }: { listings: ResolvedListing[]; onpick: (l: ResolvedListing) => void } = $props()
</script>

<div class="scroll" style="max-height:240px">
  {#each listings as l (l.listing)}
    <div class="lc-row go" role="button" tabindex="-1" onclick={() => onpick(l)} onkeydown={(e) => { if (e.key === 'Enter') onpick(l) }}>
      <div style="min-width:0"><div style="font-size:13px;font-weight:500">{symText(l.symbol)}</div><div style="font-size:11px;color:var(--ink55);overflow:hidden;text-overflow:ellipsis;white-space:nowrap">{l.name}</div></div>
      <div style="font-size:12px;color:var(--ink55);text-align:right">{l.exchange}</div>
    </div>
  {/each}
</div>

<style>
  .lc-row {
    display: grid;
    grid-template-columns: minmax(0, 1fr) auto;
    gap: 12px;
    align-items: center;
    padding: 7px 0;
    border-top: 1px solid rgba(var(--ink-rgb), 0.06);
  }
</style>
