<script lang="ts">
  import { keyed } from '../keys'
  // The News card (newsCardHtml): the tabbed card — Stories / Releases /
  // Disclosures — with a scope segment, a symbol chip, a search box, and one line
  // per item. Stories and releases come from the model's news feed; releases also
  // merge the issuer's filed releases; disclosures come from /api/filings/feed (or
  // one listing's /api/filings under a chip).
  import type { NewsItem, NewsTag } from '../model'
  import { signedPct, newsWhen, discDate, newsTextKey } from './util'
  import { bareSymbol, symText } from '../sym'
  import { sort, sortRows } from '../sort.svelte'
  import { store } from '../state.svelte'
  import { sugQuotes, sugKey, sugQuoteSchedule } from './quotes.svelte'
  import { discFeed, discBySym, showDiscFeed, showDisclosures, discTitleComing, type DiscRow } from './disc.svelte'
  import { titleComing } from '../trade/discStore.svelte'
  import Mseg from './Mseg.svelte'
  import GridHead from './GridHead.svelte'
  import { call } from '../api'
  import { searchSymbols } from '../api'
  import { tickerKey, bookListing, directoryListing, type Chip } from './newsChip'
  import { escapable } from '../escape'

  let { news }: { news: NewsItem[] } = $props()


  const SCOPE_OPTS = [['all', 'All'], ['holdings', 'Holdings'], ['watchlist', 'Watchlist']] as const
  const KIND_OPTS = [['stories', 'Stories'], ['releases', 'Releases'], ['disc', 'Disclosures']] as const
  const NEWS_COLS = [
    { key: 'when', label: 'When' },
    { key: 'news', label: 'News' },
    { key: 'symbol', label: 'Symbol', align: 'right' as const },
    { key: 'change', label: 'Change', align: 'right' as const },
  ]
  const NDISC_COLS = [
    { key: 'when', label: 'When' },
    { key: 'news', label: 'Disclosure' },
    { key: 'symbol', label: 'Symbol', align: 'right' as const },
  ]
  const FILED_RELEASE = /news release|press release/i

  let scope = $state((() => { try { return localStorage.getItem('bh2.news') || 'all' } catch { return 'all' } })())
  let kind = $state((() => { try { const k = localStorage.getItem('bh2.newsKind'); return k === 'disc' || k === 'releases' ? k : 'stories' } catch { return 'stories' } })())
  let query = $state('')
  let sym = $state<Chip | null>(null)
  // Escape clears what narrows the card: the words typed in its box first, while the box
  // has the focus (which it keeps, so a second press clears the chip), then the chip
  let box = $state<HTMLInputElement>()
  $effect(() =>
    escapable(() => {
      if (query && document.activeElement === box) { query = ''; return true }
      if (sym) { newsChip(null); return true }
      return false
    }),
  )
  let kindChosen: string | null = null
  let reading = $state('')

  // --- the chip and its borrowed tab (newsKindFor / newsChip) ---
  function newsKindFor(only: Chip): string {
    if (!only || kind === 'disc') return kind
    const mine = (n: NewsItem) => (n.tags || []).some((t) => bareSymbol(t.symbol) === only.symbol && String(t.exchange || '').toUpperCase() === only.exchange)
    const has = (k: string) => (news || []).some((n) => (k === 'releases' ? n.kind === 'release' : n.kind !== 'release') && mine(n))
    if (has(kind)) return kind
    const other = kind === 'releases' ? 'stories' : 'releases'
    return has(other) ? other : kind
  }
  function newsChip(only: Chip | null) {
    if (only && kindChosen == null) kindChosen = kind
    sym = only
    if (only) kind = newsKindFor(only)
    else if (kindChosen != null) {
      kind = kindChosen
      kindChosen = null
    }
  }
  function pickScope(v: string) {
    newsChip(null)
    scope = v
    try { localStorage.setItem('bh2.news', v) } catch { /* ignore */ }
  }
  function pickKind(v: string) {
    kindChosen = null
    kind = v === 'disc' || v === 'releases' ? v : 'stories'
    try { localStorage.setItem('bh2.newsKind', kind) } catch { /* ignore */ }
  }

  // --- the chip's own change, when the listing is neither held nor watched ---
  const chipPct = $derived.by<number | null>(() => {
    const only = sym
    if (!only) return null
    const held = (store.model?.positions || []).some((p) => bareSymbol(p.symbol).toUpperCase() === only.symbol)
    const watched = (store.model?.markets?.watchlist || []).some((w) => bareSymbol(w.symbol).toUpperCase() === only.symbol)
    if (held || watched) return null
    return sugQuotes[sugKey(only)]?.percentChange ?? null
  })

  // --- filed releases: the issuer's own releases, beside the wires' ---
  function tagOf(s: string, ex: string): NewsTag {
    const w = (store.model?.markets?.watchlist || []).find((x) => bareSymbol(x.symbol) === s)
    const p = (store.model?.positions || []).find((x) => bareSymbol(x.symbol) === s)
    return { symbol: s, exchange: ex || (p && p.exchange) || (w && w.exchange) || '', held: !!p, watched: !!w, percentChange: p ? (p.percentChange == null ? null : p.percentChange * 100) : w ? w.percentChange : null, positionId: p ? p.id : null }
  }
  function filedReleases(only: Chip | null, sc: string, wire: NewsItem[]): NewsItem[] {
    let rows: DiscRow[]
    // pure read only — the feed/payload load and title enrichment happen in the
    // effects below, never inside this derived
    if (only) {
      const rec = discBySym[only.symbol]
      if (!rec || !rec.payload) return []
      rows = (rec.payload.filings || []).map((f) => ({ ...f, symbol: only.symbol, exchange: only.exchange || '' }))
    } else {
      rows = discFeed[sc]?.data?.filings || []
    }
    const said = new Set((wire || []).map((n) => newsTextKey(n.headline)))
    return rows
      .filter((f) => FILED_RELEASE.test(String(f.type || '')) && !said.has(newsTextKey(f.subject || '')))
      .map((f) => ({
        id: 'filed:' + f.id,
        headline: f.subject || f.type || 'News release',
        source: f.source || '',
        url: f.url || '',
        publishedAt: String(f.date || ''),
        kind: 'release',
        market: false,
        tags: [tagOf(f.symbol || '', f.exchange || '')],
        filed: { id: f.id, sym: f.symbol, source: f.source, url: f.url },
      })) as (NewsItem & { filed?: { id: string; sym?: string; source: string; url: string } })[]
  }

  // --- the story / release rows ---
  const releases = $derived(kind === 'releases')
  const bare = $derived(scope === 'all' && !sym && !releases)
  const storyRows = $derived.by(() => {
    const only = sym
    const isOnly = (t: NewsTag) => !!only && bareSymbol(t.symbol) === only.symbol && String(t.exchange || '').toUpperCase() === only.exchange
    let rows = (news || []).filter(
      (n) => (releases ? n.kind === 'release' : n.kind !== 'release') && (only ? n.tags.some(isOnly) : scope === 'all' ? (releases ? n.tags.length > 0 : n.market) : n.tags.some((t) => (scope === 'holdings' ? t.held : t.watched))),
    ) as (NewsItem & { filed?: { id: string; sym?: string; source: string; url: string } })[]
    if (releases) rows = rows.concat(filedReleases(only, scope, rows))
    const q = query.trim().toUpperCase()
    if (q) {
      const tokens = q.split(/\s+/)
      const bySym = tokens.length === 1 && q.length <= 5 ? rows.filter((n) => n.tags.some((t) => bareSymbol(t.symbol).startsWith(q))) : []
      const text = (n: NewsItem) => (n.headline + ' ' + n.source).toUpperCase()
      rows = bySym.length ? bySym : rows.filter((n) => { const h = text(n); return tokens.every((t) => h.indexOf(t) >= 0) })
    }
    let s = sort.news
    if (bare && s.key !== 'when' && s.key !== 'news') s = { key: 'when', dir: 'desc' }
    const first = (n: NewsItem) => n.tags[0] || ({} as NewsTag)
    return sortRows(rows, s.key, s.dir, (n, k) => (k === 'when' ? n.publishedAt : k === 'news' ? n.headline.toLowerCase() : k === 'symbol' ? (first(n).symbol ? bareSymbol(first(n).symbol).toLowerCase() : null) : first(n).percentChange))
  })

  // --- the disclosures view ---
  const discView = $derived.by(() => {
    const only = sym
    let rows: DiscRow[]
    let empty = 'Nothing filed.'
    // pure read only — loading and enrichment run in the effects below
    if (only) {
      const rec = discBySym[only.symbol]
      if (!rec || rec.loading) return { state: 'reading' as const }
      if (rec.error) return { state: 'error' as const, error: rec.error }
      const p = rec.payload!
      rows = (p.filings || []).map((f) => ({ ...f, symbol: only.symbol, exchange: only.exchange || '' }))
      if (!rows.length) empty = Object.values(p.sources || {}).some((x) => x && x.filer) ? 'Nothing filed.' : 'No regulatory filer for this listing.'
    } else {
      const f = discFeed[scope]?.data
      if (!f) return { state: 'reading' as const }
      rows = f.filings
    }
    const q = query.trim().toUpperCase()
    if (q) {
      const tokens = q.split(/\s+/)
      const bySym = tokens.length === 1 && q.length <= 6 ? rows.filter((f) => String(f.symbol || '').startsWith(q)) : []
      const text = (f: DiscRow) => (String(f.subject || '') + ' ' + String(f.type || '') + ' ' + String(f.source || '')).toUpperCase()
      rows = bySym.length ? bySym : rows.filter((f) => { const h = text(f); return tokens.every((t) => h.indexOf(t) >= 0) })
    }
    const s = sort.ndisc
    rows = sortRows(rows, s.key, s.dir, (f, k) => (k === 'when' ? f.date : k === 'news' ? String(f.subject || f.type || '').toLowerCase() : String(f.symbol || '').toLowerCase()))
    return { state: 'rows' as const, rows, empty }
  })

  const coming = (f: DiscRow) => (sym ? titleComing(f, sym.symbol.toUpperCase()) : discTitleComing(f, scope))

  // The disclosures this view shows are sent while it shows them, and stop when it
  // stops: one listing's under the chip, or the scope's feed. The server reads the
  // documents; their titles reach the rows as they are read.
  $effect(() => {
    if (kind !== 'releases' && kind !== 'disc') return
    if (sym) return showDisclosures({ symbol: sym.symbol, exchange: sym.exchange || '', name: sym.name || '', currency: sym.currency || '' })
    return showDiscFeed(scope)
  })
  // A chip for a listing the book neither holds nor watches: fetch its quote.
  $effect(() => {
    const only = sym
    if (!only) return
    const held = (store.model?.positions || []).some((p) => bareSymbol(p.symbol).toUpperCase() === only.symbol)
    const watched = (store.model?.markets?.watchlist || []).some((w) => bareSymbol(w.symbol).toUpperCase() === only.symbol)
    if (held || watched) return
    // asked again only once the one remembered is a minute old
    sugQuoteSchedule([{ symbol: only.symbol, exchange: only.exchange, currency: only.currency }])
  })

  // --- on-demand chip lookup: a ticker typed that the card does not hold ---
  let lookupTimer: ReturnType<typeof setTimeout> | undefined
  $effect(() => {
    const key = tickerKey(query)
    clearTimeout(lookupTimer)
    if (sym || !key) return
    lookupTimer = setTimeout(() => {
      if (sym || query.trim().toUpperCase() !== key) return
      const take = (only: Chip) => {
        if (query.trim().toUpperCase() !== key) return
        newsChip(only)
        query = ''
        const items = (store.model?.markets?.news || []).some((n) => n.tags.some((t) => bareSymbol(t.symbol).toUpperCase() === only.symbol))
        if (kind !== 'disc' && !items) {
          reading = only.symbol
          call('GET /api/news/symbol', { query: { symbol: only.symbol, exchange: only.exchange, currency: only.currency || '', name: '' } }).then((r) => {
            reading = ''
            if (r && r.ok && r.exchange && sym === only) only.exchange = String(r.exchange).toUpperCase()
            // the items it read reach the card as rows inserted into the news
          })
        }
      }
      const book = bookListing(key, [...(store.model?.markets?.watchlist || []), ...(store.model?.positions || []), ...(store.model?.trades || [])])
      if (book) return take(book)
      // a word no directory names as a ticker stays a text search
      searchSymbols(key).then((m) => {
        const listed = directoryListing(key, m)
        if (listed) take(listed)
      })
    }, 500)
  })

  // --- actions ---
  function newsSym(t: { symbol: string; exchange: unknown }) {
    newsChip({ symbol: bareSymbol(t.symbol), exchange: String(t.exchange || '').toUpperCase() })
  }
  function openStory(n: NewsItem & { filed?: { id: string; sym?: string; source: string; url: string } }) {
    if (n.filed) return openDisc(n.filed)
    if (n.url) window.open(n.url, '_blank', 'noopener')
  }
  function openDisc(f: { id: string; sym?: string; source?: string; url?: string }) {
    const durl = f.source === 'SEC' && f.url ? f.url : '/api/filings/doc?symbol=' + encodeURIComponent(f.sym || '') + '&id=' + encodeURIComponent(f.id)
    window.open(durl, '_blank', 'noopener')
  }

  const first = (n: NewsItem) => n.tags[0] || ({} as NewsTag)
  function chgVal(t: NewsTag): number | null {
    return t.percentChange == null && sym && bareSymbol(t.symbol).toUpperCase() === sym.symbol ? chipPct : t.percentChange
  }
  const chgColor = (v: number | null) => (v == null ? 'var(--ink55)' : v >= 0 ? 'var(--pos)' : 'var(--neg)')

  // `Reading…` stands where `No news.` would while something is still reading for what
  // the card shows: the chip's own lookup, or the server's pass with a listing in the
  // card's scope (or the chip) still to read (the status names them; `*` is the market's feed)
  const passReading = $derived.by(() => {
    if (sym && reading === sym.symbol) return true
    const left = store.model?.status?.newsReading ?? []
    if (!left.length) return false
    const has = (s: string) => left.includes(bareSymbol(s).toUpperCase())
    if (sym) return has(sym.symbol)
    const held = (store.model?.positions ?? []).map((p) => p.symbol)
    const watched = (store.model?.markets.watchlist ?? []).map((w) => w.symbol)
    if (scope === 'holdings') return held.some(has)
    if (scope === 'watchlist') return watched.some(has)
    return kind === 'releases' ? held.concat(watched).some(has) : left.includes('*')
  })
