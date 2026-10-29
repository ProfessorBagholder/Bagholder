import { afterEach, describe, expect, it, vi } from 'vitest'
import { fireEvent, render } from '@testing-library/svelte'
import { flushSync } from 'svelte'
import RemotePanel from './RemotePanel.svelte'
import { status } from './subs.svelte'
import type { Status } from './model'
import type { AgentsStatus, RemoteStatus } from './generated/agents'

// Open from anywhere (docs/plans/page-from-anywhere.md), rendered from the status the
// server sends: a test run has no Cloudflare account, tunnel or Access application.

function agents(over: Partial<AgentsStatus> = {}): AgentsStatus {
  return {
    on: false, port: null, addresses: [],
    programs: [{ provider: 'cloudflare', path: '/usr/local/bin/cloudflared' }],
    computer: 'box', os: 'linux', installing: [], tailscale: null,
    cloudflare: { signedIn: true, signingIn: false, signInUrl: null, said: '', domain: 'example.com' },
    waiting: [], allowed: [], refused: [], codesPaused: false, error: '',
    ...over,
  }
}
function remote(over: Partial<RemoteStatus> = {}): RemoteStatus {
  return { url: '', email: '', token: true, tokenPage: 'https://dash.cloudflare.com/profile/api-tokens?x', trying: '', github: 'Bagholder at example.com', makingGithub: false, githubSaid: '', githubPage: 'https://github.com/settings/apps/bagholder-at-example-com/advanced', noTeam: false, teamPage: 'https://one.dash.cloudflare.com/', tokensPage: 'https://dash.cloudflare.com/profile/api-tokens', on: false, running: false, ready: null, said: '', ...over }
}
const set = (a: AgentsStatus, r: RemoteStatus) => {
  status.data = { agents: a, remote: r } as unknown as Status
  flushSync()
}
function server() {
  const posted: { path: string; body: Record<string, unknown> }[] = []
  vi.stubGlobal('fetch', vi.fn(async (url: string | URL | Request, init?: RequestInit) => {
    if (init?.method === 'POST') posted.push({ path: String(url), body: JSON.parse(String(init?.body ?? '{}')) })
    return new Response(JSON.stringify({ ok: true }))
  }))
  return posted
}

afterEach(() => {
  status.data = null as unknown as Status
  vi.unstubAllGlobals()
})

describe('Open from anywhere is set up in order', () => {
  it('every step is listed; only the one to do now has its button, and later ones are dimmed', () => {
    server()
    set(agents({ programs: [{ provider: 'cloudflare', path: null }] }), remote({ token: false }))
    const { container, queryByRole } = render(RemotePanel)
    const steps = () => [...container.querySelectorAll('.ag-step')].map((s) => ({ text: s.textContent?.trim() ?? '', later: s.classList.contains('ag-later') }))
    expect(steps().map((s) => s.later)).toEqual([false, true, true, true, true, true, true])
    expect(steps()[0].text).toContain('Install Cloudflare’s connector on box')
    expect(queryByRole('button', { name: 'Install' })).not.toBeNull()
    expect(queryByRole('button', { name: 'Sign in' })).toBeNull()
    set(agents({ cloudflare: { signedIn: false, signingIn: false, signInUrl: null, said: '', domain: null } }), remote({ token: false }))
    expect(steps()[0].text).toContain('Cloudflare’s connector is installed on box')
    expect(steps().map((s) => s.later)).toEqual([false, false, true, true, true, true, true])
    expect(queryByRole('button', { name: 'Sign in' })).not.toBeNull()
    set(agents(), remote({ token: false }))
    expect(steps()[1].text).toContain('Signed in to Cloudflare')
    expect(steps()[1].text).toContain('Your domain: example.com')
    expect((queryByRole('link', { name: 'Open Cloudflare’s page' }) as HTMLAnchorElement).href).toBe('https://dash.cloudflare.com/profile/api-tokens?x')
  })

  it('a sign-in under way with no tab opened here says it is ready to open, never that it waits in a tab', () => {
    server()
    // the page reloaded while Cloudflare's sign-in ran: no tab was opened from this page
    set(agents({ cloudflare: { signedIn: false, signingIn: true, signInUrl: 'https://dash.cloudflare.com/argotunnel?x', said: '', domain: null } }), remote({ token: false }))
    const { container, getByRole } = render(RemotePanel)
    expect(container.textContent).toContain('Cloudflare’s sign-in is ready')
    expect(container.textContent).not.toContain('Waiting for you in Cloudflare’s tab')
    expect((getByRole('link', { name: 'Open Cloudflare’s sign-in' }) as HTMLAnchorElement).href).toBe('https://dash.cloudflare.com/argotunnel?x')
  })

  it('an account with no Zero Trust team gets a step of its own, with Cloudflare’s page for it', async () => {
    server()
    set(agents(), remote({ token: true, noTeam: true }))
    const { container, getByRole } = render(RemotePanel)
    expect(container.textContent).toContain('Make your Zero Trust team')
    expect((getByRole('link', { name: 'Open Zero Trust' }) as HTMLAnchorElement).href).toBe('https://one.dash.cloudflare.com/')
    // it asks Cloudflare again as it opens, saying so, then offers Check again
    await vi.waitFor(() => expect(getByRole('button', { name: 'Check again' })).not.toBeNull())
  })

  it('Sign in opens Cloudflare’s tab at the click, and Cloudflare’s page goes into it once Bagholder has it', async () => {
    const posted = server()
    const tab = { closed: false, close: vi.fn(), location: { href: 'about:blank' } }
    const open = vi.fn(() => tab)
    vi.stubGlobal('open', open)
    const out = { signedIn: false, signingIn: false, signInUrl: null, said: '', domain: null }
    set(agents({ cloudflare: out }), remote({ token: false }))
    const { getByRole } = render(RemotePanel)
    await fireEvent.click(getByRole('button', { name: 'Sign in' }))
    expect(open).toHaveBeenCalledWith('about:blank', '_blank')
    expect(posted.map((p) => p.path)).toEqual(['/api/agents/cloudflare/sign-in'])
    set(agents({ cloudflare: { ...out, signingIn: true, signInUrl: 'https://dash.cloudflare.com/argotunnel?x' } }), remote({ token: false }))
    expect(tab.location.href).toBe('https://dash.cloudflare.com/argotunnel?x')
  })

  it('the address is on the domain signed in with, and Continue sends the name and the email', async () => {
    const posted = server()
    set(agents(), remote())
    const { container, getByRole, getByPlaceholderText } = render(RemotePanel)
    expect(container.textContent).toContain('Your address will be app.example.com')
    const go = getByRole('button', { name: 'Continue' }) as HTMLButtonElement
    expect(go.disabled).toBe(true)
    await fireEvent.input(getByPlaceholderText('you@example.com'), { target: { value: 'me@example.com' } })
    await fireEvent.click(go)
    expect(posted.filter((p) => p.path === '/api/remote/set-up')).toEqual([{ path: '/api/remote/set-up', body: { name: 'app', email: 'me@example.com' } }])
  })
})

