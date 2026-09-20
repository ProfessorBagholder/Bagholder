<script lang="ts">
  import { onMount } from 'svelte'
  import { notesStore, markAllRead, clearNotes, type Note } from './notes.svelte'

  let { onclose }: { onclose: () => void } = $props()

  // Opening the panel marks what is shown as read.
  onMount(() => { markAllRead() })

  function ago(iso: string): string {
    const t = Date.parse(iso)
    if (!t) return ''
    const mins = Math.round((Date.now() - t) / 60000)
    if (mins < 1) return 'now'
    if (mins < 60) return mins + 'm'
    const hrs = Math.round(mins / 60)
    if (hrs < 24) return hrs + 'h'
    return Math.round(hrs / 24) + 'd'
  }
  function open(n: Note) { if (n.href) window.open(n.href, '_blank', 'noopener') }
</script>

<div class="scrim" role="presentation" onclick={onclose}></div>
<div class="panel" role="dialog" aria-label="Notifications">
  <div class="hd">
    <span class="title">Notifications</span>
    <div class="actions">
      {#if notesStore.rows.length}<button class="clear" onclick={clearNotes}>Clear</button>{/if}
      <button class="x" onclick={onclose} aria-label="Close">×</button>
    </div>
  </div>
  <div class="body">
    {#if !notesStore.loaded}
      <p class="msg">Reading…</p>
    {:else if !notesStore.rows.length}
      <p class="msg">No activity yet.</p>
    {:else}
      {#each notesStore.rows as n (n.id)}
        <div class="row" class:link={!!n.href} role="presentation" onclick={() => open(n)}>
          <div class="r1">
            {#if !n.readAt}<span class="dot"></span>{/if}
            <span class="t">{n.title}</span>
            <span class="time">{ago(n.at)}</span>
          </div>
          {#if n.body}{#each n.body.split('\n') as line}<div class="b">{line}</div>{/each}{/if}
        </div>
      {/each}
    {/if}
  </div>
</div>

<style>
  .scrim { position: fixed; inset: 0; background: rgba(0,0,0,0.4); z-index: 60; }
  .panel { position: fixed; top: 0; right: 0; height: 100vh; width: 360px; max-width: 92vw; background: #0b0e14; border-left: 1px solid #1c2230; z-index: 61; display: flex; flex-direction: column; }
  .hd { display: flex; align-items: center; justify-content: space-between; padding: 16px 18px; border-bottom: 1px solid #1c2230; }
  .title { font-size: 15px; font-weight: 600; }
  .actions { display: flex; align-items: center; gap: 10px; }
  .clear { background: none; border: 0; color: #8b93a7; font: inherit; font-size: 12px; cursor: pointer; }
  .clear:hover { color: #e6e9ef; }
  .x { background: none; border: 0; color: #8b93a7; font-size: 20px; cursor: pointer; line-height: 1; }
  .body { flex: 1; overflow-y: auto; padding: 8px 0; }
  .msg { color: #8b93a7; font-size: 13px; padding: 12px 18px; }
  .row { padding: 10px 18px; border-bottom: 1px solid #12161f; }
  .row.link { cursor: pointer; }
  .row.link:hover { background: #141924; }
  .r1 { display: flex; align-items: center; gap: 8px; }
  .dot { width: 7px; height: 7px; border-radius: 50%; background: #3ecf8e; flex: none; }
  .t { font-size: 13px; flex: 1; min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .time { color: #8b93a7; font-size: 11px; }
  .b { color: #8b93a7; font-size: 12px; margin-top: 3px; padding-left: 15px; }
</style>
