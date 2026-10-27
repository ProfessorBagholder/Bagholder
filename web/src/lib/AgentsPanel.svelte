<script lang="ts">
  // The Agents panel (docs/plans/agents-from-anywhere.md §4): the agents' address on or
  // off, the tunnel chosen from every supported one side by side (none chosen for the
  // person), its steps, the sign-ins waiting on this computer, every agent that may read,
  // and what each read. Built as the menu's other modals are (Modals.svelte).
  import { status as statusSlot } from './subs.svelte'
  import { ui, closeModal } from './ui.svelte'
  import { call } from './api'
  import { read } from './reads.svelte'
  import { relTime } from './fmt'
  import type { Provider, Setup, MadeKey, LogLine } from './generated/agents'

  const st = $derived(statusSlot.data?.agents)
  let error = $state('')
  let busy = $state('')
  /** the tunnel being set up, before it is saved; the one saved once it is */
  let picked = $state<Provider | ''>('')
  let name = $state('')
  let funnelPort = $state(443)
  let sendsHost = $state('')
  /** a key just made: shown this once */
  let made = $state<MadeKey | null>(null)
  let keyName = $state('')
  let keyWay = $state<'mcp' | 'api'>('mcp')
  /** an agent's log, while open */
  let logOf = $state('')
  let log = $state<LogLine[]>([])
  let copied = $state('')

  $effect(() => {
    // 443 held by another program here, and nothing saved yet: Funnel serves on 8443
    if (st?.tailscale?.httpsPortTaken && !st.setup) funnelPort = 8443
  })
  $effect(() => {
    // the saved tunnel, once the status says it
    if (st?.setup && !picked) {
      picked = st.setup.provider
      name = st.setup.name
      funnelPort = st.setup.funnelPort ?? 443
      sendsHost = st.setup.sendsHost ?? ''
    }
  })

  const TUNNELS: { p: Provider; label: string; account: string; cost: string; fixed: string; reads: string; doc: string }[] = [
    { p: 'tailscale', label: 'Tailscale Funnel', account: 'Tailscale, free', cost: 'Free', fixed: 'Yes, machine.tailnet.ts.net', reads: 'No one in between: the relay passes encrypted traffic that ends on this computer', doc: 'https://tailscale.com/kb/1223/funnel' },
    { p: 'cloudflare', label: 'Cloudflare Tunnel', account: 'Cloudflare, free, with your own domain on Cloudflare', cost: 'Free', fixed: 'Yes, a name on your domain', reads: 'Cloudflare: encryption ends at its edge', doc: 'https://developers.cloudflare.com/cloudflare-one/connections/connect-networks/' },
    { p: 'ngrok', label: 'ngrok', account: 'ngrok, free', cost: 'Free', fixed: 'Yes, the one dev domain your account is given', reads: 'ngrok: encryption ends at its edge', doc: 'https://ngrok.com/docs/pricing-limits/free-plan-limits' },
    { p: 'zrok', label: 'zrok', account: 'zrok, free', cost: 'Free (2 reserved names)', fixed: 'Yes, once reserved', reads: "zrok's operators: encryption ends at their frontend, which logs the URLs and headers", doc: 'https://netfoundry.io/docs/zrok/concepts/sharing-public' },
    { p: 'own', label: 'My own reverse proxy', account: 'None', cost: '—', fixed: 'Your own domain', reads: 'Only you', doc: '' },
  ]

  const program = (p: Provider) => st?.programs.find((x) => x.provider === p)?.path ?? null
  /** each provider's steps, from nothing: a step already done here is shown done, never chosen for the person; a step Bagholder runs is a button */
  function steps(p: Provider): { what: string; code?: boolean; link?: string; done?: boolean; act?: { label: string; run: () => void } }[] {
    const port = st?.port ?? 0
    switch (p) {
      case 'tailscale': {
        const ts = st?.tailscale
        return [
          { what: 'Install Tailscale', link: 'https://tailscale.com/download', done: !!program(p) },
          { what: 'Sign in with the Tailscale app', link: 'https://tailscale.com/kb/1017/install', done: !!ts?.signedIn },
          { what: 'Let this computer use Funnel: if it is off, starting the tunnel shows a link that turns it on', link: 'https://tailscale.com/kb/1223/funnel#requirements-and-limitations', done: !!ts?.https && !!ts?.funnel },
        ]
      }
      case 'cloudflare':
        return [
          { what: 'Install cloudflared', link: 'https://developers.cloudflare.com/cloudflare-one/connections/connect-networks/downloads/', done: !!program(p) },
          { what: 'Sign in to Cloudflare, and choose the domain for the agents\' address', done: !!st?.cloudflare.signedIn, act: { label: 'Sign in to Cloudflare', run: () => void signIn('cloudflare') } },
          { what: 'Enter a name on that domain below, such as agents.yourdomain.com: starting the tunnel makes it, with its DNS record, the first time' },
        ]
      case 'ngrok':
        return [
          { what: 'Install ngrok', link: 'https://ngrok.com/download', done: !!program(p) },
          { what: 'ngrok config add-authtoken <token from your dashboard>', code: true, link: 'https://dashboard.ngrok.com/get-started/your-authtoken' },
          { what: 'Enter the dev domain your dashboard shows', link: 'https://dashboard.ngrok.com/domains' },
        ]
      case 'zrok':
        return [
          { what: 'Install zrok 2', link: 'https://docs.zrok.io/docs/guides/install/', done: !!program(p) },
          { what: 'zrok2 enable <your account token>', code: true },
          { what: 'zrok2 create name -n public ' + (name || '<a name>'), code: true },
        ]
      case 'own':
        return [{ what: 'Point your proxy at http://127.0.0.1:' + (port || '<port shown once Agents is on>') + ', keeping the Host it was asked for' }]
    }
  }
  const nameLabel = (p: Provider) => ({ tailscale: '', cloudflare: 'The name its DNS route gives', ngrok: 'Your ngrok dev domain', zrok: 'Your reserved name', own: 'Your public name' })[p]

  async function act(what: string, run: () => Promise<{ ok: boolean; error?: string }>) {
    busy = what
    error = ''
    const r = await run()
    busy = ''
    if (!r.ok) error = r.error ?? 'Bagholder did not take it.'
  }
  const switchTo = (on: boolean) => act('switch', () => call('POST /api/agents/switch', { body: { on } }))
  const save = () => {
    if (!picked) return
    const setup: Setup = { provider: picked, name, funnelPort: picked === 'tailscale' ? funnelPort : null, sendsHost: picked === 'own' && sendsHost ? sendsHost : null }
    return act('save', () => call('POST /api/agents/tunnel', { body: setup }))
  }
  const start = () => act('start', () => call('POST /api/agents/tunnel/start'))
  /** the provider's sign-in, run by Bagholder: its link opens from the lines below */
  const signIn = (p: Provider) => act('sign-in', () => call('POST /api/agents/tunnel/sign-in', { body: { provider: p } }))
  /** Tailscale asked again: once chosen, and when the person has signed in or let Funnel on since */
  const look = () => act('look', () => call('POST /api/agents/tunnel/look'))
  function pick(p: Provider) {
    picked = p
    if (p === 'tailscale') void look()
  }
  const stop = () => act('stop', () => call('POST /api/agents/tunnel/stop'))
  const check = () => act('check', () => call('POST /api/agents/check'))
  const answer = (id: string, allow: boolean) => act(id, () => call(allow ? 'POST /api/agents/requests/allow' : 'POST /api/agents/requests/deny', { body: { id } }))
  const cutOff = (id: string) => act(id, () => call('POST /api/agents/cut-off', { body: { id } }))
  async function makeKey(n: string, way: 'mcp' | 'api'): Promise<MadeKey | null> {
    busy = 'key'
    error = ''
    const r = await call('POST /api/agents/keys', { body: { name: n, way } })
    busy = ''
    if (!r.ok) {
      error = r.error
      return null
    }
    made = r
    return r
  }
  /** an agent's log, or (`REFUSED`) the requests no grant or key named, refused */
  const REFUSED = 'refused'
  async function openLog(id: string) {
    if (logOf === id) {
      logOf = ''
      return
    }
    logOf = id
    log = []
    const r = await read('GET /api/agents/log', { query: { agent: id === REFUSED ? '' : id } })
    if (r.ok && logOf === id) log = r.lines
  }
  function copy(text: string, what: string) {
    navigator.clipboard.writeText(text).then(() => (copied = what), () => (error = 'The clipboard is not available here: select the text and copy it.'))
  }

  const address = $derived(st?.address ?? '')
  /** each agent's own steps, with its address, and for a key the key made for it */
  async function copyFor(agent: string) {
    if (!address) return
    if (agent === 'claude') copy(`In Claude.ai: Settings → Connectors → Add custom connector. URL: ${address}/mcp. Then approve the code Bagholder shows on your computer.`, agent)
    else if (agent === 'chatgpt') copy(`In ChatGPT (and Dots through it): add a connector with the MCP server URL ${address}/mcp, sign in, and approve the code Bagholder shows on your computer.`, agent)
    else if (agent === 'muse') {
      const k = made?.name === 'Muse' ? made : await makeKey('Muse', 'api')
      if (k) copy(`In Muse, add a custom connector. API description: ${address}/api/agents/openapi.json. Authorization: Bearer ${k.key}`, agent)
    } else if (agent === 'grok') {
      const k = made?.name === 'Grok Bot' ? made : await makeKey('Grok Bot', 'mcp')
      if (k) copy(`In Grok Bot, add a remote MCP server. URL: ${address}/mcp/key. Header: Authorization: Bearer ${k.key}`, agent)
    }
  }
  /** a line the tunnel wrote, with each https link in it one to open: a sign-in, or the page that lets Funnel on */
  function linked(line: string): { text: string; link: boolean }[] {
    return line.split(/(https:\/\/[^\s"'<>]+)/).filter((t) => t !== '').map((t) => ({ text: t, link: t.startsWith('https://') }))
  }
  /** an agent is given the address once a check found it answers */
  const confirmed = $derived(st?.ready?.state === 'ready' || st?.ready?.state === 'unconfirmed')
  const way = (w: string) => (w === 'sign-in' ? 'Signed in' : w === 'mcp-key' ? 'MCP key' : 'API key')
  const readyWord = $derived(!st?.ready ? '' : st.ready.state === 'ready' ? 'Ready' : st.ready.said)
</script>

<div id="modalDlg" style="position:fixed;inset:0;z-index:20;background:rgba(8,9,16,.55);display:flex;align-items:center;justify-content:center;padding:20px" role="presentation" onclick={(e) => { if (e.target === e.currentTarget) closeModal() }}>
  <div class="card elev-md" style="width:640px;max-width:100%;padding:18px 20px;max-height:90vh;overflow:auto">
    <div style="display:flex;align-items:center;gap:10px;margin-bottom:12px">
      <h5>Agents</h5>
      <button class="btn btn-icon btn-secondary" aria-label="Close" style="margin-left:auto;width:28px;height:28px" onclick={closeModal}>×</button>
    </div>
    <button class="menu-row" style="display:flex;align-items:center;gap:10px;width:100%;background:none;border:0;padding:0;font:inherit;color:inherit;cursor:pointer;text-align:left" disabled={busy === 'switch'} onclick={() => switchTo(!st?.on)}>
      <span style="font-size:12.5px">Let agents elsewhere read your trade history</span><span class="sw" class:on={st?.on} style="margin-left:auto"></span>
    </button>
    <div class="dim" style="font-size:12px;line-height:1.5;margin-top:6px">Read only, through a tunnel you choose. Every agent is approved here, on this computer.{#if st?.port}{' '}Answering on 127.0.0.1:{st.port}.{/if}</div>
    {#if error}<div class="status-err" style="font-size:12px;margin-top:10px">{error}</div>{/if}
    {#if st?.error}<div class="status-err" style="font-size:12px;margin-top:10px">{st.error}</div>{/if}

    {#each st?.waiting ?? [] as w (w.id)}
      <div class="card agent-ask" style="margin-top:14px;padding:12px 14px;border:1px solid var(--accent)">
        <div style="font-size:12.5px"><b>{w.who}</b>{#if w.name}{' '}<span class="muted">({w.checked ? w.name : '“' + w.name + '”, name given by the agent, not checked'})</span>{/if}{' '}is asking to read your trade history. It sends you back to <b>{w.redirectHost}</b>.</div>
        <div style="font-size:12.5px;margin-top:6px">Allow only if you just started connecting this agent, and its page shows code <b style="font-family:ui-monospace,monospace;letter-spacing:.15em">{w.code}</b>.</div>
        <div style="display:flex;gap:8px;justify-content:flex-end;margin-top:10px">
          <button class="btn btn-secondary" disabled={busy === w.id} onclick={() => answer(w.id, false)}>Deny</button>
          <button class="btn btn-primary" disabled={busy === w.id} onclick={() => answer(w.id, true)}>Allow</button>
        </div>
      </div>
    {/each}

    {#if st?.on}
      <div class="lbl" style="margin:18px 0 8px">Choose a tunnel</div>
      <div role="radiogroup" aria-label="Tunnel">
        {#each TUNNELS as t (t.p)}
          <label class="agent-tunnel" style="display:flex;gap:10px;padding:9px 0;border-top:1px solid rgba(var(--ink-rgb),.08);cursor:pointer;font-size:12px;line-height:1.5">
            <input type="radio" name="tunnel" aria-label={t.label} checked={picked === t.p} onchange={() => pick(t.p)} style="margin-top:3px" />
            <span style="min-width:0">
              <span style="font-weight:500">{t.label}</span>{#if t.p === 'tailscale'}{' '}<span class="pill on" style="padding:1px 6px;margin-left:4px">Recommended</span>{/if}{#if t.doc}{' '}<a href={t.doc} target="_blank" rel="noopener" style="margin-left:4px;font-size:11px">About</a>{/if}
              <span class="muted" style="display:block">Account: {t.account} · Cost: {t.cost} · Address stays fixed: {t.fixed}</span>
              <span class="muted" style="display:block">Who can read the traffic: {t.reads}</span>
            </span>
          </label>
        {/each}
      </div>

      {#if picked}
        <ol style="font-size:12px;line-height:1.6;margin:12px 0 0;padding-left:18px">
          {#each steps(picked) as s, i (i)}
            <li>{#if s.done}<span class="muted">Done: </span>{/if}{#if s.link && !s.code}<a href={s.link} target="_blank" rel="noopener">{s.what}</a>{:else if s.link}<code>{s.what}</code>{' '}<a href={s.link} target="_blank" rel="noopener">Where</a>{:else if s.act}{s.what}{#if !s.done}{' '}<button class="btn btn-secondary" style="margin-left:6px" disabled={busy === 'sign-in' || !!st?.tunnel.signingIn} onclick={s.act.run}>{st?.tunnel.signingIn ? 'Waiting for the sign-in…' : s.act.label}</button>{/if}{:else if s.code}<code>{s.what}</code>{:else}{s.what}{/if}</li>
          {/each}
        </ol>
        {#if picked === 'tailscale'}
          <div style="display:flex;gap:8px;align-items:center;margin-top:6px">
            <button class="btn btn-secondary" disabled={busy === 'look'} onclick={look}>{busy === 'look' ? 'Asking Tailscale…' : 'Check again'}</button>
            {#if st?.tailscale?.said}<span class="muted" style="font-size:11.5px">{st.tailscale.said}</span>{/if}
          </div>
        {/if}
        {#if program(picked)}<div class="muted" style="font-size:11px;margin-top:6px">Bagholder will start {program(picked)}.</div>{/if}
        <div style="display:flex;gap:8px;margin-top:10px;align-items:flex-end;flex-wrap:wrap">
          {#if picked === 'tailscale'}
            <div><div class="lbl" style="margin-bottom:5px">Funnel port</div>
              <select class="input" bind:value={funnelPort} aria-label="Funnel port"><option value={443}>443</option><option value={8443}>8443 (443 is in use)</option></select>
            </div>
            {#if st?.tailscale?.httpsPortTaken}<div class="muted" style="font-size:11.5px;align-self:center">Another program on this computer answers on 443, so Funnel serves on 8443.</div>{/if}
            {#if funnelPort === 8443}<div class="status-err" style="font-size:11.5px;align-self:center">Claude.ai did not reach Funnel on 8443 when Bagholder was tested: to connect it, free 443 on this computer or choose another tunnel.</div>{/if}
          {:else}
            <div style="flex:1;min-width:200px"><div class="lbl" style="margin-bottom:5px">{nameLabel(picked)}</div><input class="input" bind:value={name} spellcheck="false" aria-label={nameLabel(picked)} /></div>
          {/if}
          {#if picked === 'own'}
            <div style="flex:1;min-width:200px"><div class="lbl" style="margin-bottom:5px">The Host your proxy sends, if not that</div><input class="input" bind:value={sendsHost} spellcheck="false" aria-label="Host sent" /></div>
          {/if}
          <button class="btn btn-secondary" disabled={busy === 'save'} onclick={save}>Save</button>
        </div>
      {/if}

      {#if st.setup}
        <div style="display:flex;gap:8px;align-items:center;margin-top:14px;flex-wrap:wrap">
          {#if st.setup.provider !== 'own'}
            {#if st.tunnel.running}<button class="btn btn-secondary" disabled={busy === 'stop'} onclick={stop}>{st.tunnel.signingIn ? 'Cancel the sign-in' : 'Stop tunnel'}</button>
            {:else}<button class="btn btn-primary" disabled={busy === 'start'} onclick={start}>Start tunnel</button>{/if}
          {/if}
          <button class="btn btn-secondary" disabled={busy === 'check' || !address} onclick={check}>{busy === 'check' ? 'Checking…' : 'Check the address'}</button>
          {#if readyWord}<span class={st.ready?.state === 'failed' ? 'status-err' : st.ready?.state === 'ready' ? '' : 'muted'} style="font-size:12px">{readyWord}</span>{/if}
        </div>
        {#if address}<div class="muted" style="font-size:11.5px;margin-top:8px">Address: {address}{confirmed ? '' : ', given to agents once the check finds it answers'}</div>{/if}
      {/if}
      {#if st.tunnel.exited}<div class="status-err" style="font-size:12px;margin-top:6px">{st.tunnel.exited}</div>{/if}
      {#if st.tunnel.lines.length}<pre class="scroll" style="font-size:11px;max-height:120px;margin:8px 0 0;white-space:pre-wrap">{#each st.tunnel.lines as line, i (i)}{#each linked(line) as part, j (j)}{#if part.link}<a href={part.text} target="_blank" rel="noopener">{part.text}</a>{:else}{part.text}{/if}{/each}{'\n'}{/each}</pre>{/if}

      {#if address && confirmed}
        <div class="lbl" style="margin:18px 0 8px">Connect an agent</div>
        <div style="display:flex;gap:8px;flex-wrap:wrap">
          {#each [['claude', 'Claude.ai'], ['chatgpt', 'ChatGPT and Dots'], ['muse', 'Muse'], ['grok', 'Grok Bot']] as a (a[0])}
            <button class="btn btn-secondary" disabled={busy === 'key'} onclick={() => copyFor(a[0])}>{copied === a[0] ? 'Copied: ' : 'Copy steps for '}{a[1]}</button>
          {/each}
        </div>
      {/if}

      <div class="lbl" style="margin:18px 0 8px">Make a key</div>
      <div style="display:flex;gap:8px;align-items:center;flex-wrap:wrap">
        <input class="input" style="flex:1;min-width:160px" bind:value={keyName} placeholder="The agent it is for" aria-label="Key name" />
        <select class="input" style="width:auto" bind:value={keyWay} aria-label="Way in"><option value="mcp">MCP (/mcp/key)</option><option value="api">JSON API</option></select>
        <button class="btn btn-secondary" disabled={busy === 'key' || !keyName.trim()} onclick={async () => { if (await makeKey(keyName, keyWay)) keyName = '' }}>Make key</button>
      </div>
      {#if made}
        <div class="card agent-key" style="margin-top:10px;padding:10px 12px">
          <div style="font-size:12px">The key for {made.name}, shown this once:</div>
          <div style="display:flex;gap:8px;align-items:center;margin-top:6px"><code style="font-size:11.5px;word-break:break-all">{made.key}</code><button class="btn btn-secondary" style="margin-left:auto" onclick={() => copy(made!.key, 'key')}>{copied === 'key' ? 'Copied' : 'Copy'}</button></div>
        </div>
      {/if}
    {/if}

    {#if st?.connected.length}
      <div class="lbl" style="margin:18px 0 8px">Connected agents</div>
      {#each st.connected as c (c.id)}
        <div style="padding:8px 0;border-top:1px solid rgba(var(--ink-rgb),.08)">
          <div style="display:flex;gap:10px;align-items:center;font-size:12.5px">
            <button style="background:none;border:0;padding:0;font:inherit;color:inherit;cursor:pointer;text-align:left;min-width:0" onclick={() => openLog(c.id)}><span style="font-weight:500">{c.name}</span> <span class="muted">· {way(c.way)}{c.who ? ' · ' + c.who : ''} · made {relTime(c.madeAt)}{c.lastUsedAt ? ' · last read ' + relTime(c.lastUsedAt) : ' · no reads yet'}</span></button>
            {#if c.cutOffAt}<span class="muted" style="margin-left:auto;font-size:11.5px">Cut off</span>
            {:else}<button class="btn btn-secondary" style="margin-left:auto;color:var(--neg)" disabled={busy === c.id} onclick={() => cutOff(c.id)}>Cut off</button>{/if}
          </div>
          {#if logOf === c.id}
            <div class="scroll" style="max-height:160px;margin-top:6px;font-size:11.5px">
              {#each log as l, i (i)}<div class="muted">{relTime(l.at)} · {l.tool ? l.tool + ' ' + (l.arguments ?? '') : 'refused: ' + l.refusal}</div>{:else}<div class="muted">No reads yet.</div>{/each}
            </div>
          {/if}
        </div>
      {/each}
    {/if}
    {#if st?.on}
      <div style="padding:8px 0;border-top:1px solid rgba(var(--ink-rgb),.08);font-size:12.5px">
        <button style="background:none;border:0;padding:0;font:inherit;color:inherit;cursor:pointer;text-align:left" onclick={() => openLog(REFUSED)}><span class="muted">Requests refused from callers no agent's sign-in or key names</span></button>
        {#if logOf === REFUSED}
          <div class="scroll" style="max-height:160px;margin-top:6px;font-size:11.5px">
            {#each log as l, i (i)}<div class="muted">{relTime(l.at)} · refused: {l.refusal}</div>{:else}<div class="muted">None.</div>{/each}
          </div>
        {/if}
      </div>
    {/if}
  </div>
</div>