</script>

<div class="card elev-sm" style="padding:14px 16px 12px;display:flex;flex-direction:column;min-height:0">
  <div style="display:flex;align-items:center;gap:14px;min-height:28px;margin-bottom:8px">
    <h5>News</h5>
    <Mseg options={SCOPE_OPTS} cur={scope} onpick={pickScope} />
    <Mseg options={KIND_OPTS} cur={kind} onpick={pickKind} />
    {#if sym}
      <span class="chip"><span class="cf">Symbol</span><span class="cv" style="cursor:default">{symText(sym.symbol)}</span><button class="cx" aria-label="Show every listing" onclick={() => newsChip(null)}>×</button></span>
    {/if}
    <div style="margin-left:auto;display:flex;align-items:center;gap:7px;padding:4px 9px;width:200px;border-radius:6px;background:var(--field);box-shadow:inset 0 0 0 1px rgba(var(--ink-rgb),.1)">
      <input bind:this={box} bind:value={query} placeholder={kind === 'disc' ? 'Search symbol or filing' : 'Search symbol or headline'} aria-label="Search the news" autocomplete="off" style="flex:1;min-width:0;border:0;background:transparent;outline:none;font:400 12.5px var(--font);color:var(--ink)" />
    </div>
  </div>

  {#if kind === 'disc'}
    {#if discView.state === 'reading'}
      <div class="muted" style="font-size:12px">Reading…</div>
    {:else if discView.state === 'error'}
      <div class="status-err" style="font-size:12px">{discView.error}</div>
    {:else if discView.rows.length}
      <div class="nw-head nw-disc"><GridHead table="ndisc" cols={NDISC_COLS} /></div>
      <div class="scroll nw-list" style="flex:1;min-height:0;max-height:436px">
        {#each keyed(discView.rows, (r) => r.id) as { row: f, key } (key)}
          <div class="nw-row nw-disc" role="button" tabindex="-1" onclick={() => openDisc({ id: f.id, sym: f.symbol, source: f.source, url: f.url })} onkeydown={(e) => { if (e.key === 'Enter') openDisc({ id: f.id, sym: f.symbol, source: f.source, url: f.url }) }}>
            <div class="tab" style="font-size:11px;color:var(--ink55)">{discDate(f)}</div>
            <div>
              <div style="font-size:13px;overflow:hidden;text-overflow:ellipsis">{#if coming(f)}<span class="dc-skel" style="width:70%"></span>{:else}{f.subject || f.type || ''}{/if}</div>
              <div style="font-size:11px;color:var(--ink55);overflow:hidden;text-overflow:ellipsis">{(f.subject || coming(f) ? String(f.type || '') + ' · ' : '') + String(f.source || '')}</div>
            </div>
            <div style="text-align:right"><button class="nw-sym go" onclick={(e) => { e.stopPropagation(); newsSym({ symbol: f.symbol || '', exchange: f.exchange || '' }) }}>{f.symbol}</button></div>
          </div>
        {/each}
      </div>
    {:else}
      <div class="muted" style="font-size:12px">{discView.empty}</div>
    {/if}
  {:else if storyRows.length}
    <div class="nw-head" class:nw-bare={bare}><GridHead table="news" cols={bare ? NEWS_COLS.slice(0, 2) : NEWS_COLS} /></div>
    <div class="scroll nw-list" style="flex:1;min-height:0;max-height:436px">
      {#each keyed(storyRows, (r) => r.id) as { row: n, key } (key)}
        <div class="nw-row" class:nw-bare={bare} role="button" tabindex="-1" onclick={() => openStory(n)} onkeydown={(e) => { if (e.key === 'Enter') openStory(n) }}>
          <div class="tab" style="font-size:11px;color:var(--ink55)">{newsWhen(n.publishedAt)}</div>
          <div>
            <div style="font-size:13px;overflow:hidden;text-overflow:ellipsis">{n.headline}</div>
            <div style="font-size:11px;color:var(--ink55);overflow:hidden;text-overflow:ellipsis">{n.source}</div>
          </div>
          {#if !bare}
            <div style="text-align:right">{#if n.tags.length}<button class="nw-sym go" onclick={(e) => { e.stopPropagation(); newsSym(first(n)) }}>{bareSymbol(first(n).symbol)}</button>{/if}</div>
            <div style="text-align:right">{#if n.tags.length}<span class="tab" style="font-size:13px;color:{chgColor(chgVal(first(n)))}">{signedPct(chgVal(first(n)))}</span>{/if}</div>
          {/if}
        </div>
      {/each}
    </div>
  {:else}
    <div class="muted" style="font-size:12px">{passReading ? 'Reading…' : releases ? 'No releases.' : 'No news.'}</div>
  {/if}
</div>
