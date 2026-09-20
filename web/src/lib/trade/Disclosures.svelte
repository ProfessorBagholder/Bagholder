<script lang="ts">
  import type { Trade, Filing } from '../model'

  let { trade }: { trade: Trade } = $props()

  let filings = $state<Filing[]>([])
  let sources = $state<string[]>([])
  let loading = $state(true)
  let noFiler = $state(false)
  let expanded = $state<string | null>(null)

  // A coin, an index, a futures/currency contract or an option has no regulatory
  // filer. Read once at mount; the parent keys TradeDetail per trade id.
  // svelte-ignore state_referenced_locally
  const HAS_FILER = trade.kind === 'Shares' || trade.kind === 'ETF'

  async function load() {
    if (!HAS_FILER) { loading = false; noFiler = true; return }
    try {
      const q = new URLSearchParams({ symbol: trade.symbol, exchange: trade.exchange || '', currency: trade.currency || '', kind: trade.kind || '' })
      const r = await fetch('/api/filings?' + q.toString())
      const d = await r.json()
      if (d.ok) {
        filings = d.filings ?? []
        sources = d.sources ?? []
        noFiler = !filings.length && !(d.available && d.available.length)
      } else {
        noFiler = true
      }
    } catch {
      /* leave empty */
    }
    loading = false
  }
  $effect(() => { load() })
</script>

<div class="card">
  <div class="head">
    <h5>Disclosures</h5>
    {#if sources.length}<span class="src">{sources.join(' · ')}</span>{/if}
  </div>

  {#if loading}
    <p class="msg">Reading…</p>
  {:else if noFiler}
    <p class="msg">No regulatory filer for this listing.</p>
  {:else if !filings.length}
    <p class="msg">Nothing filed.</p>
  {:else}
    <div class="scroll">
      {#each filings as f (f.id)}
        <div class="row" role="presentation" onclick={() => (expanded = expanded === f.id ? null : f.id)}>
          <div class="main">
            <div class="title" class:clip={expanded !== f.id}>{f.subject || f.title || f.type}</div>
            {#if f.summary}<div class="summary" class:clip={expanded !== f.id}>{f.summary}</div>{/if}
          </div>
          <div class="meta">
            <span class="date">{f.dateText || f.date}</span>
            <span class="tag">{f.source}{f.type ? ' · ' + f.type : ''}</span>
            <a class="link" href={f.url} target="_blank" rel="noopener noreferrer" onclick={(e) => e.stopPropagation()} aria-label="Open document">↗</a>
          </div>
        </div>
      {/each}
    </div>
  {/if}
</div>

<style>
  .card { background: #141924; border: 1px solid #1c2230; border-radius: 12px; padding: 16px 18px; }
  .head { display: flex; align-items: baseline; justify-content: space-between; margin-bottom: 10px; }
  .head h5 { margin: 0; font-size: 14px; font-weight: 600; }
  .src { color: #8b93a7; font-size: 11px; }
  .msg { color: #8b93a7; font-size: 13px; margin: 4px 0; }
  .scroll { max-height: 360px; overflow-y: auto; }
  .row { display: flex; gap: 12px; justify-content: space-between; padding: 9px 0; border-bottom: 1px solid #12161f; cursor: pointer; }
  .row:hover { background: #171d29; }
  .main { min-width: 0; flex: 1; }
  .title { font-size: 13px; }
  .summary { color: #8b93a7; font-size: 12px; margin-top: 2px; }
  .clip { overflow: hidden; text-overflow: ellipsis; display: -webkit-box; -webkit-line-clamp: 2; line-clamp: 2; -webkit-box-orient: vertical; }
  .meta { display: flex; flex-direction: column; align-items: flex-end; gap: 2px; white-space: nowrap; }
  .date { color: #8b93a7; font-size: 12px; }
  .tag { color: #8b93a7; font-size: 11px; }
  .link { color: #4b9fff; text-decoration: none; font-size: 14px; }
</style>
