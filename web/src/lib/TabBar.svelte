<script lang="ts">
  import { TABS, TAB_LABEL, route, go, type Tab } from './router.svelte'

  // The sliding underline is a plain reactive element, positioned from the active
  // tab's measured geometry. Under Svelte this CANNOT hit the v1.46.2 bug: there
  // is no reconciler to strip its inline style on a redraw, so it never re-fades.
  let btns = $state<Partial<Record<Tab, HTMLButtonElement>>>({})
  let ind = $state({ left: 0, width: 0 })

  function measure() {
    const b = btns[route.tab]
    if (b) ind = { left: b.offsetLeft, width: b.offsetWidth }
  }
  // Re-measure whenever the active tab changes or the buttons mount.
  $effect(() => {
    route.tab
    measure()
  })
</script>

<svelte:window onresize={measure} />

<nav class="tabs">
  {#each TABS as t (t)}
    <button
      bind:this={btns[t]}
      class="tab"
      class:active={route.tab === t}
      onclick={() => go(t)}
    >
      {TAB_LABEL[t]}
    </button>
  {/each}
  <span class="underline" style="transform: translateX({ind.left}px); width: {ind.width}px"></span>
</nav>

<style>
  .tabs { position: relative; display: flex; gap: 20px; border-bottom: 1px solid #1c2230; }
  .tab {
    background: none; border: 0; color: #8b93a7; font: inherit; font-size: 14px;
    padding: 10px 2px; cursor: pointer; transition: color 0.15s;
  }
  .tab:hover { color: #c4cbd8; }
  .tab.active { color: #e6e9ef; }
  .underline {
    position: absolute; bottom: -1px; left: 0; height: 2px; background: #3ecf8e;
    transition: transform 0.22s ease, width 0.22s ease;
  }
</style>
