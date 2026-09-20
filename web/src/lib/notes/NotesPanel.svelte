<script lang="ts">
  import { onDestroy } from 'svelte'
  import { notesStore, markAllRead, clearNotes, type Note } from './notes.svelte'
  import { ICONS } from '../icons'
  import { bareSymbol } from '../sym'
  import { goSub } from '../router.svelte'

  let { onclose }: { onclose: () => void } = $props()

  // Closing is having looked: every row reads read, the badge clears. Faithful to
  // ledger.html closeNotes() — read on close, so the unread dots stay while open.
  onDestroy(() => { markAllRead() })

  const MON = ['Jan', 'Feb', 'Mar', 'Apr', 'May', 'Jun', 'Jul', 'Aug', 'Sep', 'Oct', 'Nov', 'Dec']

  function orderWhenWord(iso: string): string {
    const t = Date.parse(iso)
    if (!isFinite(t)) return '—'
    const d = new Date(t), now = new Date()
    const h = d.getHours() % 12 || 12, m = String(d.getMinutes()).padStart(2, '0'), ap = d.getHours() < 12 ? 'AM' : 'PM'
    const time = h + ':' + m + ' ' + ap
    if (d.toDateString() === now.toDateString()) return 'Today ' + time
    return MON[d.getMonth()] + ' ' + d.getDate() + (d.getFullYear() !== now.getFullYear() ? ' ' + d.getFullYear() : '') + ', ' + time
  }
  // When the thing happened, which is what a reader wants: the release's own moment,
  // the filing's own day. The app is told at some later moment, and that is its business.
  function noteWhenWord(n: Note): string {
    const at = (n.extra || {}).at
    if (!at) return orderWhenWord(n.at)
    const day = at.length === 10
    const when = orderWhenWord(day ? at + 'T12:00:00' : at)
    if (when === '—') return orderWhenWord(n.at)
    return day ? when.replace(/,? \d{1,2}:\d{2} (AM|PM)$/, '').replace(/^Today$/, 'Today') : when
  }

  function notesClose() { onclose() }

  // Every row leads to the thing it is about: a filed document or a release opens
  // where it is published, in a new tab; a row carrying a ticker opens that listing.
  // (An order notice's Orders-panel target is a gap — see report.)
  function noteOpen(n: Note) {
    const x = n.extra || {}
    if (x.doc && x.source !== 'SEC') { window.open('/api/filings/doc?symbol=' + encodeURIComponent(x.symbol || '') + '&id=' + encodeURIComponent(x.doc), '_blank', 'noopener'); return }
    if (x.url) { window.open(x.url, '_blank', 'noopener'); return }
    if (x.symbol) {
      onclose()
      goSub('markets', 'listing:' + bareSymbol(x.symbol) + '@' + String(x.exchange || '').toUpperCase())
    }
  }
</script>

<div id="ntWrap">
  <div class="tk-scrim" role="presentation" onclick={notesClose}></div>
  <div class="tk" role="dialog" aria-label="Notifications">
    <div class="tk-hd">
      <span class="tk-title">Notifications</span>
      <button class="tk-x" aria-label="Close" onclick={notesClose}><svg width="14" height="14" viewBox="0 0 256 256" fill="currentColor"><path d={ICONS.x} /></svg></button>
    </div>
    <div class="tk-body" data-keep-scroll="notes" style="gap:16px">
      <div class="od-head">
        <span class="od-group">History</span>
        {#if notesStore.rows.length}<button class="od-link muted" onclick={clearNotes}>Clear</button>{/if}
      </div>
      <div class="od-list">
        {#if notesStore.rows.length}
          {#each notesStore.rows as n (n.id)}
            <div class="od-card nt-card" role="presentation" onclick={() => noteOpen(n)}>
              <div class="od-row1"><span class="od-title" style="display:flex;align-items:center;gap:8px;min-width:0">{#if !n.readAt}<span class="nt-dot"></span>{/if}<span style="min-width:0;overflow:hidden;text-overflow:ellipsis">{n.title}</span></span></div>
              {#if n.body}{#each String(n.body).split('\n') as line}<div class="od-row2"><span class="od-line">{line}</span></div>{/each}{/if}
              <div class="od-foot"><span class="od-when">{noteWhenWord(n)}</span></div>
            </div>
          {/each}
        {:else}
          <div class="dim" style="font-size:12px">Nothing yet.</div>
        {/if}
      </div>
    </div>
  </div>
</div>
