<script lang="ts">
  // ledger's gridTh: the header row for a CSS-grid table (.nw-head / .wl-head /
  // .si-head). Each column is a .gth cell; clicking it sorts, except a `plain`
  // column which is a label only. The arrow shows the active column's direction.
  import { sort, toggleSort } from '../sort.svelte'

  export interface Col {
    key: string
    label: string
    align?: 'left' | 'right' | 'center'
    plain?: boolean
  }
  let { table, cols }: { table: string; cols: readonly Col[] } = $props()
</script>

{#each cols as c (c.key)}
  {@const on = sort[table]?.key === c.key}
  {@const right = c.align === 'right'}
  {#if c.plain}
    <div class="gth" style="text-align:{c.align || 'left'}"><span class="th-in">{c.label}</span></div>
  {:else}
    <div class="gth" class:on style="text-align:{c.align || 'left'}" onclick={() => toggleSort(table, c.key)}>
      <span class="th-in" style="flex-direction:{right ? 'row-reverse' : 'row'}">{c.label}<span class="arrow" style="color:{on ? 'var(--accent)' : 'transparent'}">{on && sort[table].dir === 'asc' ? '▲' : '▼'}</span></span>
    </div>
  {/if}
{/each}