describe('Bagholder’s own sign-in on GitHub', () => {
  it('opens GitHub’s tab at the click and posts Bagholder’s manifest into it; once Cloudflare holds it, it is done', async () => {
    const posted: { path: string; body: Record<string, unknown> }[] = []
    vi.stubGlobal('fetch', vi.fn(async (url: string | URL | Request, init?: RequestInit) => {
      if (init?.method === 'POST') posted.push({ path: String(url), body: JSON.parse(String(init?.body ?? '{}')) })
      if (String(url).endsWith('/api/remote/github')) return new Response(JSON.stringify({ ok: true, action: 'https://github.com/settings/apps/new?state=s1', manifest: '{"name":"Bagholder at example.com"}' }))
      return new Response(JSON.stringify({ ok: true }))
    }))
    const tab = { closed: false, close: vi.fn(), document: { title: '', body: { textContent: '' } } }
    const open = vi.fn(() => tab)
    vi.stubGlobal('open', open)
    const submitted: { action: string; target: string; manifest: string }[] = []
    const submit = vi.spyOn(HTMLFormElement.prototype, 'submit').mockImplementation(function (this: HTMLFormElement) {
      submitted.push({ action: this.action, target: this.target, manifest: (this.elements.namedItem('manifest') as HTMLInputElement).value })
    })
    set(agents(), remote({ github: '', githubPage: '' }))
    const { container, getByRole } = render(RemotePanel)
    const step = () => container.querySelectorAll('.ag-step')[3].textContent ?? ''
    expect(step()).toContain('Make Bagholder’s sign-in on GitHub')
    await fireEvent.click(getByRole('button', { name: 'Make it on GitHub' }))
    expect(open).toHaveBeenCalledWith('about:blank', 'bagholder-github')
    await vi.waitFor(() => expect(submitted).toHaveLength(1))
    expect(submitted[0]).toEqual({ action: 'https://github.com/settings/apps/new?state=s1', target: 'bagholder-github', manifest: '{"name":"Bagholder at example.com"}' })
    expect(posted.find((p) => p.path === '/api/remote/github')?.body).toEqual({ origin: location.origin })
    set(agents(), remote({ github: '', githubPage: '', makingGithub: true }))
    expect(step()).toContain('Waiting for you in GitHub’s tab')
    set(agents(), remote())
    expect(step()).toContain('Bagholder’s sign-in on GitHub is made')
    expect(step()).toContain('GitHub shows it as Bagholder at example.com')
    submit.mockRestore()
  })

  it('a sign-in Cloudflare refused says why, with Try again', () => {
    server()
    set(agents(), remote({ github: '', githubSaid: 'Cloudflare didn’t add Bagholder’s GitHub sign-in: Authentication error.' }))
    const { container, getByRole } = render(RemotePanel)
    expect(container.textContent).toContain('Cloudflare didn’t add Bagholder’s sign-in')
    expect(getByRole('button', { name: 'Try again' })).not.toBeNull()
  })
})

