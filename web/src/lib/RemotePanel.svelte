<script lang="ts">
  // Open from anywhere (docs/plans/page-from-anywhere.md): this Bagholder at an address
  // on the person's domain, through a Cloudflare tunnel, behind Cloudflare Access letting
  // in the one email they name. Setting it up is a list of steps, each saying where it
  // stands: done, the one to do now, under way (and where: Cloudflare's tab, Bagholder),
  // failed with what to do, or still to come. Once set up: the tunnel starting and the
  // address checked from outside, then the address and who can open it. Built as the
  // Agents panel is, and styled as it is.
  import { status as statusSlot } from './subs.svelte'
  import { closeModal } from './ui.svelte'
  import { call } from './api'
  import { ICONS } from './icons'

  const st = $derived(statusSlot.data?.remote)
  const agents = $derived(statusSlot.data?.agents)
  const cf = $derived(agents?.cloudflare)
  const computer = $derived(agents?.computer || 'the computer running Bagholder')
  const installed = $derived(!!agents?.programs.find((p) => p.provider === 'cloudflare')?.path)
  const installing = $derived(agents?.installing.find((x) => x.provider === 'cloudflare'))
  const domain = $derived(cf?.domain || '')
  const hostOf = (url: string) => url.replace(/^https:\/\//, '')

  let error = $state('')
  let busy = $state('')
  let copied = $state(false)
  let token = $state('')
  let name = $state('app')
  let naming = $state(false)
  let email = $state('')
  let removing = $state(false)
  /** set up from this panel just now: its last two steps follow in the same list, then that it is ready */
  let fresh = $state(false)
  /** the address just removed: said done, until the panel closes */
  let removed = $state('')

  async function act(what: string, run: () => Promise<{ ok: boolean; error?: string }>): Promise<boolean> {
    busy = what
    error = ''
    const r = await run()
    busy = ''
    if (!r.ok) error = r.error ?? 'Bagholder did not take it.'
    return r.ok
  }
  const install = () => act('install', () => call('POST /api/agents/install', { body: { provider: 'cloudflare' } }))
  /** Cloudflare's tab, opened at the click (a tab opened later is blocked), as the Agents panel opens it */
  let signInWindow: Window | null = null
  /** whether a tab was opened for this sign-in: only then does the step say it waits there */
  let tabOpened = $state(false)
  async function signIn() {
    signInWindow = window.open('about:blank', '_blank')
    tabOpened = !!signInWindow
    try {
      // what the tab shows until Cloudflare's page is in it
      if (signInWindow) {
        signInWindow.document.body.textContent = 'Opening Cloudflare’s sign-in…'
      }
    } catch {
      // a tab Bagholder can't write to still gets Cloudflare's page below
    }
    if (!(await act('cf', () => call('POST /api/agents/cloudflare/sign-in')))) {
      signInWindow?.close()
      signInWindow = null
      tabOpened = false
    }
  }
  $effect(() => {
    // Cloudflare's page, once Bagholder has it, into the tab opened at the click
    const url = cf?.signInUrl
    if (url && signInWindow && !signInWindow.closed) {
      signInWindow.location.href = url
      signInWindow = null
    }
  })
  async function saveToken() {
    if (await act('token', () => call('POST /api/remote/token', { body: { token: token.trim() } }))) token = ''
  }
  const lookForTeam = () => act('team', () => call('POST /api/remote/team'))
  /** Bagholder's GitHub app: its manifest, posted to GitHub in a tab of its own */
  let githubForm: { action: string; manifest: string } | null = null
  let githubTab = $state(false)
  function postManifest(target: string) {
    if (!githubForm) return
    const f = document.createElement('form')
    f.method = 'post'
    f.action = githubForm.action
    f.target = target
    const i = document.createElement('input')
    i.type = 'hidden'
    i.name = 'manifest'
    i.value = githubForm.manifest
    f.appendChild(i)
    document.body.appendChild(f)
    f.submit()
    f.remove()
  }
  async function makeGithub() {
    // the tab opened at the click (one opened later is blocked), GitHub's page posted into it once Bagholder has the manifest
    const tab = window.open('about:blank', 'bagholder-github')
    try {
      if (tab) {
        tab.document.body.textContent = 'Opening GitHub…'
      }
    } catch {
      // a tab Bagholder can't write to still gets GitHub's page
    }
    busy = 'github'
    error = ''
    const r = await call('POST /api/remote/github', { body: { origin: location.origin } })
    busy = ''
    if (!r.ok) {
      tab?.close()
      error = r.error ?? 'Bagholder did not take it.'
      return
    }
    githubForm = { action: r.action, manifest: r.manifest }
    githubTab = !!tab
    postManifest(tab ? 'bagholder-github' : '_blank')
  }
  const addLogin = () => act('login', () => call('POST /api/remote/github/login'))
  async function setUp() {
    // followed in this list from the moment it is asked: the status may say it is set up before the answer comes
    fresh = true
    if (!(await act('setup', () => call('POST /api/remote/set-up', { body: { name: name.trim() || 'app', email: email.trim() } })))) fresh = false
  }
  const switchTo = (on: boolean) => act('switch', () => call('POST /api/remote/switch', { body: { on } }))
  const check = () => act('check', () => call('POST /api/remote/check'))
  /** Bagholder's GitHub sign-in, said once removed: kept for the next set-up, and GitHub's page for it */
  let removedGithub = $state({ name: '', page: '' })
  async function remove() {
    const host = st ? hostOf(st.url) : ''
    const kept = { name: st?.github ?? '', page: st?.githubPage ?? '' }
    removing = false
    if (await act('remove', () => call('POST /api/remote/remove'))) {
      removed = host
      removedGithub = kept
    }
  }
  function copy(text: string) {
    navigator.clipboard.writeText(text).then(
      () => {
        copied = true
        setTimeout(() => (copied = false), 2000)
      },
      () => (error = 'The clipboard is not available here: select the text and copy it.'),
    )
  }
  let asked = false
  $effect(() => {
    // a token kept from before: whether the account has its team now, asked once, quietly
    if (!asked && st?.token && !st.url) {
      asked = true
      void lookForTeam()
    }
  })

  // ---- the steps ----
  type Mark = 'done' | 'now' | 'wait' | 'bad' | 'later'
  const connector = $derived<Mark>(installed ? 'done' : installing && !installing.failed ? 'wait' : installing?.failed ? 'bad' : 'now')
  const signedIn = $derived<Mark>(connector !== 'done' ? 'later' : cf?.signedIn ? 'done' : cf?.signingIn ? (tabOpened || !cf.signInUrl ? 'wait' : 'now') : cf?.said ? 'bad' : 'now')
  const access = $derived<Mark>(signedIn !== 'done' ? 'later' : st?.token && !st.noTeam ? 'done' : st?.noTeam ? 'bad' : 'now')
  /** Bagholder's own GitHub sign-in: waiting in GitHub's tab only while that tab, opened here, has its page */
  const github = $derived<Mark>(access !== 'done' ? 'later' : st?.github ? 'done' : st?.githubSaid ? 'bad' : busy === 'github' || (st?.makingGithub && githubTab) ? 'wait' : 'now')
  const address = $derived<Mark>(st?.url ? 'done' : github !== 'done' ? 'later' : busy === 'setup' ? 'wait' : 'now')

  // ---- set up ----
  const pill = $derived(!st?.url ? '' : !st.on ? 'Stopped' : !st.running ? (st.said ? 'Stopped' : 'Getting ready…') : st.ready === true ? 'Ready' : st.ready === false ? 'Can’t be reached' : 'Getting ready…')
  const tunnelMark = $derived<Mark>(st?.running ? 'done' : st?.said ? 'bad' : 'wait')
  const checkMark = $derived<Mark>(!st?.running ? 'later' : st.ready === true ? 'done' : st.ready === false ? 'bad' : 'wait')
</script>

{#snippet mark(m: Mark, n: number)}
  {#if m === 'done'}
    <svg class="ag-ic" style="color:var(--pos)" viewBox="0 0 20 20" aria-hidden="true"><circle cx="10" cy="10" r="8.5" fill="none" stroke="currentColor" stroke-width="1.5" /><path d="M6.2 10.4l2.5 2.5 5.1-5.3" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" /></svg>
  {:else if m === 'wait'}
    <svg class="ag-ic ag-spin" viewBox="0 0 20 20" aria-hidden="true"><circle cx="10" cy="10" r="8" fill="none" stroke="var(--hair)" stroke-width="2.2" /><path d="M10 2a8 8 0 0 1 8 8" fill="none" stroke="var(--accent)" stroke-width="2.2" stroke-linecap="round" /></svg>
  {:else if m === 'bad'}
    <svg class="ag-ic" style="color:var(--neg)" viewBox="0 0 20 20" aria-hidden="true"><circle cx="10" cy="10" r="8.5" fill="none" stroke="currentColor" stroke-width="1.5" /><path d="M10 5.8v5.2" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" /><circle cx="10" cy="14" r="1.1" fill="currentColor" /></svg>
  {:else}
    <span class="ag-ic ag-num" class:now={m === 'now'} aria-hidden="true">{n}</span>
  {/if}
{/snippet}

{#snippet comingUp(first: number)}
  <!-- the tunnel starting, then the address asked from outside until it asks for the sign-in: Bagholder tries on its own, saying each try -->
  {@const host = st?.url ? hostOf(st.url) : 'the address'}
  <div class="ag-step" class:ag-later={!st?.url}>
    {@render mark(st?.url ? tunnelMark : 'later', first)}
    <div class="ag-grow">
      <div>{!st?.url ? 'Start the tunnel' : tunnelMark === 'done' ? 'The tunnel is running' : tunnelMark === 'bad' ? 'The tunnel stopped' : 'Starting the tunnel…'}</div>
      {#if st?.url && tunnelMark === 'bad'}<div class="muted ag-small">{st.said}</div>{/if}
    </div>
    {#if st?.url && tunnelMark === 'bad'}<button class="btn btn-secondary" disabled={busy === 'switch'} onclick={() => switchTo(true)}>Start it again</button>{/if}
  </div>
  <div class="ag-step" class:ag-later={!st?.url || checkMark === 'later'}>
    {@render mark(st?.url ? checkMark : 'later', first + 1)}
    <div class="ag-grow">
      <div>{checkMark === 'done' ? `${host} asks for your sign-in` : checkMark === 'bad' ? `${host} didn’t answer` : st?.url && checkMark === 'wait' ? `Checking that ${host} asks for your sign-in…` : 'Check that the address asks for your sign-in'}</div>
      {#if st?.url && checkMark === 'wait'}<div class="muted ag-small">{st.trying ? `Not yet: ${st.trying} Bagholder tries again every few seconds.` : 'Bagholder checks every few seconds. It can take a minute or two the first time.'}</div>{/if}
      {#if checkMark === 'bad'}<div class="muted ag-small">{st?.said} Bagholder tried for ten minutes.</div>{/if}
    </div>
    {#if checkMark === 'bad'}<button class="btn btn-secondary" disabled={busy === 'check'} onclick={check}>Check again</button>{/if}
  </div>
{/snippet}

<!-- back from Cloudflare's tab: whether the account has its team now, asked quietly -->
<svelte:window onfocus={() => { if (st?.noTeam) void lookForTeam() }} />

<div id="modalDlg" class="ag-back" role="presentation" onclick={(e) => { if (e.target === e.currentTarget) closeModal() }}>
  <div class="card elev-md ag-card" role="dialog" aria-modal="true" aria-labelledby="rmTitle">
    <div class="ag-head">
      <h5 id="rmTitle" class="ag-manage-head">Open from anywhere</h5>
      {#if pill}<span class="ag-pill" class:ok={pill === 'Ready'} class:bad={pill === 'Can’t be reached'} class:off={pill === 'Stopped'}>{pill}</span>{/if}
      <button class="tk-x" aria-label="Close" onclick={closeModal}><svg width="14" height="14" viewBox="0 0 256 256" fill="currentColor"><path d={ICONS.x} /></svg></button>
    </div>
    {#if error}<div class="status-err ag-small">{error}</div>{/if}

    {#if !st || !agents}
      <!-- nothing said before the status has come: no step is known yet -->
    {:else if removed}
      <div class="ag-done">
        <svg class="ag-big" style="color:var(--pos)" viewBox="0 0 20 20" aria-hidden="true"><circle cx="10" cy="10" r="9" fill="none" stroke="currentColor" stroke-width="1" /><path d="M6 10.5l2.5 2.5L14 7.5" fill="none" stroke="currentColor" stroke-width="1.4" stroke-linecap="round" stroke-linejoin="round" /></svg>
        <div class="ag-q">{removed} is removed</div>
        <p class="ag-p">Its tunnel, its address and its Access application are deleted at Cloudflare, and Bagholder no longer keeps the token you made for it.</p>
        {#if removedGithub.name}<p class="ag-p ag-small muted">Bagholder’s GitHub sign-in, {removedGithub.name}, stays, so setting this up again skips that step. If you won’t, delete it at Cloudflare (Zero Trust → Integrations → Identity providers) and on GitHub.</p>{/if}
        <p class="ag-p ag-small muted">{cf?.signedIn ? 'The token, named Bagholder, stays in your Cloudflare account until you delete it there.' : 'The token, named Bagholder, and the ones Cloudflare made each time you signed in, named for Cloudflare Tunnel, stay in your Cloudflare account until you delete them there.'} Bagholder can’t: deleting a token takes the right to make tokens of any kind.</p>
      </div>
      <div class="ag-foot">
        <a class="btn btn-secondary" href={st.tokensPage} target="_blank" rel="noopener">Open Cloudflare’s API tokens</a>
        {#if removedGithub.page}<a class="btn btn-secondary" href={removedGithub.page} target="_blank" rel="noopener">Open Bagholder’s GitHub app</a>{/if}
        <button class="btn btn-primary ag-right" onclick={closeModal}>Done</button>
      </div>
    {:else if busy === 'remove'}
      <div class="ag-box ag-steps">
        <div class="ag-step">
          {@render mark('wait', 0)}
          <div class="ag-grow">
            <div>Removing {hostOf(st.url)} at Cloudflare…</div>
            <div class="muted ag-small">Its tunnel, its address and its Access application.</div>
          </div>
        </div>
      </div>
    {:else if st.url && fresh && st.ready === true}
      <div class="ag-done">
        <svg class="ag-big" style="color:var(--pos)" viewBox="0 0 20 20" aria-hidden="true"><circle cx="10" cy="10" r="9" fill="none" stroke="currentColor" stroke-width="1" /><path d="M6 10.5l2.5 2.5L14 7.5" fill="none" stroke="currentColor" stroke-width="1.4" stroke-linecap="round" stroke-linejoin="round" /></svg>
        <div class="ag-q">{hostOf(st.url)} is ready</div>
        <p class="ag-p">Open it on your phone or any computer. Cloudflare asks for your sign-in first, and lets in {st.email} alone.</p>
      </div>
      <div class="ag-foot">
        <a class="btn btn-secondary" href={st.url} target="_blank" rel="noopener">Open {hostOf(st.url)}</a>
        <button class="btn btn-primary ag-right" onclick={() => { fresh = false; closeModal() }}>Done</button>
      </div>
    {:else if st.url && !fresh}
      {#if st.on && pill !== 'Ready'}
        <div class="ag-box ag-steps">
          {@render comingUp(1)}
        </div>
      {/if}
      <dl class="ag-facts">
        <dt>Address</dt>
        <dd>
          <span class="ag-row">
            <button class="ag-url" aria-label="Copy the address" onclick={() => copy(st.url)}><span class="ag-mono">{hostOf(st.url)}</span><svg class="ag-copyic" class:on={copied} viewBox="0 0 256 256" fill="currentColor" aria-hidden="true"><path d={copied ? ICONS.check : ICONS.copy} /></svg></button>
            {#if pill === 'Ready'}<a class="ag-link" href={st.url} target="_blank" rel="noopener">Open</a>{/if}
          </span>
        </dd>
        <dt>Who can open it</dt>
        <dd>{st.email}</dd>
      </dl>
      <div class="muted ag-small">The address opens while <b>{computer}</b> is on and awake.</div>
      {#if removing}
        <div class="ag-note ag-bad">
          <div class="ag-grow">Remove {hostOf(st.url)}? Its tunnel, its address and its Access application are deleted at Cloudflare.</div>
          <button class="btn btn-secondary" onclick={() => (removing = false)}>Cancel</button>
          <button class="btn btn-primary" disabled={busy === 'remove'} onclick={remove}>Remove</button>
        </div>
      {/if}
      <div class="ag-foot">
        <button class="ag-link ag-remove" onclick={() => (removing = true)}>Remove</button>
        {#if st.on}<button class="btn btn-secondary ag-right" disabled={busy === 'switch'} onclick={() => switchTo(false)}>Stop</button>
        {:else}<button class="btn btn-primary ag-right" disabled={busy === 'switch'} onclick={() => switchTo(true)}>Start</button>{/if}
      </div>
    {:else}
      <p class="ag-p">Open this Bagholder from your phone or any computer, at an address on your domain. Cloudflare asks for your sign-in first and lets in only the email you name.</p>
      <div class="ag-box ag-steps">
        <div class="ag-step">
          {@render mark(connector, 1)}
          <div class="ag-grow">
            {#if connector === 'done'}<div>Cloudflare’s connector is installed on <b>{computer}</b></div>
            {:else if connector === 'wait'}<div>Installing Cloudflare’s connector on <b>{computer}</b>…</div>
            {:else}
              <div>{connector === 'bad' ? 'Cloudflare’s connector couldn’t be installed on' : 'Install Cloudflare’s connector on'} <b>{computer}</b></div>
              {#if connector === 'bad'}<div class="status-err ag-small">{installing?.failed}</div>{:else}<div class="muted ag-small">A small program from Cloudflare, called cloudflared.</div>{/if}
            {/if}
          </div>
          {#if connector === 'now' || connector === 'bad'}<button class="btn btn-secondary" disabled={busy === 'install'} onclick={install}>{connector === 'bad' ? 'Try again' : 'Install'}</button>{/if}
        </div>

        <div class="ag-step" class:ag-later={signedIn === 'later'}>
          {@render mark(signedIn, 2)}
          <div class="ag-grow">
            {#if signedIn === 'done'}
              <div>Signed in to Cloudflare</div>
              {#if domain}<div class="muted ag-small">Your domain: {domain}</div>{/if}
            {:else if signedIn === 'wait' && !cf?.signInUrl}<div>Starting Cloudflare’s sign-in…</div>
            {:else if signedIn === 'wait'}
              <div>Waiting for you in Cloudflare’s tab</div>
              <div class="muted ag-small">Choose the domain for your address, then press Authorize.</div>
            {:else if cf?.signingIn}
              <div>Cloudflare’s sign-in is ready</div>
              <div class="muted ag-small">Open it, choose the domain for your address, then press Authorize.</div>
            {:else}
              <div>Sign in to Cloudflare</div>
              {#if signedIn === 'bad'}<div class="status-err ag-small">{cf?.said}</div>{:else if signedIn === 'now'}<div class="muted ag-small">You choose the domain for your address there.</div>{/if}
            {/if}
          </div>
          {#if cf?.signingIn && cf.signInUrl && signedIn !== 'done'}<a class="btn btn-secondary" href={cf.signInUrl} target="_blank" rel="noopener" onclick={() => (tabOpened = true)}>{tabOpened ? 'Open it again' : 'Open Cloudflare’s sign-in'}</a>
          {:else if signedIn === 'now' || signedIn === 'bad'}<button class="btn btn-secondary" disabled={busy === 'cf'} onclick={signIn}>{signedIn === 'bad' ? 'Try again' : 'Sign in'}</button>{/if}
        </div>

        <div class="ag-step" class:ag-later={access === 'later'}>
          {@render mark(access, 3)}
          <div class="ag-grow">
            {#if access === 'done'}<div>Bagholder can set up Cloudflare Access</div>
            {:else if access === 'bad'}
              <div>Make your Zero Trust team</div>
              <div class="muted ag-small">Cloudflare Access needs one. On Cloudflare, pick a team name and the Free plan, then come back here.</div>
            {:else}
              <div>Let Bagholder set up Cloudflare Access</div>
              {#if access === 'now'}<div class="muted ag-small">On Cloudflare’s page, press Continue to summary, then Create Token. Copy the token and paste it here.</div>{/if}
            {/if}
          </div>
          {#if access === 'now'}<a class="btn btn-secondary" href={st.tokenPage} target="_blank" rel="noopener">Open Cloudflare’s page</a>
          {:else if access === 'bad'}<a class="btn btn-secondary" href={st.teamPage} target="_blank" rel="noopener">Open Zero Trust</a><button class="btn btn-secondary" disabled={busy === 'team'} onclick={lookForTeam}>{busy === 'team' ? 'Checking…' : 'Check again'}</button>{/if}
        </div>
        {#if access === 'now'}
          <div class="ag-copy ag-indent">
            <input class="input ag-grow" bind:value={token} spellcheck="false" autocomplete="off" aria-label="Cloudflare token" placeholder="Paste the token" />
            <button class="btn btn-secondary" disabled={!token.trim() || busy === 'token'} onclick={saveToken}>{busy === 'token' ? 'Checking…' : 'Save'}</button>
          </div>
        {/if}

        <div class="ag-step" class:ag-later={github === 'later'}>
          {@render mark(github, 4)}
          <div class="ag-grow">
            {#if github === 'done'}
              <div>Bagholder’s sign-in on GitHub is made</div>
              <div class="muted ag-small">GitHub shows it as {st.github}</div>
            {:else if github === 'wait' && busy === 'github'}<div>Opening GitHub…</div>
            {:else if github === 'wait'}
              <div>Waiting for you in GitHub’s tab</div>
              <div class="muted ag-small">Press Create GitHub App there.</div>
            {:else if github === 'bad'}
              <div>Cloudflare didn’t add Bagholder’s sign-in</div>
              <div class="status-err ag-small">{st.githubSaid}</div>
            {:else}
              <div>Make Bagholder’s sign-in on GitHub</div>
              {#if github === 'now'}<div class="muted ag-small">GitHub opens a private app named for Bagholder, which can read your email and nothing else. Press Create GitHub App there.</div>{/if}
            {/if}
          </div>
          {#if github === 'now'}<button class="btn btn-secondary" disabled={busy === 'github'} onclick={makeGithub}>Make it on GitHub</button>
          {:else if github === 'wait' && busy !== 'github'}<button class="btn btn-secondary" onclick={() => postManifest('_blank')}>Open it again</button>
          {:else if github === 'bad'}<button class="btn btn-secondary" disabled={busy === 'login'} onclick={addLogin}>{busy === 'login' ? 'Trying…' : 'Try again'}</button>{/if}
        </div>

        <div class="ag-step" class:ag-later={address === 'later'}>
          {@render mark(address, 5)}
          <div class="ag-grow">
            {#if address === 'done'}<div>Your address is <b class="ag-mono">{hostOf(st.url)}</b>, for {st.email}</div>
            {:else if address === 'wait'}<div>Setting it up at Cloudflare…</div>
            {:else}<div>Choose your address and who can open it</div>{/if}
          </div>
        </div>
        {#if address === 'now'}
          <div class="ag-indent ag-fields">
            {#if naming}
              <label class="ag-field-l">
                <span>Name for the address</span>
                <div class="ag-copy"><input class="input ag-grow" bind:value={name} spellcheck="false" aria-label="Name for the address" />{#if domain}<span class="muted">.{domain}</span>{/if}</div>
              </label>
            {:else}
              {#if domain}<div>Your address will be <b class="ag-mono">{(name.trim() || 'app') + '.' + domain}</b></div>
              {:else}<div>Your address will be <b class="ag-mono">{name.trim() || 'app'}</b> on the domain you chose at Cloudflare</div>{/if}
              <button class="ag-link" onclick={() => (naming = true)}>Use a different name</button>
            {/if}
            <label class="ag-field-l">
              <span>Who can open it</span>
              <input class="input" type="email" bind:value={email} spellcheck="false" autocomplete="email" placeholder="you@example.com" />
              <span class="muted ag-small">Your GitHub account’s primary email: <a class="ag-inline" href="https://github.com/settings/emails" target="_blank" rel="noopener">GitHub’s email settings</a> show it. GitHub tells Cloudflare that email when you sign in.</span>
            </label>
          </div>
        {/if}
        {@render comingUp(6)}
      </div>
      {#if address === 'now' || address === 'wait'}
        <div class="ag-foot"><button class="btn btn-primary ag-right" disabled={!email.trim() || busy === 'setup'} onclick={setUp}>Continue</button></div>
      {/if}
    {/if}
  </div>
</div>

<style>
  .ag-back { position: fixed; inset: 0; z-index: 20; background: rgba(8, 9, 16, .55); display: flex; align-items: center; justify-content: center; padding: 20px }
  .ag-card { width: 640px; max-width: 100%; padding: 18px 20px; max-height: 90vh; overflow: auto; display: flex; flex-direction: column; gap: 14px; font-size: 13px; line-height: 1.5 }
  .ag-head { display: flex; align-items: center; gap: 10px }
  .ag-manage-head { margin: 0; min-width: 0 }
  .ag-small { font-size: 12px }
  .ag-mono { font-family: ui-monospace, SFMono-Regular, Menlo, monospace }
  .ag-p { margin: 0; color: var(--ink2, inherit) }
  .ag-grow { flex: 1; min-width: 0 }
  .ag-row { display: flex; align-items: center; gap: 10px }
  .ag-right { margin-left: auto }
  .ag-foot { display: flex; align-items: center; gap: 10px; margin-top: 4px }
  .ag-box { border: 1px solid var(--hair); border-radius: 10px; padding: 12px 14px }
  .ag-remove { color: var(--neg) }
  .ag-facts { display: grid; grid-template-columns: 120px 1fr; gap: 10px 16px; margin: 4px 0 6px }
  .ag-facts dt { color: var(--ink2, inherit); opacity: .7; font-size: 12.5px }
  .ag-facts dd { margin: 0; display: flex; flex-direction: column; align-items: flex-start; gap: 2px; min-width: 0 }
  .ag-pill + .tk-x { margin-left: 0 }
  .ag-pill { margin-left: auto; display: inline-flex; align-items: center; gap: 6px; font-size: 12px; padding: 2px 10px; border-radius: 999px; border: 1px solid var(--hair) }
  .ag-pill::before { content: ''; width: 7px; height: 7px; border-radius: 50%; background: var(--draft) }
  .ag-pill.ok::before { background: var(--pos) }
  .ag-pill.bad::before { background: var(--neg) }
  .ag-pill.off::before { background: var(--hair) }
  .ag-url { display: inline-flex; align-items: center; gap: 6px; background: none; border: 0; padding: 0; font: inherit; color: inherit; cursor: pointer; text-align: left; max-width: 100% }
  .ag-url > span:first-child { overflow: hidden; text-overflow: ellipsis; white-space: nowrap }
  .ag-copyic { width: 14px; height: 14px; flex: none; opacity: 0; transition: opacity .12s }
  .ag-url:hover .ag-copyic, .ag-url:focus-visible .ag-copyic { opacity: .75 }
  .ag-copyic.on { opacity: .75 }
  .ag-field-l > span:first-child { font-size: 13px }
  .ag-steps { display: flex; flex-direction: column; gap: 12px }
  .ag-step { display: flex; align-items: center; gap: 12px }
  .ag-step > :global(svg), .ag-step > .ag-num { align-self: flex-start; margin-top: 1px }
  .ag-later { opacity: .5 }
  .ag-indent { margin-left: 30px }
  .ag-fields { display: flex; flex-direction: column; gap: 10px }
  .ag-card a.btn { text-decoration: none }
  .ag-ic { width: 18px; height: 18px; flex: none }
  .ag-num { display: inline-flex; align-items: center; justify-content: center; box-sizing: border-box; border-radius: 50%; border: 1.5px solid var(--hair); font-size: 11px; font-weight: 600 }
  .ag-num.now { border-color: var(--accent); color: var(--accent) }
  .ag-spin { animation: ag-turn 1s linear infinite }
  @keyframes ag-turn { to { transform: rotate(360deg) } }
  .ag-note { display: flex; align-items: center; gap: 12px; padding: 12px 14px; border-radius: 10px; border: 1px solid var(--hair) }
  .ag-bad { border-color: rgba(var(--neg-rgb), .6) }
  .ag-link { align-self: flex-start; background: none; border: 0; padding: 0; font: inherit; font-size: 12.5px; color: var(--accent); cursor: pointer; text-align: left; text-decoration: none }
  .ag-link:hover { text-decoration: underline }
  .ag-link.ag-remove { color: var(--neg) }
  .ag-done { display: flex; flex-direction: column; align-items: center; text-align: center; gap: 10px; padding: 24px 0 8px }
  .ag-big { width: 52px; height: 52px }
  .ag-q { font-size: 14px; font-weight: 600 }
  .ag-copy { display: flex; align-items: center; gap: 8px }
  .ag-inline { color: var(--accent); text-decoration: none }
  .ag-inline:hover { text-decoration: underline }
  .ag-field-l { display: flex; flex-direction: column; gap: 5px }
</style>
