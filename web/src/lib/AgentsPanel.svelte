<script lang="ts">
  // The Agents panel (brief 81 §3, docs/plans/agents-from-anywhere.md §4). Your agents,
  // each with a page of its own (the address it uses, its setup steps again, its reads,
  // Remove), and the addresses agents reach Bagholder at: one for every agent, and a
  // second only when an agent can't use the first (Claude.ai connects only on 443).
  // Connecting an agent: which agent, an address it can use (one set up when there is
  // none), and adding Bagholder in the agent, where a sign-in shows the code to type.
  // Built as the menu's other modals are (Modals.svelte).
  import { status as statusSlot } from './subs.svelte'
  import { copyText } from './clipboard'
  import { closeModal } from './ui.svelte'
  import { call } from './api'
  import { read } from './reads.svelte'
  import { relTime } from './fmt'
  import { ICONS } from './icons'
  import type { Address, Connected, Provider, Setup, MadeKey, LogLine } from './generated/agents'

  const st = $derived(statusSlot.data?.agents)
  let error = $state('')
  let busy = $state('')
  let copied = $state('')

  type Kind = 'claude' | 'chatgpt' | 'grok' | 'muse' | 'other'
  const AGENTS: { id: Kind; name: string; how: string }[] = [
    { id: 'claude', name: 'Claude.ai', how: 'Signs in with a code you type' },
    { id: 'chatgpt', name: 'ChatGPT', how: 'Signs in with a code you type' },
    { id: 'grok', name: 'Grok Bot', how: 'Signs in with a code you type' },
    { id: 'muse', name: 'Muse', how: 'Uses a key you paste into it' },
    { id: 'other', name: 'Another agent', how: 'Any agent that can add an MCP server' },
  ]
  /** an agent connected here is named for the agent picked (Grok Bot signs in as Cursor) */
  const kindOf = (c: Connected): Kind => AGENTS.find((a) => a.name === c.name)?.id ?? (c.way === 'api-key' ? 'muse' : 'other')
  const PROVIDER: Record<Provider, string> = { tailscale: 'Tailscale', cloudflare: 'Cloudflare', ngrok: 'ngrok', zrok: 'zrok', own: 'Your proxy' }

  type View = 'home' | 'manage' | 'server' | 'agent' | 'reach' | 'ready' | 'add' | 'done'
  let view = $state<View | ''>('')
  let kind = $state<Kind>('claude')
  const agentName = $derived(AGENTS.find((a) => a.id === kind)!.name)
  /** the agent as a sentence names it: one not on the list is "your agent", never its card's "Another agent" */
  const agentWords = $derived(kind === 'other' ? 'your agent' : agentName)
  $effect(() => {
    // an error belongs to the screen it happened on: moving to another clears it
    void view
    error = ''
  })
  $effect(() => {
    // the panel opens on your agents once one is set up, else on connecting one
    if (!view && st) view = st.allowed.length || st.addresses.length ? 'home' : 'agent'
  })

  // ---- the addresses ----
  const addresses = $derived(st?.addresses ?? [])
  const hostOf = (url: string) => url.replace(/^https:\/\//, '')
  /** given to agents once a check found it answers, or it could not be told from here */
  const up = (a: Address) => a.running && !!a.url && (a.ready?.state === 'ready' || a.ready?.state === 'unconfirmed')
  /** claude.ai connects only on 443 (anthropics/claude-ai-mcp#1155) */
  const works = (a: Address, k: Kind) => k !== 'claude' || a.port443
  const usable = (k: Kind) => addresses.filter((a) => up(a) && works(a, k))
  /** the server an agent was set up with */
  const addressOf = (c: Connected) => addresses.find((a) => !!a.url && c.at === a.url) ?? null
  const agentsAt = (a: Address) => (st?.allowed ?? []).filter((c) => c.at === a.url && !!a.url)

  async function act(what: string, run: () => Promise<{ ok: boolean; error?: string }>): Promise<boolean> {
    busy = what
    error = ''
    const r = await run()
    busy = ''
    if (!r.ok) error = r.error ?? 'Bagholder did not take it.'
    return r.ok
  }
  const switchTo = (on: boolean) => act('switch', () => call('POST /api/agents/switch', { body: { on } }))
  const look = () => act('look', () => call('POST /api/agents/tunnel/look'))
  /** a way's program installed by Bagholder where it runs: the status follows it */
  const install = (provider: Provider) => act('install', () => call('POST /api/agents/install', { body: { provider } }))
  const startTunnel = (provider: Provider) => act('start', () => call('POST /api/agents/tunnel/start', { body: { provider } }))
  const stopTunnel = (provider: Provider) => act('stop', () => call('POST /api/agents/tunnel/stop', { body: { provider } }))
  /** an address given up: its tunnel stopped, and its way gone */
  const removeAddress = (provider: Provider) => act('remove', () => call('POST /api/agents/tunnel/remove', { body: { provider } }))
  const check = (provider: Provider) => act('check', () => call('POST /api/agents/check', { body: { provider } }))
  const deny = (id: string) => act(id, () => call('POST /api/agents/requests/deny', { body: { id } }))
  const resume = () => act('resume', () => call('POST /api/agents/codes/resume'))
  const remove = (id: string) => act(id, () => call('POST /api/agents/cut-off', { body: { id } }))
  /** an agent named as the person knows it */
  const rename = (id: string, name: string) => act(id, () => call('POST /api/agents/rename', { body: { id, name } }))
  function copy(text: string, what: string) {
    copyText(text).then(
      () => {
        // a check for two seconds, then the copy icon again, as the header's copy button
        copied = what
        setTimeout(() => { if (copied === what) copied = '' }, 2000)
      },
      () => (error = 'The clipboard is not available here: select the text and copy it.'),
    )
  }
  /** six digits, as they are read aloud: two groups of three */
  const grouped = (c: string) => c.slice(0, 3) + ' ' + c.slice(3)
  async function startAgain(provider: Provider) {
    if (!st?.on && !(await switchTo(true))) return
    await startTunnel(provider)
  }

  // ---- connecting: which agent, and the address it is given ----
  /** the address this connection uses */
  let using = $state<Provider | null>(null)
  const usingAddr = $derived(addresses.find((a) => a.provider === using) ?? null)
  /** an agent reconnected at another server: its sign-in there replaces the one it had */
  let moving = $state<Connected | null>(null)
  /** what the steps are called while it is: Reconnect one that can't reach Bagholder, else changing its server */
  let movingTitle = $state('')
  /** a new server was set up on the way: Back from adding it goes to it getting ready */
  let walked = $state(false)
  function fromAgent() {
    error = ''
    allowedBefore = new Set(st?.allowed.map((a) => a.id) ?? [])
    made = null
    moving = null
    toReach('connect', 'agent')
  }

  // ---- step 2: an address set up ----
  /** what the address is set up for: the agent being connected, or another server */
  type ReachFor = 'connect' | 'another'
  let reachFor = $state<ReachFor>('connect')
  let reachBack = $state<View>('agent')
  /** the address being set up, once saved: step 3 follows it */
  let settingUp = $state<Provider | null>(null)
  const WAYS: { p: Provider; label: string; note: string }[] = [
    { p: 'tailscale', label: 'Tailscale', note: 'Free, recommended' },
    { p: 'cloudflare', label: 'Cloudflare', note: 'Free, with a domain you have on Cloudflare' },
    { p: 'ngrok', label: 'ngrok', note: 'Free, with an address ngrok gives you' },
    { p: 'own', label: 'Your own proxy', note: 'A web server you run, with your own domain' },
  ]
  /** the server of this kind already set up: connecting picks it as it is */
  const existing = (p: Provider) => addresses.find((a) => a.provider === p) ?? null
  /** why a server can't be picked here, if it can't: one of each kind, one that is
   * not answering, and Tailscale on 8443, which is no address for Claude.ai */
  const notFor = (p: Provider): string => {
    const a = existing(p)
    if (a && reachFor !== 'connect') return 'Already set up'
    if (a && moving && addressOf(moving)?.provider === p) return `${moving.name} uses it now`
    if (a && !works(a, kind)) return 'Claude.ai needs port 443, and this one is on 8443'
    if (a && !up(a)) return status(a)
    if (!a && p === 'tailscale' && reachFor === 'connect' && kind === 'claude' && port443Taken) return 'Claude.ai needs port 443, which another program is using'
    return ''
  }
  /** connecting: your tunnels first, then the kinds not set up yet; adding one: those kinds alone */
  const mine = $derived(reachFor === 'connect' ? WAYS.filter((w) => existing(w.p)) : [])
  const fresh = $derived(WAYS.filter((w) => !existing(w.p)))
  let picked = $state<Provider>('tailscale')
  /** the Cloudflare address's name, before its domain */
  let name = $state('agents')
  /** your own proxy's address, typed whole */
  let proxyHost = $state('')
  /** the name's own field, shown only when the person asks for another than "agents" */
  let naming = $state(false)
  /** the domain the Cloudflare sign-in was made for, once Cloudflare said its name, or an address made on it says it */
  const cfDomain = $derived(st?.cloudflare.domain || hostOf(addresses.find((a) => a.provider === 'cloudflare')?.url ?? '').split('.').slice(1).join('.'))
  let sendsHost = $state('')
  let hostDiffers = $state(false)
  let token = $state('')
  function toReach(f: ReachFor, back: View) {
    reachFor = f
    reachBack = back
    walked = false
    view = 'reach'
    picked = [...mine, ...fresh].find((w) => !notFor(w.p))?.p ?? 'own'
    name = 'agents'
    proxyHost = ''
    naming = false
    void look()
  }
  /** a window opened at the click, so the browser lets it open: Cloudflare's page goes in it once Bagholder has it */
  let signInWindow: Window | null = null
  const ts = $derived(st?.tailscale)
  /** 443 taken here: Tailscale asks only while no tunnel of Bagholder's runs, so an address on 8443 says it */
  const port443Taken = $derived(!!ts?.httpsPortTaken || addresses.some((a) => a.provider === 'tailscale' && !a.port443))
  const program = (p: Provider) => st?.programs.find((x) => x.provider === p)?.path ?? null
  /** what Tailscale still needs here, the first of it: nothing is shown once it has all */
  /** Tailscale is the system's service there: on Linux, the command that installs or signs it in, run there */
  const linux = $derived(st?.os === 'linux')
  const tsNeed = $derived(
    !program('tailscale') && !ts?.installed
      ? linux ? { what: 'Tailscale isn’t installed on', how: 'Run this on it, then come back here:', command: 'curl -fsSL https://tailscale.com/install.sh | sh', link: '', button: '' }
        : { what: 'Tailscale isn’t installed on', how: 'Install the Tailscale app there, from tailscale.com/download.', command: '', link: '', button: '' }
    : !ts?.signedIn
      ? linux ? { what: 'Tailscale isn’t signed in on', how: 'Run this on it and sign in, then come back here:', command: 'sudo tailscale up', link: '', button: '' }
        : { what: 'Tailscale isn’t signed in on', how: 'Open the Tailscale app there and sign in.', command: '', link: '', button: '' }
    : !ts?.https || !ts?.funnel ? { what: 'Tailscale Funnel is off for', how: 'Funnel lets agents on the internet reach it.', command: '', link: 'https://tailscale.com/kb/1223/funnel#requirements-and-limitations', button: 'How to turn it on' }
    : null,
  )
  const tsDone = $derived(!tsNeed)
  /** the computer Bagholder runs on, by its name: the page may be open on another */
  const computer = $derived(st?.computer || 'the computer running Bagholder')
  const cf = $derived(st?.cloudflare)
  const canGoOn = $derived(
    !!notFor(picked) ? false
    : reachFor === 'connect' && existing(picked) ? true
    : picked === 'tailscale' ? tsDone
    : picked === 'cloudflare' ? !!program('cloudflare') && !!cf?.signedIn && !!name.trim()
    : picked === 'ngrok' ? !!program('ngrok') && !!token.trim()
    : !!proxyHost.trim(),
  )
  async function cloudflareSignIn() {
    signInWindow = window.open('about:blank', '_blank')
    if (!(await act('cf', () => call('POST /api/agents/cloudflare/sign-in')))) {
      signInWindow?.close()
      signInWindow = null
    }
  }
  $effect(() => {
    // Cloudflare's page, once Bagholder has it, into the window opened at the click
    const url = st?.cloudflare?.signInUrl
    if (url && signInWindow && !signInWindow.closed) {
      signInWindow.location.href = url
      signInWindow = null
    }
  })
  async function fromReach() {
    // a server already set up: on to adding Bagholder in the agent
    if (reachFor === 'connect' && existing(picked)) {
      using = picked
      walked = false
      view = 'add'
      return
    }
    if (picked === 'ngrok' && !(await act('save', () => call('POST /api/agents/ngrok/token', { body: { token: token.trim() } })))) return
    const setup: Setup = picked !== 'tailscale'
      ? { provider: picked, name: picked === 'ngrok' ? '' : picked === 'own' ? proxyHost.trim() : name.trim(), funnelPort: null, sendsHost: picked === 'own' && hostDiffers && sendsHost.trim() ? sendsHost.trim() : null }
      : { provider: 'tailscale', name: '', funnelPort: port443Taken ? 8443 : 443, sendsHost: null }
    if (!st?.on && !(await switchTo(true))) return
    if (!(await act('save', () => call('POST /api/agents/tunnel', { body: setup })))) return
    // beside the addresses already set up: none of them is stopped
    if (setup.provider !== 'own' && !(await startTunnel(setup.provider))) return
    settingUp = setup.provider
    walked = true
    view = 'ready'
  }
  /** what is picked, said plainly: the one MCP server is Bagholder's, and a tunnel makes it reachable */
  const reachTitle = $derived(reachFor === 'connect' ? `Which tunnel should make Bagholder’s MCP server reachable for ${moving?.name ?? agentWords}?` : 'Which tunnel should make Bagholder’s MCP server reachable?')

  // ---- step 3: the address getting ready ----
  const setting = $derived(addresses.find((a) => a.provider === settingUp) ?? null)
  let details = $state(false)
  const stoppedOf = (a: Address | null) => !!a && a.provider !== 'own' && !a.running && !!a.exited
  /** a page the tunnel asks you to open (its sign-in, or the page that lets Funnel on): never the address it serves */
  const asks = $derived(setting?.lines.map((l) => /(https:\/\/[^\s"'<>]+)/.exec(l)?.[1]).filter((l): l is string => !!l && !(setting!.url && l.startsWith(setting!.url))).at(-1) ?? '')
  async function fromReady() {
    if (reachFor === 'connect') {
      using = settingUp
      view = 'add'
    } else {
      viewing = null
      view = 'home'
    }
  }

  // ---- step 4: add it in the agent ----
  let allowedBefore = new Set<string>()
  let made = $state<MadeKey | null>(null)
  let keyInstead = $state(false)
  const signsIn = (k: Kind) => k !== 'muse' && !(k === 'other' && keyInstead)
  /** the newest sign-in waiting: the one this agent just started */
  const asking = $derived(st?.waiting.at(-1) ?? null)
  async function makeKey(n: string, way: 'mcp' | 'api', at: string) {
    busy = 'key'
    error = ''
    const r = await call('POST /api/agents/keys', { body: { name: n, way, at } })
    busy = ''
    if (!r.ok) error = r.error
    else made = r
  }
  let doneName = $state('')
  $effect(() => {
    if (view !== 'add' || !st) return
    // signed in: a grant this agent did not have when the step began
    const grant = st.allowed.find((a) => a.way === 'sign-in' && !allowedBefore.has(a.id))
    // a key: its first read
    const key = made ? st.allowed.find((a) => a.id === made!.id && a.lastUsedAt) : null
    const now = grant ?? key
    if (!now) return
    const named = moving ? moving.name : kind !== 'other' ? agentName : now.name
    if (grant && grant.name !== named) void rename(grant.id, named)
    if (moving && moving.id !== now.id) void remove(moving.id)
    doneName = named
    made = null
    moving = null
    view = 'done'
  })

  /** where the header's ‹ goes: the screen before, or none on your agents and when nothing came before */
  const backTo = $derived<View | null>(
    view === 'manage' || view === 'server' ? 'home'
    : view === 'agent' ? (st?.allowed.length || addresses.length ? 'home' : null)
    : view === 'reach' ? reachBack
    : view === 'ready' ? 'reach'
    : view === 'add' ? (walked ? 'ready' : 'reach')
    : null,
  )
  /** a server's own page */
  let viewing = $state<Provider | null>(null)
  const viewed = $derived(addresses.find((a) => a.provider === viewing) ?? null)

  // ---- an agent's own page ----
  let managing = $state('')
  const managed = $derived(st?.allowed.find((c) => c.id === managing) ?? null)
  function manage(c: Connected) {
    managing = c.id
    kind = kindOf(c)
    using = addressOf(c)?.provider ?? usable(kind)[0]?.provider ?? null
    moving = null
    confirming = false
    log = []
    logAll = false
    view = 'manage'
    void loadLog(c.id)
  }
  /** Reconnect: the same server step as connecting one, its sign-in at the server
   * picked replacing the one it has */
  function reconnect(c: Connected, title: string) {
    allowedBefore = new Set(st?.allowed.map((a) => a.id) ?? [])
    made = null
    moving = c
    movingTitle = title
    kind = kindOf(c)
    toReach('connect', 'manage')
  }
  /** Remove, asked once more where it is */
  let confirming = $state(false)
  let logAll = $state(false)
  let log = $state<LogLine[]>([])
  async function loadLog(id: string) {
    const r = await read('GET /api/agents/log', { query: { agent: id } })
    if (r.ok && managing === id) log = r.lines
  }
  /** an agent signed in at an address gone or not answering, or a key with no address up */
  /** a key made before keys named their server has none until it is used */
  const cantReach = (c: Connected) => !!c.at && (!addressOf(c) || !up(addressOf(c)!))
  /** a read, as a sentence: what the agent asked for */
  const READ_WORDS: Record<string, string> = { trades: 'Read your trades', trade: 'Read one trade', positions: 'Read your holdings', lists: 'Read your filter lists', stats: 'Read your stats', income: 'Read your income', exposure: 'Read your exposure', net_worth: 'Read your net worth', transactions: 'Read your chequing and card transactions', markets: 'Read the markets and your watchlist' }
  const readWords = (l: LogLine) => (l.tool ? (READ_WORDS[l.tool] ?? `Read ${l.tool}`) : `Refused: ${l.refusal}`)
  const MONTHS = ['January', 'February', 'March', 'April', 'May', 'June', 'July', 'August', 'September', 'October', 'November', 'December']
  /** a day as people write it: 28 October 2026 */
  const longDay = (iso: string) => `${+iso.slice(8, 10)} ${MONTHS[+iso.slice(5, 7) - 1]} ${iso.slice(0, 4)}`
  /** names as a sentence says them: "Claude", "Claude and ChatGPT", "Claude, Muse and ChatGPT" */
  const names = (n: string[]) => (n.length < 2 ? n.join('') : `${n.slice(0, -1).join(', ')} and ${n.at(-1)}`)
  /** a server asked once more before it's removed */
  let removingAddr = $state<Provider | null>(null)
  let editing = $state('')
  let editName = $state('')
  async function endEdit(c: Connected) {
    if (editing !== c.id) return
    editing = ''
    const n = editName.replace(/\s+/g, ' ').trim().slice(0, 60)
    if (n && n !== c.name) await rename(c.id, n)
  }
  /** the name's text, all of it selected, ready to type over */
  const focus = (el: HTMLElement) => {
    el.focus()
    const r = document.createRange()
    r.selectNodeContents(el)
    const sel = window.getSelection()
    sel?.removeAllRanges()
    sel?.addRange(r)
  }
  const refusedCount = $derived(st?.refused.reduce((n, r) => n + r.count, 0) ?? 0)
  const status = (a: Address) =>
    !st?.on ? 'Off' : stoppedOf(a) ? 'Stopped' : a.provider !== 'own' && !a.running ? 'Not running' : up(a) ? 'Ready' : 'Getting ready…'
</script>

{#snippet icon(state: 'done' | 'wait' | 'bad')}
  {#if state === 'done'}
    <svg class="ag-ic" style="color:var(--pos)" viewBox="0 0 20 20" aria-hidden="true"><circle cx="10" cy="10" r="8.5" fill="none" stroke="currentColor" stroke-width="1.5" /><path d="M6.2 10.4l2.5 2.5 5.1-5.3" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" /></svg>
  {:else if state === 'wait'}
    <svg class="ag-ic ag-spin" viewBox="0 0 20 20" aria-hidden="true"><circle cx="10" cy="10" r="8" fill="none" stroke="var(--hair)" stroke-width="2.2" /><path d="M10 2a8 8 0 0 1 8 8" fill="none" stroke="var(--accent)" stroke-width="2.2" stroke-linecap="round" /></svg>
  {:else if state === 'bad'}
    <svg class="ag-ic" style="color:var(--neg)" viewBox="0 0 20 20" aria-hidden="true"><circle cx="10" cy="10" r="8.5" fill="none" stroke="currentColor" stroke-width="1.5" /><path d="M10 5.8v5.2" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" /><circle cx="10" cy="14" r="1.1" fill="currentColor" /></svg>
  {/if}
{/snippet}

{#snippet copyIcon(text: string, what: string)}
  <button class="heat-ghost" aria-label="Copy" onclick={() => copy(text, what)}><svg width="14" height="14" viewBox="0 0 256 256" fill="currentColor"><path d={copied === what ? ICONS.check : ICONS.copy} /></svg></button>
{/snippet}

{#snippet copyBox(text: string, what: string)}
  <div class="ag-copy">
    <code class="ag-field">{text}</code>
    {@render copyIcon(text, what)}
  </div>
{/snippet}

{#snippet named(c: Connected, cls: string)}
  {#if editing === c.id}
    <!-- the name itself, edited where it is: a text cursor and an underline, nothing moving -->
    <span class="ag-rename ag-editing {cls}" contenteditable="true" role="textbox" tabindex="0" aria-label="Name" bind:textContent={editName} use:focus onblur={() => endEdit(c)} onkeydown={(e) => {
      if (e.key === 'Enter') { e.preventDefault(); e.currentTarget.blur() }
      // Escape ends the edit alone, never the panel
      else if (e.key === 'Escape') { e.preventDefault(); e.stopPropagation(); editing = '' }
    }}></span>
  {:else}
    <button class="ag-rename {cls}" aria-label={`Rename ${c.name}`} onclick={() => { editing = c.id; editName = c.name }}>{c.name}<svg class="ag-pen" viewBox="0 0 256 256" fill="currentColor" aria-hidden="true"><path d={ICONS.pencil} /></svg></button>
  {/if}
{/snippet}

{#snippet way(w: { p: Provider; label: string; note: string }, note: string)}
  {@const why = notFor(w.p)}
  <button class="ag-choice" role="radio" aria-checked={picked === w.p} class:on={picked === w.p} disabled={!!why} onclick={() => { picked = w.p; if (w.p === 'tailscale') void look() }}>
    <span class="ag-name">{w.label}</span>
    <span class="muted ag-small">{why || note}</span>
  </button>
{/snippet}

{#snippet need(what: string, how: string, link: string, button: string, on = '', command = '')}
  <!-- what is missing, and where: the computer Bagholder runs on, by its name, as the page may be open on another -->
  <div class="ag-note">
    <div class="ag-grow"><div>{what}{#if on}{' '}<b>{on}</b>{/if}</div>{#if how}<div class="muted ag-small">{how}</div>{/if}{#if command}<div class="ag-sub">{@render copyBox(command, 'command')}</div>{/if}</div>
    {#if link}<a class="btn btn-secondary" href={link} target="_blank" rel="noopener">{button}</a>{/if}
  </div>
{/snippet}

{#snippet installNote(p: Provider, what: string, how: string)}
  <!-- installed by Bagholder where it runs: a download in this browser would land on this device instead -->
  {@const i = st?.installing.find((x) => x.provider === p)}
  {#if i && !i.failed}
    <div class="ag-note ag-wait">{@render icon('wait')}<span class="ag-grow">Installing {what} on <b>{computer}</b>…</span></div>
  {:else}
    <div class="ag-note">
      <div class="ag-grow">
        <div>{what} isn’t installed on <b>{computer}</b></div>
        {#if i?.failed}<div class="status-err ag-small">{i.failed}</div>{:else if how}<div class="muted ag-small">{how}</div>{/if}
      </div>
      <button class="btn btn-secondary" disabled={busy === 'install'} onclick={() => install(p)}>{i?.failed ? 'Try again' : 'Install'}</button>
    </div>
  {/if}
{/snippet}

{#snippet urlCopy(url: string, key: string)}
  <!-- the MCP server's address an agent is given, copied with the icon shown on hover -->
  <button class="ag-url ag-small ag-mono" aria-label={`Copy ${url}`} onclick={(e) => { e.stopPropagation(); copy(url, key) }} onkeydown={(e) => e.stopPropagation()}>
    <span class="muted">{url}</span>
    <svg class="ag-copyic" class:on={copied === key} viewBox="0 0 256 256" fill="currentColor" aria-hidden="true"><path d={copied === key ? ICONS.check : ICONS.copy} /></svg>
  </button>
{/snippet}

{#snippet paused()}
  <div class="ag-note ag-bad">{@render icon('bad')}<div class="ag-grow"><div>Sign-in is paused: codes were typed wrong several times.</div><div class="muted ag-small">If that wasn't you, someone else reached your address.</div></div><button class="btn btn-secondary" disabled={busy === 'resume'} onclick={resume}>Resume sign-in</button></div>
{/snippet}

{#snippet codeOrWait(waitingFor: string)}
  {#if st?.codesPaused}
    {@render paused()}
  {:else if asking}
    <div class="ag-code">
      <div class="ag-digits">{grouped(asking.code)}</div>
      {@render copyIcon(asking.code, 'code')}
      <div class="muted ag-small">{asking.who}{asking.name ? (asking.checked ? ` · ${asking.name}` : ` · calls itself “${asking.name}”`) : ''}</div>
    </div>
    <div class="ag-row ag-small"><span class="muted">Not you?</span><button class="btn btn-secondary" disabled={busy === asking.id} onclick={() => deny(asking.id)}>End it</button></div>
  {:else}
    <div class="ag-note ag-wait">{@render icon('wait')}<span>{waitingFor}</span></div>
  {/if}
{/snippet}

{#snippet steps4(k: Kind, url: string, waitingFor: string)}
  {#if signsIn(k)}
    <ol class="ag-ol">
      <li>
        {#if k === 'claude'}In Claude.ai, open Settings → Connectors → Add custom connector, name it Bagholder and paste this address:
        {:else if k === 'chatgpt'}In ChatGPT, open Plugins → Add custom MCP server. Change the type from STDIO to <b>Streamable HTTP</b>, paste this address as the URL, and leave every other field empty:
        {:else if k === 'grok'}In Grok Bot, add a remote MCP server named Bagholder with this address:
        {:else}In your agent, add a remote MCP server (Streamable HTTP) named Bagholder with this address:{/if}
        <div class="ag-sub">{@render copyBox(url + '/mcp', 'mcp')}</div>
      </li>
      <li>
        Press Connect, then type this code on the page that opens:
        <div class="ag-sub">{@render codeOrWait(waitingFor)}</div>
      </li>
    </ol>
    {#if k === 'other'}<button class="ag-link" onclick={() => { keyInstead = true; made = null }}>It can’t sign in: use a key instead</button>{/if}
  {:else}
    <ol class="ag-ol">
      {#if k === 'muse'}
        <li>In Muse, add a custom connector with this API description:<div class="ag-sub">{@render copyBox(url + '/api/agents/openapi.json', 'api')}</div></li>
      {:else}
        <li>In your agent, add a remote MCP server named Bagholder with this address:<div class="ag-sub">{@render copyBox(url + '/mcp/key', 'mcpkey')}</div></li>
      {/if}
      <li>
        Set its authorization to this Bearer key:
        <div class="ag-sub">
          {#if made}
            {@render copyBox(made.key, 'key')}
            <div class="muted ag-small ag-gap">Shown once: copy it now.</div>
          {:else}
            <button class="btn btn-primary" disabled={busy === 'key'} onclick={() => (k === 'muse' ? makeKey('Muse', 'api', url) : makeKey('Another agent', 'mcp', url))}>Make a key</button>
          {/if}
        </div>
      </li>
    </ol>
    {#if made}<div class="ag-note ag-wait">{@render icon('wait')}<span>Waiting for its first read…</span></div>{/if}
    {#if k === 'other'}<button class="ag-link" onclick={() => { keyInstead = false; made = null }}>It can sign in: use the sign-in instead</button>{/if}
  {/if}
{/snippet}

<!-- back from a download or a sign-in elsewhere: Bagholder looks again, quietly, so a click on Continue is never held up by it -->
<svelte:window onfocus={() => { if (view === 'reach') void call('POST /api/agents/tunnel/look') }} />

<div id="modalDlg" class="ag-back" role="presentation" onclick={(e) => { if (e.target === e.currentTarget) closeModal() }}>
  <div class="card elev-md ag-card" role="dialog" aria-modal="true" aria-labelledby="agTitle">
    <div class="ag-head">
      {#if backTo}<button class="tk-x ag-backbtn" aria-label="Back" onclick={() => (view = backTo!)}><svg width="14" height="14" viewBox="0 0 256 256" fill="currentColor" style="transform:scaleX(-1)"><path d={ICONS.caret} /></svg></button>{/if}
      {#if view === 'manage' && managed}
        <h5 id="agTitle" class="ag-manage-head">{@render named(managed, '')}</h5>
        <span class="ag-pill" class:ok={!cantReach(managed)} class:bad={cantReach(managed)}>{cantReach(managed) ? 'Can’t reach Bagholder' : 'Connected'}</span>
      {:else if view === 'server' && viewed}
        {@const s = status(viewed)}
        <h5 id="agTitle" class="ag-manage-head">{PROVIDER[viewed.provider]}</h5>
        <span class="ag-pill" class:ok={s === 'Ready'} class:bad={s === 'Stopped'} class:off={s === 'Off' || s === 'Not running'}>{s}</span>
      {:else}
        <h5 id="agTitle">{view === 'home' || view === '' ? 'Agents' : reachFor !== 'connect' && (view === 'reach' || view === 'ready') ? 'Add a tunnel' : view === 'agent' ? 'Connect an agent' : view === 'done' ? `Connect ${doneName || agentWords}` : moving ? movingTitle : kind === 'other' ? 'Connect an agent' : `Connect ${agentName}`}</h5>
      {/if}
      <button class="tk-x" aria-label="Close" onclick={closeModal}><svg width="14" height="14" viewBox="0 0 256 256" fill="currentColor"><path d={ICONS.x} /></svg></button>
    </div>
    {#if error}<div class="status-err ag-small">{error}</div>{/if}
    {#if st?.error}<div class="status-err ag-small">{st.error}</div>{/if}

    {#if view === 'home'}
      {#if st?.codesPaused}{@render paused()}{/if}
      {#each st?.waiting ?? [] as w (w.id)}
        <div class="ag-box ag-asks">
          <div><b>{w.who}</b>{#if w.name}{' '}<span class="muted">({w.checked ? w.name : `calls itself “${w.name}”`})</span>{/if} is signing in. Type this code on its page:</div>
          <div class="ag-code"><div class="ag-digits">{grouped(w.code)}</div>{@render copyIcon(w.code, 'code' + w.id)}</div>
          <div class="ag-row ag-small"><span class="muted">Not you?</span><button class="btn btn-secondary" disabled={busy === w.id} onclick={() => deny(w.id)}>End it</button></div>
        </div>
      {/each}
      <div class="ag-sechead">
        <span class="lbl">Your agents</span>
        <button class="btn btn-primary ag-sm" onclick={() => { kind = 'claude'; keyInstead = false; view = 'agent' }}>+ Connect agent</button>
      </div>
      <div class="ag-list">
        {#each st?.allowed ?? [] as c (c.id)}
          {@const problem = cantReach(c)}
          <button class="ag-agent ag-rowbtn" onclick={() => manage(c)}>
            <span class="ag-dot" class:ok={!problem && !!addressOf(c)} class:bad={problem} class:off={!problem && !addressOf(c)} aria-hidden="true"></span>
            <span class="ag-grow">
              <span class="ag-name ag-block">{c.name}</span>
              {#if problem}<span class="status-err ag-small ag-block">Can’t reach Bagholder</span>
              {:else if addressOf(c)}<span class="muted ag-small ag-block">Connected with {PROVIDER[addressOf(c)!.provider]}</span>{/if}
            </span>
            <svg class="ag-chev" viewBox="0 0 256 256" fill="currentColor" aria-hidden="true"><path d={ICONS.caret} /></svg>
          </button>
        {:else}
          <div class="muted ag-small ag-empty">No agents yet.</div>
        {/each}
      </div>

      <div class="ag-sechead">
        <span class="lbl">Tunnels</span>
        <button class="btn btn-secondary ag-sm" onclick={() => toReach('another', 'home')}>+ Add tunnel</button>
      </div>
      <div class="ag-list">
        {#each addresses as a (a.provider)}
          {@const s = status(a)}
          <div class="ag-agent ag-rowbtn" role="button" tabindex="0" aria-label={`${PROVIDER[a.provider]} ${s}`} onclick={() => { viewing = a.provider; removingAddr = null; view = 'server' }} onkeydown={(e) => { if (e.key === 'Enter' || e.key === ' ') { e.preventDefault(); viewing = a.provider; removingAddr = null; view = 'server' } }}>
            <span class="ag-dot" class:ok={s === 'Ready'} class:bad={s === 'Stopped'} class:off={s === 'Off' || s === 'Not running'} aria-hidden="true"></span>
            <span class="ag-grow">
              <span class="ag-name ag-block">{PROVIDER[a.provider]}{#if s !== 'Ready'}{' '}<span class="muted ag-small">· {s}</span>{/if}</span>
              {#if a.url}<span class="ag-block">{@render urlCopy(a.url + '/mcp', 'url' + a.provider)}</span>{/if}
            </span>
            <svg class="ag-chev" viewBox="0 0 256 256" fill="currentColor" aria-hidden="true"><path d={ICONS.caret} /></svg>
          </div>
        {:else}
          <div class="muted ag-small ag-empty">No tunnels yet.</div>
        {/each}
      </div>
      {#if st?.on && refusedCount}<div class="muted ag-small">Refused {refusedCount} {refusedCount === 1 ? 'request' : 'requests'} from unknown callers.</div>{/if}

    {:else if view === 'manage' && managed}
      {@const c = managed}
      {@const a = addressOf(c)}
        <dl class="ag-facts">
          <dt>Tunnel</dt>
          <dd>
            {#if a}
              <span>{PROVIDER[a.provider]}{up(a) ? '' : ' · not answering'}</span>
              {@render urlCopy(a.url + '/mcp', 'its' + a.provider)}
            {:else if c.at}
              <span class="status-err">Its tunnel is gone</span>
            {:else}
              <span class="muted">Not used yet</span>
            {/if}
            {#if cantReach(c)}<button class="btn btn-primary ag-sm ag-fix" onclick={() => reconnect(c, `Reconnect ${c.name}`)}>Reconnect</button>
            {:else}<button class="ag-link ag-block" onclick={() => reconnect(c, `Change ${c.name}’s tunnel`)}>Change tunnel</button>{/if}
          </dd>
          <dt>Added</dt>
          <dd>{longDay(c.madeAt)}{c.way === 'sign-in' && c.who ? `, through ${c.who}` : ''}</dd>
        </dl>

        <div class="lbl ag-lbl">Recent activity</div>
        <div class="ag-list">
          {#each logAll ? log : log.slice(0, 5) as l, i (i)}
            <div class="ag-agent ag-row ag-small">
              <span class="ag-grow" class:status-err={!l.tool}>{readWords(l)}</span>
              <span class="muted">{relTime(l.at)}</span>
            </div>
          {:else}
            <div class="muted ag-small ag-empty">No activity yet.</div>
          {/each}
        </div>
        {#if log.length > 5}<button class="ag-link" onclick={() => (logAll = !logAll)}>{logAll ? 'Show fewer' : `Show all ${log.length}`}</button>{/if}

        <div class="ag-foot">
          {#if confirming}
            <span class="ag-grow">Remove {c.name}? It won’t be able to read your Bagholder anymore.</span>
            <button class="btn btn-secondary" onclick={() => (confirming = false)}>Cancel</button>
            <button class="btn btn-secondary ag-remove" disabled={busy === c.id} onclick={async () => { if (await remove(c.id)) { confirming = false; view = 'home' } }}>Remove</button>
          {:else}
            <button class="ag-link ag-remove" onclick={() => (confirming = true)}>Remove agent</button>
          {/if}
        </div>

    {:else if view === 'server' && viewed}
      {@const a = viewed}
      {@const s = status(a)}
      {@const off = s === 'Off' || s === 'Stopped' || s === 'Not running'}
      {@const users = agentsAt(a)}
      <dl class="ag-facts">
        <dt>Address</dt>
        <dd class="ag-main">{#if a.url}{@render urlCopy(a.url + '/mcp', 'server' + a.provider)}{:else}<span class="muted">Not known yet</span>{/if}</dd>
        <dt>Agents</dt>
        <dd>
          {#each users as c (c.id)}<button class="ag-link" onclick={() => manage(c)}>{c.name}</button>{:else}<span class="muted">None yet</span>{/each}
        </dd>
        <dt>On</dt>
        <dd>
          <button class="ag-switch" role="switch" aria-checked={!off} aria-label={`${PROVIDER[a.provider]} on`} disabled={busy === 'start' || busy === 'stop' || busy === 'switch'} onclick={() => (off ? startAgain(a.provider) : stopTunnel(a.provider))}><span class="sw" class:on={!off}></span></button>
          {#if off && users.length}<span class="muted ag-small">While it’s off, {names(users.map((c) => c.name))} can’t reach Bagholder.</span>{/if}
        </dd>
      </dl>
      <div class="ag-foot">
        {#if removingAddr === a.provider}
          <span class="ag-grow">Remove {PROVIDER[a.provider]}?{users.length ? ` ${names(users.map((c) => c.name))} will stop reaching Bagholder.` : ''}{a.provider === 'cloudflare' ? ` This also deletes the tunnel and its address ${hostOf(a.url)} from your Cloudflare account.` : ''}</span>
          <button class="btn btn-secondary" onclick={() => (removingAddr = null)}>Cancel</button>
          <button class="btn btn-secondary ag-remove" disabled={busy === 'remove'} onclick={async () => { if (await removeAddress(a.provider)) { removingAddr = null; view = 'home' } }}>Remove</button>
        {:else}
          <button class="ag-link ag-remove" onclick={() => (removingAddr = a.provider)}>Remove tunnel</button>
        {/if}
      </div>

    {:else if view === 'agent'}
      <div class="ag-q">Which agent do you want to connect?</div>
      <div class="ag-choices" role="radiogroup" aria-label="Agent">
        {#each AGENTS as a (a.id)}
          <button class="ag-choice" role="radio" aria-checked={kind === a.id} class:on={kind === a.id} onclick={() => (kind = a.id)}>
            <span class="ag-name">{a.name}</span>
            <span class="muted ag-small">{a.how}</span>
          </button>
        {/each}
      </div>
      <div class="ag-foot">
        <button class="btn btn-primary ag-right" onclick={fromAgent}>Continue</button>
      </div>

    {:else if view === 'reach'}
      <div class="ag-q">{reachTitle}</div>
      <div class="ag-pick" role="radiogroup" aria-label="Server">
        {#if mine.length}
          <div class="lbl">Your tunnels</div>
          <div class="ag-choices">{#each mine as w (w.p)}{@render way(w, `${existing(w.p)?.url ?? ''}/mcp`)}{/each}</div>
          {#if fresh.length}<div class="lbl">New tunnel</div>{/if}
        {/if}
        {#if fresh.length}<div class="ag-choices">{#each fresh as w (w.p)}{@render way(w, w.note)}{/each}</div>
        {:else if !mine.length}<div class="muted ag-small ag-empty">Every kind of tunnel is set up already.</div>{/if}
      </div>
      {#if reachFor === 'connect' && existing(picked)}
        <!-- set up already: nothing more to do here -->
      {:else if picked === 'tailscale' && tsNeed}
        {@render need(tsNeed.what, tsNeed.how, tsNeed.link, tsNeed.button, computer, tsNeed.command)}
        {#if ts?.said}<div class="status-err ag-small">{ts.said}</div>{/if}
      {:else if picked === 'cloudflare'}
        {#if !program('cloudflare')}
          {@render installNote('cloudflare', 'Cloudflare’s connector', 'A small program from Cloudflare, called cloudflared.')}
        {:else if !cf?.signedIn}
          <div class="ag-note">
            <div class="ag-grow">
              <div>{cf?.signingIn ? 'Waiting for you in Cloudflare’s tab' : 'Sign in to Cloudflare'}</div>
              <div class="muted ag-small">{cf?.signingIn ? 'Choose the domain for your address, then Authorize. If Cloudflare shows its dashboard instead, open it again.' : 'You choose the domain for your address there.'}</div>
              {#if cf?.said}<div class="status-err ag-small">{cf.said}</div>{/if}
            </div>
            {#if cf?.signingIn && cf.signInUrl}<a class="btn btn-secondary" href={cf.signInUrl} target="_blank" rel="noopener">Open it again</a>
            {:else}<button class="btn btn-secondary" disabled={busy === 'cf'} onclick={cloudflareSignIn}>Sign in</button>{/if}
          </div>
        {:else if naming}
          <label class="ag-field-l">
            <span>Name for the address</span>
            <div class="ag-copy"><input class="input ag-grow" bind:value={name} spellcheck="false" aria-label="Name for the address" />{#if cfDomain}<span class="muted">.{cfDomain}</span>{/if}</div>
          </label>
        {:else}
          {#if cfDomain}<div>Your address will be <b class="ag-mono">{(name.trim() || 'agents') + '.' + cfDomain}</b></div>
          {:else}<div>Your address will be <b class="ag-mono">{name.trim() || 'agents'}</b> on the domain you chose at Cloudflare</div>{/if}
          <button class="ag-link" onclick={() => (naming = true)}>Use a different name</button>
        {/if}
      {:else if picked === 'ngrok'}
        {#if !program('ngrok')}
          {@render installNote('ngrok', 'ngrok', '')}
        {:else}
          <!-- ngrok runs only for a signed-in account, and a program signs in with the account's authtoken: ngrok has no sign-in page for programs -->
          <ol class="ag-ol">
            <li>
              Sign in to ngrok (free) and copy your authtoken
              <div class="ag-sub"><a class="btn btn-secondary" href="https://dashboard.ngrok.com/get-started/your-authtoken" target="_blank" rel="noopener">Open ngrok</a></div>
            </li>
            <li>
              Paste it here
              <div class="ag-sub"><input class="input ag-wide" type="password" bind:value={token} spellcheck="false" autocomplete="off" aria-label="ngrok authtoken" /></div>
            </li>
          </ol>
        {/if}
      {:else if picked === 'own'}
        <label class="ag-field-l">
          <span>Your proxy’s address</span>
          <input class="input" bind:value={proxyHost} spellcheck="false" placeholder="agents.yourdomain.com" />
        </label>
        {#if hostDiffers}
          <label class="ag-field-l"><span>The Host your proxy sends</span><input class="input" bind:value={sendsHost} spellcheck="false" /></label>
        {:else}<button class="ag-link" onclick={() => (hostDiffers = true)}>My proxy sends a different Host</button>{/if}
      {/if}
      <div class="ag-foot">
        <button class="btn btn-primary ag-right" disabled={!canGoOn || !!busy} onclick={fromReach}>{busy === 'save' || busy === 'start' || busy === 'switch' ? 'Starting…' : 'Continue'}</button>
      </div>

    {:else if view === 'ready'}
      {@const a = setting}
      <div class="ag-q">{a && up(a) && a.ready?.state === 'ready' ? 'Your tunnel is ready' : 'Getting your tunnel ready'}</div>
      {#if a?.provider === 'own' && st?.port}
        <div class="ag-field-l"><span>Point your proxy at this, keeping the Host it receives</span>{@render copyBox(`http://127.0.0.1:${st.port}`, 'target')}</div>
      {/if}
      <div class="ag-box ag-steps">
        {#if a && a.provider !== 'own'}
          <div class="ag-step">
            {@render icon(a.running ? 'done' : stoppedOf(a) ? 'bad' : 'wait')}
            <div class="ag-grow">
              <div>{a.running ? `${PROVIDER[a.provider]} started` : stoppedOf(a) ? `${PROVIDER[a.provider]} stopped` : `Starting ${PROVIDER[a.provider]}…`}</div>
              {#if stoppedOf(a) && ts?.operator === false && a.provider === 'tailscale'}
                <div class="muted ag-small">If Tailscale says this user may not start Funnel, run this once in a terminal, then Start it again:</div>
                <div class="ag-sub">{@render copyBox('sudo tailscale set --operator=$USER', 'operator')}</div>
              {:else if stoppedOf(a)}<div class="muted ag-small">Check that you’re still signed in to {PROVIDER[a.provider]}.</div>{/if}
            </div>
            {#if stoppedOf(a)}<button class="btn btn-secondary" disabled={busy === 'start'} onclick={() => startAgain(a.provider)}>Start it again</button>{/if}
          </div>
        {/if}
        {#if !stoppedOf(a)}
          <div class="ag-step">
            {@render icon(a?.ready?.state === 'ready' ? 'done' : a?.ready ? 'bad' : 'wait')}
            <div class="ag-grow">
              <div>{a?.ready?.state === 'ready' ? 'Your tunnel answers from the internet' : a?.ready?.state === 'unconfirmed' ? 'Bagholder couldn’t confirm your tunnel answers' : a?.ready?.state === 'failed' ? 'Your tunnel didn’t answer yet' : 'Checking that your tunnel answers'}</div>
              {#if !a?.ready}<div class="muted ag-small">This can take a minute or two the first time.</div>{/if}
              {#if a?.ready?.state === 'unconfirmed'}<div class="muted ag-small">It may still work. Try it from {reachFor === 'connect' ? agentWords : 'your agent'}.</div>{/if}
              {#if a?.ready?.state === 'failed'}<div class="muted ag-small">{a.ready.said}</div>{/if}
            </div>
            {#if a?.ready && a.ready.state !== 'ready'}<button class="btn btn-secondary" disabled={busy === 'check'} onclick={() => check(a.provider)}>{busy === 'check' ? 'Checking…' : 'Check again'}</button>{/if}
          </div>
        {/if}
      </div>
      {#if asks && !(a && up(a))}
        {@render need(`${a ? PROVIDER[a.provider] : 'The tunnel'} needs you to approve this on its site`, '', asks, 'Open')}
      {/if}
      {#if a?.ready?.state === 'ready' && a.url}
        <div class="ag-row"><span class="muted">Your address:</span><span class="ag-mono">{hostOf(a.url)}</span></div>
      {/if}
      {#if a && (stoppedOf(a) || a.ready?.state === 'failed' || a.ready?.state === 'unconfirmed') && (a.lines.length || a.exited)}
        <button class="ag-link" aria-expanded={details} onclick={() => (details = !details)}>{details ? 'Hide details' : 'Details for troubleshooting'}</button>
        {#if details}<pre class="scroll ag-pre">{a.exited ? a.exited + '\n' : ''}{a.lines.join('\n')}</pre>{/if}
      {/if}
      <div class="ag-foot">
        <button class="btn btn-primary ag-right" disabled={!(a && up(a))} onclick={fromReady}>{a?.ready?.state === 'unconfirmed' ? 'Continue anyway' : 'Continue'}</button>
      </div>

    {:else if view === 'add' && usingAddr}
      <div class="ag-q">Now add Bagholder in {agentWords}</div>
      {@render steps4(kind, usingAddr.url, `Waiting for ${agentWords}…`)}

    {:else if view === 'done'}
      <div class="ag-done">
        <svg class="ag-big" style="color:var(--pos)" viewBox="0 0 20 20" aria-hidden="true"><circle cx="10" cy="10" r="9" fill="none" stroke="currentColor" stroke-width="1" /><path d="M6 10.5l2.5 2.5L14 7.5" fill="none" stroke="currentColor" stroke-width="1.4" stroke-linecap="round" stroke-linejoin="round" /></svg>
        <div class="ag-q">{doneName} is connected</div>
        <p class="ag-p">Try asking it for your last five trades.</p>
      </div>
      <div class="ag-foot"><button class="btn btn-primary ag-right" onclick={() => (view = 'home')}>Done</button></div>
    {/if}
  </div>
</div>

<style>
  .ag-back { position: fixed; inset: 0; z-index: 20; background: rgba(8, 9, 16, .55); display: flex; align-items: center; justify-content: center; padding: 20px }
  .ag-card { width: 640px; max-width: 100%; padding: 18px 20px; max-height: 90vh; overflow: auto; display: flex; flex-direction: column; gap: 14px; font-size: 13px; line-height: 1.5 }
  .ag-head { display: flex; align-items: center; gap: 10px }
  .ag-manage-head { margin: 0; min-width: 0 }
  .ag-rename { display: inline-flex; align-items: center; gap: 6px; background: none; border: 0; padding: 0; font: inherit; font-weight: 600; color: inherit; cursor: pointer }
  .ag-pen { width: 13px; height: 13px; opacity: 0; transition: opacity .12s }
  .ag-rename:hover .ag-pen, .ag-rename:focus-visible .ag-pen { opacity: .75 }
  .ag-small { font-size: 12px }
  .ag-mono { font-family: ui-monospace, SFMono-Regular, Menlo, monospace }
  .ag-p { margin: 0; color: var(--ink2, inherit) }
  .ag-q { font-size: 14px; font-weight: 600 }
  .ag-lbl { margin-bottom: -6px }
  .ag-grow { flex: 1; min-width: 0 }
  .ag-row { display: flex; align-items: center; gap: 10px }
  .ag-right { margin-left: auto }
  .ag-foot { display: flex; align-items: center; gap: 10px; margin-top: 4px }
  .ag-box { border: 1px solid var(--hair); border-radius: 10px; padding: 12px 14px }
  .ag-asks { border-color: var(--accent); display: flex; flex-direction: column; gap: 8px }
  .ag-dot { width: 9px; height: 9px; border-radius: 50%; background: var(--draft); flex: none }
  .ag-dot.ok { background: var(--pos) }
  .ag-dot.bad { background: var(--neg) }
  .ag-list { display: flex; flex-direction: column }
  .ag-agent { padding: 10px 0; border-top: 1px solid var(--hair) }
  .ag-agent:last-child { border-bottom: 1px solid var(--hair) }
  .ag-name { font-weight: 600 }
  .ag-remove { color: var(--neg) }
  .ag-facts { display: grid; grid-template-columns: 96px 1fr; gap: 10px 16px; margin: 4px 0 6px }
  .ag-facts dt { color: var(--ink2, inherit); opacity: .7; font-size: 12.5px }
  .ag-facts dd { margin: 0; display: flex; flex-direction: column; align-items: flex-start; gap: 2px; min-width: 0 }
  .ag-fix { margin-top: 6px }
  .ag-facts .ag-main .muted { color: inherit; opacity: 1 }
  .ag-switch { background: none; border: 0; padding: 0; margin-top: 2px; cursor: pointer; display: inline-flex }
  .ag-switch:disabled { cursor: default; opacity: .6 }
  .ag-pick { display: flex; flex-direction: column; gap: 8px }
  .ag-backbtn { margin: 0 0 0 -6px }
  .ag-pill + .tk-x { margin-left: 0 }
  .ag-pill { margin-left: auto; display: inline-flex; align-items: center; gap: 6px; font-size: 12px; padding: 2px 10px; border-radius: 999px; border: 1px solid var(--hair) }
  .ag-pill::before { content: ''; width: 7px; height: 7px; border-radius: 50%; background: var(--draft) }
  .ag-pill.ok::before { background: var(--pos) }
  .ag-pill.bad::before { background: var(--neg) }
  .ag-sechead { display: flex; align-items: center; justify-content: space-between; margin-bottom: -6px }
  .ag-sm { padding: 4px 10px; font-size: 12.5px }
  .ag-block { display: block }
  .ag-rowbtn { display: flex; align-items: center; gap: 12px; width: 100%; background: none; border: 0; border-top: 1px solid var(--hair); margin: 0; padding: 10px 4px 10px 0; font: inherit; color: inherit; text-align: left; cursor: pointer }
  .ag-rowbtn:hover .ag-chev, .ag-rowbtn:focus-visible .ag-chev { opacity: .9 }
  .ag-rowbtn:hover .ag-name { text-decoration: underline; text-underline-offset: 3px }
  .ag-chev { width: 16px; height: 16px; flex: none; opacity: .45 }
  .ag-dot.off { background: var(--hair) }
  .ag-pill.off::before { background: var(--hair) }
  .ag-url { display: inline-flex; align-items: center; gap: 6px; background: none; border: 0; padding: 0; font: inherit; color: inherit; cursor: pointer; text-align: left; max-width: 100% }
  .ag-url > span:first-child { overflow: hidden; text-overflow: ellipsis; white-space: nowrap }
  .ag-copyic { width: 14px; height: 14px; flex: none; opacity: 0; transition: opacity .12s }
  .ag-url:hover .ag-copyic, .ag-url:focus-visible .ag-copyic { opacity: .75 }
  .ag-copyic.on { opacity: .75 }
  .ag-editing { display: inline-block; min-width: 2ch; outline: none; cursor: text; box-shadow: inset 0 -1px 0 var(--accent) }
  .ag-log { max-height: 160px }
  .ag-empty { padding: 10px 0; border-top: 1px solid var(--hair); border-bottom: 1px solid var(--hair) }
  .ag-choices { display: grid; grid-template-columns: 1fr 1fr; gap: 10px }
  .ag-three { grid-template-columns: 1fr 1fr 1fr }
  .ag-field-l > span:first-child { font-size: 13px }
  .ag-choice { display: flex; flex-direction: column; gap: 2px; text-align: left; padding: 12px 14px; border-radius: 10px; border: 1px solid var(--hair); background: transparent; color: inherit; font: inherit; cursor: pointer }
  .ag-choice:hover:not(:disabled) { border-color: rgba(var(--accent-rgb), .6) }
  .ag-choice:disabled { cursor: default; opacity: .55 }
  .ag-choice.on { border-color: var(--accent); background: rgba(var(--accent-rgb), .1); box-shadow: inset 0 0 0 1px var(--accent) }
  .ag-steps { display: flex; flex-direction: column; gap: 12px }
  .ag-step { display: flex; align-items: center; gap: 12px }
  .ag-step > :global(svg) { align-self: flex-start; margin-top: 1px }
  .ag-card a.btn { text-decoration: none }
  .ag-ic { width: 18px; height: 18px; flex: none }
  .ag-spin { animation: ag-turn 1s linear infinite }
  @keyframes ag-turn { to { transform: rotate(360deg) } }
  .ag-note { display: flex; align-items: center; gap: 12px; padding: 12px 14px; border-radius: 10px; border: 1px solid var(--hair) }
  .ag-note > :global(svg) { align-self: flex-start; margin-top: 1px }
  .ag-wait { border-style: dashed }
  .ag-bad { border-color: rgba(var(--neg-rgb), .6) }
  .ag-link { align-self: flex-start; background: none; border: 0; padding: 0; font: inherit; font-size: 12.5px; color: var(--accent); cursor: pointer; text-align: left }
  .ag-link:hover { text-decoration: underline }
  .ag-link.ag-remove { color: var(--neg) }
  .ag-wide { width: 100%; box-sizing: border-box }
  .ag-ol { margin: 0; padding-left: 20px; display: flex; flex-direction: column; gap: 14px }
  .ag-sub { margin-top: 8px }
  .ag-gap { margin-top: 6px }
  .ag-copy { display: flex; align-items: center; gap: 8px }
  .ag-field { flex: 1; min-width: 0; padding: 8px 10px; border-radius: 8px; background: var(--well); border: 1px solid var(--hair); font-size: 12.5px; overflow: hidden; text-overflow: ellipsis; white-space: nowrap }
  .ag-field-l { display: flex; flex-direction: column; gap: 5px }
  .ag-code { display: flex; align-items: baseline; gap: 14px; flex-wrap: wrap }
  .ag-digits { font: 600 30px/1.2 ui-monospace, SFMono-Regular, Menlo, monospace; letter-spacing: .15em }
  .ag-pre { font-size: 11px; max-height: 140px; margin: 0; white-space: pre-wrap }
  .ag-done { display: flex; flex-direction: column; align-items: center; text-align: center; gap: 10px; padding: 24px 0 8px }
  .ag-big { width: 52px; height: 52px }
  @media (max-width: 520px) { .ag-choices, .ag-three { grid-template-columns: 1fr } }
</style>
