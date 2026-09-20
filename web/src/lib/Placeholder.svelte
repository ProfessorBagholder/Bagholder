<script lang="ts">
  import { TAB_LABEL, type Tab } from './router.svelte'
  let { tab }: { tab: Tab } = $props()
</script>

<div class="card">
  <h5>{TAB_LABEL[tab]}</h5>
  <p>Not yet migrated — lands in Phase 2 of the Svelte migration.</p>
</div>

<style>
  .card { background: #141924; border: 1px solid #1c2230; border-radius: 12px; padding: 28px; }
  .card h5 { margin: 0 0 8px; font-size: 16px; }
  .card p { margin: 0; color: #8b93a7; font-size: 13px; }
</style>
