<script lang="ts">
  import { onMount } from 'svelte'
  import { store, loadModel } from './lib/state.svelte'
  import Dashboard from './lib/Dashboard.svelte'

  onMount(() => {
    loadModel()
    // prove the reactive path: refresh the model periodically like the legacy poll
    const id = setInterval(loadModel, 30_000)
    return () => clearInterval(id)
  })
</script>

<main>
  <header>
    <span class="brand">Bagholder</span>
    <span class="v3">/v3 · Svelte spike</span>
    <span class="tab">Dashboard</span>
  </header>

  {#if store.error}
    <p class="msg err">Could not load model: {store.error}</p>
  {:else if !store.model}
    <p class="msg">Loading…</p>
  {:else}
    <Dashboard model={store.model} />
  {/if}
</main>

<style>
  :global(body) { margin: 0; background: #0b0e14; color: #e6e9ef; font-family: system-ui, -apple-system, sans-serif; }
  main { max-width: 1200px; margin: 0 auto; padding: 20px 16px 60px; }
  header { display: flex; align-items: baseline; gap: 12px; margin-bottom: 20px; }
  .brand { font-size: 20px; font-weight: 700; }
  .v3 { color: #8b93a7; font-size: 12px; }
  .tab { margin-left: auto; color: #3ecf8e; font-weight: 600; border-bottom: 2px solid #3ecf8e; padding-bottom: 2px; }
  .msg { color: #8b93a7; }
  .err { color: #f0616d; }
</style>