describe('Continue, then the same list to the end', () => {
  it('the tunnel and the check follow as steps 5 and 6, each try said, never a Check again while it tries; then ready', async () => {
    server()
    set(agents(), remote())
    const { container, getByRole, getByPlaceholderText, queryByRole } = render(RemotePanel)
    const steps = () => [...container.querySelectorAll('.ag-step')].map((s) => s.textContent?.trim() ?? '')
    expect(steps()).toHaveLength(7)
    expect(steps()[5]).toContain('Start the tunnel')
    await fireEvent.input(getByPlaceholderText('you@example.com'), { target: { value: 'me@example.com' } })
    await fireEvent.click(getByRole('button', { name: 'Continue' }))
    const up = { url: 'https://app.example.com', email: 'me@example.com', on: true }
    set(agents(), remote({ ...up, running: false }))
    expect(steps()).toHaveLength(7)
    expect(steps()[3]).toContain('Bagholder’s sign-in on GitHub is made')
    expect(steps()[4]).toContain('Your address is app.example.com, for me@example.com')
    expect(steps()[5]).toContain('Starting the tunnel…')
    set(agents(), remote({ ...up, running: true, trying: 'app.example.com isn’t in public DNS yet.' }))
    expect(steps()[5]).toContain('The tunnel is running')
    expect(steps()[6]).toContain('Checking that app.example.com asks for your sign-in…')
    expect(steps()[6]).toContain('Not yet: app.example.com isn’t in public DNS yet. Bagholder tries again every few seconds.')
    expect(queryByRole('button', { name: 'Check again' })).toBeNull()
    set(agents(), remote({ ...up, running: true, ready: true }))
    expect(container.textContent).toContain('app.example.com is ready')
    expect((getByRole('link', { name: 'Open app.example.com' }) as HTMLAnchorElement).href).toBe('https://app.example.com/')
  })
})

describe('Open from anywhere, set up', () => {
  it('says the address, who can open it, and its state', () => {
    server()
    set(agents(), remote({ url: 'https://app.example.com', email: 'me@example.com', on: true, running: true, ready: true }))
    const { container } = render(RemotePanel)
    expect(container.querySelector('.ag-pill')?.textContent).toBe('Ready')
    expect(container.textContent).toContain('app.example.com')
    expect(container.textContent).toContain('me@example.com')
    set(agents(), remote({ url: 'https://app.example.com', email: 'me@example.com', on: true, running: true, ready: false, said: 'app.example.com can’t be reached yet.' }))
    expect(container.querySelector('.ag-pill')?.textContent).toBe('Can’t be reached')
    expect(container.textContent).toContain('app.example.com can’t be reached yet.')
  })

  it('Remove asks first, then removes, then says what went', async () => {
    const posted = server()
    set(agents(), remote({ url: 'https://app.example.com', email: 'me@example.com', on: true, running: true, ready: true }))
    const { container, getByRole, getAllByRole } = render(RemotePanel)
    await fireEvent.click(getByRole('button', { name: 'Remove' }))
    expect(posted).toEqual([])
    // the confirmation's own Remove, above the panel's foot
    const confirm = getAllByRole('button', { name: 'Remove' })[0]
    await fireEvent.click(confirm)
    expect(posted.map((p) => p.path)).toEqual(['/api/remote/remove'])
    // removed: what went, never the steps to set it up again, nor a tunnel "starting"
    set(agents(), remote({ url: '', on: false, running: false, token: false }))
    await vi.waitFor(() => expect(container.textContent).toContain('app.example.com is removed'))
    expect(container.textContent).not.toContain('Starting the tunnel')
    expect(container.querySelectorAll('.ag-step')).toHaveLength(0)
    expect((getByRole('link', { name: 'Open Cloudflare’s API tokens' }) as HTMLAnchorElement).href).toBe('https://dash.cloudflare.com/profile/api-tokens')
    // Bagholder's GitHub sign-in is kept for the next set-up, and said so, with GitHub's page for it
    expect(container.textContent).toContain('Bagholder’s GitHub sign-in, Bagholder at example.com, stays')
    expect((getByRole('link', { name: 'Open Bagholder’s GitHub app' }) as HTMLAnchorElement).href).toBe('https://github.com/settings/apps/bagholder-at-example-com/advanced')
    // the sign-in kept for an agents' tunnel: only the token made for Bagholder is left to delete
    expect(container.textContent).toContain('The token, named Bagholder, stays in your Cloudflare account')
    // the sign-in forgotten too: its tokens are said as well
    set(agents({ cloudflare: { signedIn: false, signingIn: false, signInUrl: null, said: '', domain: null } }), remote({ url: '', on: false, running: false, token: false }))
    expect(container.textContent).toContain('the ones Cloudflare made each time you signed in, named for Cloudflare Tunnel')
  })
})
