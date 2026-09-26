<script lang="ts">
  // ledger's msegHtml: a segmented control. Options are [value, label] pairs, or
  // the string '|' for a divider. The active value gets .on.
  type Opt = readonly [string, string] | '|'
  let { options, cur, onpick }: { options: readonly Opt[]; cur: string; onpick: (v: string) => void } = $props()
</script>

<span class="mseg">
  {#each options as o, i (i)}
    {#if o === '|'}
      <span class="mseg-div"></span>
    {:else}
      <button class="mseg-opt" class:on={o[0] === cur} onclick={() => onpick(o[0])}>{o[1]}</button>
    {/if}
  {/each}
</span>
