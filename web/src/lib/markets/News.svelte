<script lang="ts">
  import type { NewsItem } from '../model'

  let { news }: { news: NewsItem[] } = $props()

  function ago(iso: string): string {
    const t = Date.parse(iso)
    if (!t) return ''
    const mins = Math.round((Date.now() - t) / 60000)
    if (mins < 60) return mins + 'm'
    const hrs = Math.round(mins / 60)
    if (hrs < 24) return hrs + 'h'
    return Math.round(hrs / 24) + 'd'
  }
  const chg = (n: number | null) => (n == null ? '' : (n >= 0 ? '+' : '') + n.toFixed(2) + '%')
</script>

<div class="card">
  <h5>News</h5>
  <div class="scroll">
    {#each news.slice(0, 120) as n (n.id)}
      <a class="item" href={n.url} target="_blank" rel="noopener noreferrer">
        <div class="hl">{n.headline}</div>
        <div class="meta">
          <span class="src">{n.source}</span>
          <span class="dot">·</span>
          <span class="time">{ago(n.publishedAt)}</span>
          {#each n.tags as t (t.symbol + t.exchange)}
            <span class="tag {t.percentChange == null ? '' : t.percentChange >= 0 ? 'pos' : 'neg'}">{t.symbol}{#if t.percentChange != null}<i>{chg(t.percentChange)}</i>{/if}</span>
          {/each}
        </div>
      </a>
    {/each}
  </div>
</div>

<style>
  .card { background: #141924; border: 1px solid #1c2230; border-radius: 12px; padding: 16px 18px; }
  .card h5 { margin: 0 0 10px; font-size: 14px; font-weight: 600; }
  .scroll { max-height: 430px; overflow-y: auto; }
  .item { display: block; padding: 9px 0; border-bottom: 1px solid #12161f; text-decoration: none; color: inherit; }
  .item:hover .hl { color: #fff; }
  .hl { font-size: 13px; line-height: 1.35; color: #e6e9ef; }
  .meta { display: flex; align-items: center; gap: 6px; flex-wrap: wrap; margin-top: 4px; }
  .src { color: #8b93a7; font-size: 11px; }
  .dot { color: #5b6474; }
  .time { color: #8b93a7; font-size: 11px; }
  .tag { background: #1c2230; border-radius: 4px; padding: 1px 6px; font-size: 10.5px; color: #c4cbd8; display: inline-flex; gap: 4px; align-items: center; }
  .tag i { font-style: normal; font-variant-numeric: tabular-nums; }
  .tag.pos i { color: #3ecf8e; } .tag.neg i { color: #f0616d; }
</style>
