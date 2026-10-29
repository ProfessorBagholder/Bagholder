import { afterEach, describe, expect, it, vi } from 'vitest'
import { fireEvent, render } from '@testing-library/svelte'
import { flushSync } from 'svelte'
import AgentsPanel from './AgentsPanel.svelte'
import { ICONS } from './icons'
import { status } from './subs.svelte'
import type { Status } from './model'
import type { Address, AgentsStatus, Connected, TailscaleSays, Waiting } from './generated/agents'

// The Agents panel (brief 81 §3), rendered from the status the server sends: the e2e
// run cannot make a metadata-document client, whose document Bagholder fetches only
// from a public address, nor a tunnel, and a test run has neither.

const asking = (w: Partial<Waiting>): Waiting => ({ id: 'r', code: '482913', who: '', name: '', checked: false, madeAt: '2026-10-27T12:00:00Z', ...w })
const TS: TailscaleSays = { installed: true, signedIn: true, https: true, funnel: true, name: 'box.tail1234.ts.net', address: '100.64.0.1', httpsPortTaken: false, said: '', operator: null }
const TAILSCALE = 'https://box.tail1234.ts.net'
const CLOUDFLARE = 'https://agents.example.com'
const address = (a: Partial<Address>): Address => ({ provider: 'tailscale', url: TAILSCALE, port443: true, running: true, ready: { state: 'ready' }, exited: null, lines: [], ...a })

function agents(over: Partial<AgentsStatus> = {}): AgentsStatus {
  return {
    on: false,
    port: null,
    addresses: [],
    programs: [{ provider: 'tailscale', path: '/usr/bin/tailscale' }, { provider: 'cloudflare', path: '/usr/local/bin/cloudflared' }],
    computer: 'box',
    os: 'linux',
    installing: [],
    tailscale: null,
    cloudflare: { signedIn: false, signingIn: false, signInUrl: null, said: '' },
    waiting: [],
    allowed: [],
    refused: [],
    codesPaused: false,
    error: '',
    ...over,
  }
}
/** one address up and answering, through Tailscale */
const up = (over: Partial<AgentsStatus> = {}) =>
  agents({ on: true, port: 40000, addresses: [address({})], tailscale: TS, ...over })
const grant = (over: Partial<Connected> = {}): Connected => ({ id: 'g1', name: 'Claude', way: 'sign-in', who: 'claude.ai', at: TAILSCALE, madeAt: '2026-10-27T11:00:00Z', lastUsedAt: null, cutOffAt: null, ...over })
const set = (a: AgentsStatus) => {
  status.data = { agents: a } as unknown as Status
  flushSync()
}

/** every POST the panel sends, with its body; a key made answers with one named for its way */
function server() {
  const posted: { path: string; body: Record<string, unknown> }[] = []
  vi.stubGlobal('fetch', vi.fn(async (url: string | URL | Request, init?: RequestInit) => {
    const path = String(url)
    const body = JSON.parse(String(init?.body ?? '{}'))
    if (init?.method === 'POST') posted.push({ path, body })
    if (path.includes('/api/agents/keys')) return new Response(JSON.stringify({ ok: true, id: 'k1', name: body.name, way: body.way, key: `bh${body.way}_made` }))
    return new Response(JSON.stringify({ ok: true, lines: [] }))
  }))
  return posted
}
const paths = (posted: { path: string }[]) => posted.map((p) => p.path)

afterEach(() => {
  status.data = null as unknown as Status
  vi.unstubAllGlobals()
})

describe('the Agents panel says who is asking', () => {
  // brief 73, 4: a metadata-document client by its client_id's host, the name from its
  // document beside it, unmarked; a registered one's name marked as the agent's own word
  it('a checked name as it is, a name the agent gave marked as given, and the code with Copy', () => {
    server()
    set(up({ allowed: [grant()], waiting: [asking({ id: 'r1', who: 'claude.ai', name: 'Claude', checked: true }), asking({ id: 'r2', who: 'agent.example', name: 'Totally Claude', checked: false })] }))
    const { container, getAllByRole } = render(AgentsPanel)
    flushSync()
    const cards = [...container.querySelectorAll('.ag-asks')].map((c) => c.textContent ?? '')
    expect(cards).toHaveLength(2)
    expect(cards[0]).toContain('claude.ai (Claude) is signing in')
    expect(cards[0]).not.toContain('calls itself')
    expect(cards[0]).toContain('482 913')
    expect(getAllByRole('button', { name: 'Copy' })).toHaveLength(2)
    expect(cards[1]).toContain('agent.example (calls itself “Totally Claude”) is signing in')
  })
})

describe('connecting an agent', () => {
  it('walks the four steps from no address, shows the code on the step that waits for it, and names the agent as picked', async () => {
    const posted = server()
    set(agents())
    const { container, getByRole } = render(AgentsPanel)
    flushSync()
    expect(container.textContent).not.toContain('Step ')
    await fireEvent.click(getByRole('radio', { name: /Grok Bot/ }))
    await fireEvent.click(getByRole('button', { name: 'Continue' }))
    // every way in one picker, Tailscale first and picked; asked as the step opens
    expect(paths(posted)).toContain('/api/agents/tunnel/look')
    expect(getByRole('radio', { name: /^Tailscale/ }).getAttribute('aria-checked')).toBe('true')
    for (const w of [/^Cloudflare/, /^ngrok/, /^Your own proxy/]) expect(getByRole('radio', { name: w })).toBeTruthy()
    set(agents({ tailscale: { ...TS, funnel: false } }))
    expect(container.textContent).toContain('Which tunnel should make Bagholder’s MCP server reachable for Grok Bot?')
    expect(container.textContent).not.toContain('Your tunnels')
    expect((getByRole('button', { name: 'Continue' }) as HTMLButtonElement).disabled).toBe(true)
    set(agents({ tailscale: TS }))
    await fireEvent.click(getByRole('button', { name: 'Continue' }))
    await vi.waitFor(() => expect(container.textContent).toContain('Getting your tunnel ready'))
    // switched on, Tailscale saved on 443, started
    expect(paths(posted)).toEqual(expect.arrayContaining(['/api/agents/switch', '/api/agents/tunnel', '/api/agents/tunnel/start']))
    expect(posted.find((p) => p.path === '/api/agents/tunnel/start')?.body).toEqual({ provider: 'tailscale' })
    expect(posted.find((p) => p.path === '/api/agents/tunnel')?.body).toEqual({ provider: 'tailscale', name: '', funnelPort: 443, sendsHost: null })
    set(up({ addresses: [address({ ready: null })] }))
    expect((getByRole('button', { name: 'Continue' }) as HTMLButtonElement).disabled).toBe(true)
    // the address the tunnel prints is not a page to approve; a sign-in it asks for is
    set(up({ addresses: [address({ ready: null, lines: ['Available on the internet:', TAILSCALE + '/'] })] }))
    expect(container.textContent).not.toContain('approve this on its site')
    set(up({ addresses: [address({ ready: null, lines: ['To enable, visit:', 'https://login.tailscale.com/f/funnel?node=abc'] })] }))
    expect(container.textContent).toContain('approve this on its site')
    set(up())
    expect(container.textContent).toContain('Your tunnel is ready')
    expect(container.textContent).not.toContain('Details for troubleshooting')
    await fireEvent.click(getByRole('button', { name: 'Continue' }))
    expect(container.textContent).toContain('Now add Bagholder in Grok Bot')
    expect(container.textContent).toContain(TAILSCALE + '/mcp')
    expect(container.textContent).toContain('Waiting for Grok Bot…')
    set(up({ waiting: [asking({ who: 'An app on your computer', name: 'Grok Bot' })] }))
    expect(container.textContent).toContain('482 913')
    // Grok Bot signs in as Cursor: connected here, it is named what the person picked
    set(up({ allowed: [grant({ id: 'g9', name: 'Cursor', who: 'www.cursor.com' })] }))
    expect(container.textContent).toContain('Grok Bot is connected')
    await vi.waitFor(() => expect(posted.find((p) => p.path === '/api/agents/rename')?.body).toEqual({ id: 'g9', name: 'Grok Bot' }))
  })

  it('asks which tunnel the agent uses: yours first, picked, and a new one offered', async () => {
    server()
    set(up({ allowed: [grant()] }))
    const { container, getByRole } = render(AgentsPanel)
    flushSync()
    await fireEvent.click(getByRole('button', { name: '+ Connect agent' }))
    await fireEvent.click(getByRole('button', { name: 'Continue' }))
    expect(container.textContent).toContain('Which tunnel should make Bagholder’s MCP server reachable for Claude.ai?')
    expect(container.textContent).toContain('Your tunnels')
    expect(getByRole('radio', { name: /^Tailscale/ }).getAttribute('aria-checked')).toBe('true')
    expect(getByRole('radio', { name: /^Tailscale/ }).textContent).toContain(TAILSCALE + '/mcp')
    expect(container.textContent).toContain('New tunnel')
    for (const w of [/^Cloudflare/, /^ngrok/, /^Your own proxy/]) expect(getByRole('radio', { name: w })).toBeTruthy()
    await fireEvent.click(getByRole('button', { name: 'Continue' }))
    expect(container.textContent).toContain('Now add Bagholder in Claude.ai')
    expect(container.textContent).not.toContain('instead')
  })

  it('picks a tunnel of yours the agent can use, never one it cannot or a new one', async () => {
    server()
    set(up({ allowed: [grant({ name: 'Grok Bot' })], addresses: [address({ provider: 'cloudflare', url: CLOUDFLARE }), address({ port443: false })] }))
    const { getByRole } = render(AgentsPanel)
    flushSync()
    await fireEvent.click(getByRole('button', { name: '+ Connect agent' }))
    await fireEvent.click(getByRole('radio', { name: /^Claude/ }))
    await fireEvent.click(getByRole('button', { name: 'Continue' }))
    expect(getByRole('radio', { name: /^Cloudflare/ }).getAttribute('aria-checked')).toBe('true')
    expect(getByRole('radio', { name: /^ngrok/ }).getAttribute('aria-checked')).toBe('false')
  })

  // brief 81 §2: wrong codes pause sign-in until the person resumes it, here
  it('says sign-in is paused where the code would be, and resumes it', async () => {
    const posted = server()
    set(up({ allowed: [grant()] }))
    const { container, getByRole } = render(AgentsPanel)
    flushSync()
    await fireEvent.click(getByRole('button', { name: '+ Connect agent' }))
    await fireEvent.click(getByRole('button', { name: 'Continue' }))
    await fireEvent.click(getByRole('button', { name: 'Continue' }))
    set(up({ allowed: [grant()], codesPaused: true, waiting: [asking({})] }))
    expect(container.textContent).toContain('Sign-in is paused')
    expect(container.textContent).not.toContain('482 913')
    await fireEvent.click(getByRole('button', { name: 'Resume sign-in' }))
    expect(paths(posted)).toContain('/api/agents/codes/resume')
  })

  // claude.ai connects only on 443 (anthropics/claude-ai-mcp#1155): an address on 8443
  // is never offered to it; another is set up beside it, and the others keep theirs
  it('sets up a second address for Claude.ai when the one there is on 8443, keeping the first', async () => {
    const posted = server()
    set(up({ allowed: [grant({ name: 'Grok Bot' })], addresses: [address({ port443: false })] }))
    const { container, getByRole } = render(AgentsPanel)
    flushSync()
    await fireEvent.click(getByRole('button', { name: '+ Connect agent' }))
    await fireEvent.click(getByRole('button', { name: 'Continue' }))
    expect(container.textContent).not.toContain('Now add Bagholder')
    // Tailscale is shown, and why it can't be picked; Cloudflare is picked
    expect((getByRole('radio', { name: /^Tailscale/ }) as HTMLButtonElement).disabled).toBe(true)
    expect(getByRole('radio', { name: /^Tailscale/ }).textContent).toContain('Claude.ai needs port 443, and this one is on 8443')
    expect(getByRole('radio', { name: /^Cloudflare/ }).getAttribute('aria-checked')).toBe('true')
    expect(paths(posted)).not.toContain('/api/agents/tunnel/stop')
  })
})

describe('an error', () => {
  it('stays on the screen it happened on, and goes with Back', async () => {
    vi.stubGlobal('fetch', vi.fn(async (url: string | URL | Request) => new Response(JSON.stringify(String(url).includes('/api/agents/tunnel') ? { ok: false, error: 'That address is already used on your Cloudflare domain.' } : { ok: true, lines: [] }))))
    set(up({ allowed: [grant()], cloudflare: { signedIn: true, signingIn: false, signInUrl: null, said: '' } }))
    const { container, getByRole } = render(AgentsPanel)
    flushSync()
    await fireEvent.click(getByRole('button', { name: '+ Add tunnel' }))
    await fireEvent.click(getByRole('radio', { name: /^Cloudflare/ }))
    await fireEvent.click(getByRole('button', { name: 'Continue' }))
    await vi.waitFor(() => expect(container.textContent).toContain('That address is already used'))
    await fireEvent.click(getByRole('button', { name: 'Back' }))
    expect(container.textContent).not.toContain('That address is already used')
  })
})

describe('the steps in each agent', () => {
  // ChatGPT's form starts on STDIO with its own fields: the step names the type to pick
  it("tells ChatGPT's type, Streamable HTTP, and that nothing else is filled in", async () => {
    server()
    set(up({ allowed: [grant()] }))
    const { container, getByRole } = render(AgentsPanel)
    flushSync()
    await fireEvent.click(getByRole('button', { name: '+ Connect agent' }))
    await fireEvent.click(getByRole('radio', { name: /ChatGPT/ }))
    await fireEvent.click(getByRole('button', { name: 'Continue' }))
    await fireEvent.click(getByRole('button', { name: 'Continue' }))
    expect(container.textContent).toContain('Plugins → Add custom MCP server')
    expect(container.textContent).toContain('from STDIO to Streamable HTTP')
    expect(container.textContent).toContain('leave every other field empty')
  })
})

describe('Tailscale on the computer Bagholder runs on', () => {
  // Funnel on 443 reaches Bagholder only when no other program answers on 443 at this
  // computer's tailnet address: 443 then, else 8443, never a choice left to the person.
  // On Linux, Funnel is started by root or the user Tailscale names its operator: never
  // known before a start, so said only when a start failed, never a step that blocks
  it('takes 443 when it is free, 8443 when another program answers there, and asks for the operator only when a start failed', async () => {
    const posted = server()
    const saved = () => posted.filter((p) => p.path === '/api/agents/tunnel').map((p) => p.body.funnelPort)
    set(agents({ tailscale: { ...TS, httpsPortTaken: true, operator: false } }))
    const { container, getByRole } = render(AgentsPanel)
    flushSync()
    await fireEvent.click(getByRole('radio', { name: /Grok Bot/ }))
    await fireEvent.click(getByRole('button', { name: 'Continue' }))
    // nothing to choose, nothing said, nothing in the way
    expect(container.textContent).not.toContain('443')
    expect(container.textContent).not.toContain('operator')
    expect(container.querySelector('input[type="checkbox"], select')).toBeNull()
    await fireEvent.click(getByRole('button', { name: 'Continue' }))
    await vi.waitFor(() => expect(saved()).toEqual([8443]))
    set(up({ tailscale: { ...TS, operator: false }, addresses: [address({ port443: false, running: false, ready: null, exited: 'Tailscale ended (exit status 1).' })] }))
    expect(container.textContent).toContain('sudo tailscale set --operator=$USER')
    await fireEvent.click(getByRole('button', { name: 'Back' }))
    set(agents({ tailscale: { ...TS, httpsPortTaken: false, operator: false } }))
    await fireEvent.click(getByRole('button', { name: 'Continue' }))
    await vi.waitFor(() => expect(saved()).toEqual([8443, 443]))
  })

  it('shows Claude.ai, before it fails, that Tailscale on 8443 is no address for it', async () => {
    server()
    set(agents({ tailscale: { ...TS, httpsPortTaken: true } }))
    const { container, getByRole } = render(AgentsPanel)
    flushSync()
    await fireEvent.click(getByRole('button', { name: 'Continue' }))
    expect(container.textContent).toContain('Claude.ai needs port 443, which another program is using')
    expect((getByRole('radio', { name: /^Tailscale/ }) as HTMLButtonElement).disabled).toBe(true)
  })
})

describe('a key', () => {
  // brief 80 §8.1: opening a step never makes a key; it is made for that agent and
  // way in when the person asks, and shown once
  it("is made only when asked, for Muse's JSON API, and shown", async () => {
    const posted = server()
    set(up({ allowed: [grant()] }))
    const { container, getByRole } = render(AgentsPanel)
    flushSync()
    await fireEvent.click(getByRole('button', { name: '+ Connect agent' }))
    await fireEvent.click(getByRole('radio', { name: /Muse/ }))
    await fireEvent.click(getByRole('button', { name: 'Continue' }))
    await fireEvent.click(getByRole('button', { name: 'Continue' }))
    expect(container.textContent).toContain(TAILSCALE + '/api/agents/openapi.json')
    expect(posted.filter((p) => p.path.includes('/api/agents/keys'))).toEqual([])
    await fireEvent.click(getByRole('button', { name: 'Make a key' }))
    await vi.waitFor(() => expect(container.textContent).toContain('bhapi_made'))
    expect(posted.filter((p) => p.path.includes('/api/agents/keys')).map((p) => p.body)).toEqual([{ name: 'Muse', way: 'api', at: TAILSCALE }])
  })

  it("is made for another agent's MCP at /mcp/key, with no name to type", async () => {
    const posted = server()
    set(up({ allowed: [grant()] }))
    const { container, getByRole } = render(AgentsPanel)
    flushSync()
    await fireEvent.click(getByRole('button', { name: '+ Connect agent' }))
    await fireEvent.click(getByRole('radio', { name: /Another agent/ }))
    await fireEvent.click(getByRole('button', { name: 'Continue' }))
    // an agent not on the list is "your agent" in every sentence, never its card's name
    expect(container.querySelector('h5')?.textContent).toBe('Connect an agent')
    expect(container.textContent).toContain('Which tunnel should make Bagholder’s MCP server reachable for your agent?')
    await fireEvent.click(getByRole('button', { name: 'Continue' }))
    expect(container.textContent).toContain('Now add Bagholder in your agent')
    expect(container.textContent).not.toContain('Another agent')
    await fireEvent.click(getByRole('button', { name: /use a key instead/ }))
    expect(container.textContent).toContain(TAILSCALE + '/mcp/key')
    expect(container.querySelector('input')).toBeNull()
    await fireEvent.click(getByRole('button', { name: 'Make a key' }))
    await vi.waitFor(() => expect(container.textContent).toContain('bhmcp_made'))
    expect(posted.filter((p) => p.path.includes('/api/agents/keys')).map((p) => p.body)).toEqual([{ name: 'Another agent', way: 'mcp', at: TAILSCALE }])
  })
})

describe('your agents and their addresses', () => {
  it('lists each agent as one row, its name and what it is connected with, and the MCP server\'s addresses', () => {
    server()
    set(up({ allowed: [grant(), grant({ id: 'k1', name: 'Muse', way: 'api-key', who: '' })], refused: [{ why: 'with no key', count: 3 }] }))
    const { container, getByRole } = render(AgentsPanel)
    flushSync()
    expect(getByRole('button', { name: /^Claude\s*Connected with Tailscale$/ })).toBeTruthy()
    // each agent's dot shows its state, and stays out of the row's name
    expect(container.querySelectorAll('button.ag-agent .ag-dot.ok')).toHaveLength(2)
    expect(getByRole('button', { name: /^Muse\s*Connected with Tailscale$/ })).toBeTruthy()
    // the detail lives on the agent's page, not in the list
    expect(container.textContent).not.toContain('Signed in through')
    expect(container.textContent).not.toContain(' ago')
    expect(container.textContent).toContain('Tunnels')
    expect(container.textContent).toContain(TAILSCALE + '/mcp')
    expect(container.textContent).toContain('Refused 3 requests from unknown callers.')
  })

  it('says plainly that an agent whose address is gone cannot reach Bagholder', () => {
    server()
    set(up({ allowed: [grant({ name: 'Grok Bot', at: 'https://gone.tail1234.ts.net:8443' })] }))
    const { container } = render(AgentsPanel)
    flushSync()
    expect(container.textContent).toContain('Can’t reach Bagholder')
  })

  it("shows each address's MCP server address, and copies it", async () => {
    server()
    const copied: string[] = []
    vi.stubGlobal('navigator', { clipboard: { writeText: async (t: string) => void copied.push(t) } })
    set(up({ allowed: [grant()] }))
    const { container, getByRole } = render(AgentsPanel)
    flushSync()
    expect(container.textContent).toContain(TAILSCALE + '/mcp')
    await fireEvent.click(getByRole('button', { name: `Copy ${TAILSCALE}/mcp` }))
    await vi.waitFor(() => expect(copied).toEqual([TAILSCALE + '/mcp']))
    expect(getByRole('button', { name: `Copy ${TAILSCALE}/mcp` }).querySelector('path')?.getAttribute('d')).toBe(ICONS.check)
  })

  it('says an address is not running when it was stopped, never that it is getting ready, and turns it on', async () => {
    const posted = server()
    set(up({ allowed: [grant()], addresses: [address({ running: false, ready: null })] }))
    const { container, getByRole } = render(AgentsPanel)
    flushSync()
    expect(container.textContent).toContain('Tailscale · Not running')
    expect(container.textContent).not.toContain('Getting ready')
    await fireEvent.click(getByRole('button', { name: /^Tailscale/ }))
    expect(getByRole('switch', { name: 'Tailscale on' }).getAttribute('aria-checked')).toBe('false')
    await fireEvent.click(getByRole('switch', { name: 'Tailscale on' }))
    await vi.waitFor(() => expect(paths(posted)).toContain('/api/agents/tunnel/start'))
  })

  it('renames an agent where its name is, on its page, Enter keeping it and Escape ending only the edit', async () => {
    const posted = server()
    set(up({ allowed: [grant({ name: 'Cursor', who: 'www.cursor.com' })] }))
    const { container, getByRole } = render(AgentsPanel)
    flushSync()
    await fireEvent.click(getByRole('button', { name: /^Cursor/ }))
    const type = async (text: string) => {
      const el = getByRole('textbox', { name: 'Name' })
      el.textContent = text
      await fireEvent.input(el)
      return el
    }
    // the name itself edited: no form field put in its place
    await fireEvent.click(getByRole('button', { name: 'Rename Cursor' }))
    expect(container.querySelector('input')).toBeNull()
    const escaped = new KeyboardEvent('keydown', { key: 'Escape', bubbles: true, cancelable: true })
    const el = await type('Elsewhere')
    el.dispatchEvent(escaped)
    flushSync()
    expect(escaped.defaultPrevented).toBe(true)
    expect(container.querySelector('.ag-facts')).not.toBeNull()
    expect(posted.filter((p) => p.path === '/api/agents/rename')).toEqual([])
    await fireEvent.click(getByRole('button', { name: 'Rename Cursor' }))
    await fireEvent.blur(await type('  Grok Bot '))
    await vi.waitFor(() => expect(posted.filter((p) => p.path === '/api/agents/rename').map((p) => p.body)).toEqual([{ id: 'g1', name: 'Grok Bot' }]))
  })

  it("an agent's page: connected, its tunnel, when it was added, its activity, and Remove", async () => {
    const posted = server()
    set(up({ allowed: [grant({ name: 'Cursor', who: 'www.cursor.com' })] }))
    const { container, getByRole } = render(AgentsPanel)
    flushSync()
    await fireEvent.click(getByRole('button', { name: /^Cursor/ }))
    expect(container.querySelector('.ag-pill')?.textContent).toBe('Connected')
    expect(container.querySelector('.ag-facts')?.textContent).toContain('Tailscale')
    expect(getByRole('button', { name: `Copy ${TAILSCALE}/mcp` })).toBeTruthy()
    expect(container.textContent).toContain('through www.cursor.com')
    expect(container.querySelector('.ag-facts')?.textContent).toContain('Added')
    // its reads, asked as the page opens: no button to see them
    await vi.waitFor(() => expect(container.textContent).toContain('No activity yet.'))
    // while it works: its tunnel can be changed, and nothing needs reconnecting
    expect(container.textContent).not.toContain('Reconnect')
    await fireEvent.click(getByRole('button', { name: 'Change tunnel' }))
    expect(container.querySelector('h5')?.textContent).toBe('Change Cursor’s tunnel')
    expect(container.textContent).toContain('Which tunnel should make Bagholder’s MCP server reachable for Cursor?')
    // the tunnel it uses now is not one to change to
    expect((getByRole('radio', { name: /^Tailscale/ }) as HTMLButtonElement).disabled).toBe(true)
    expect(getByRole('radio', { name: /^Tailscale/ }).textContent).toContain('Cursor uses it now')
    await fireEvent.click(getByRole('button', { name: 'Back' }))
    // Remove asks once more, and Cancel keeps it
    await fireEvent.click(getByRole('button', { name: 'Remove agent' }))
    expect(container.textContent).toContain('Remove Cursor? It won’t be able to read your Bagholder anymore.')
    await fireEvent.click(getByRole('button', { name: 'Cancel' }))
    expect(paths(posted)).not.toContain('/api/agents/cut-off')
    await fireEvent.click(getByRole('button', { name: 'Remove agent' }))
    await fireEvent.click(getByRole('button', { name: 'Remove' }))
    await vi.waitFor(() => expect(posted.find((p) => p.path === '/api/agents/cut-off')?.body).toEqual({ id: 'g1' }))
  })

  it('adding a tunnel shows only what is missing; Bagholder installs ngrok where it runs, never in this browser', async () => {
    const posted = server()
    set(up({ allowed: [grant()], tailscale: TS }))
    const { container, getByRole, queryByLabelText } = render(AgentsPanel)
    flushSync()
    await fireEvent.click(getByRole('button', { name: '+ Add tunnel' }))
    expect(container.querySelector('h5')?.textContent).toBe('Add a tunnel')
    expect(container.textContent).toContain('Which tunnel should make Bagholder’s MCP server reachable?')
    await fireEvent.click(getByRole('radio', { name: /^ngrok/ }))
    // not installed: that alone, on the computer Bagholder runs on, named; nothing to download here
    expect(container.textContent).toContain('ngrok isn’t installed on box')
    expect(container.querySelector('.ag-note b')?.textContent).toBe('box')
    expect(container.querySelector('a[href*="ngrok.com/download"]')).toBeNull()
    expect(queryByLabelText('ngrok authtoken')).toBeNull()
    expect(container.querySelector('.ag-steps')).toBeNull()
    expect((getByRole('button', { name: 'Continue' }) as HTMLButtonElement).disabled).toBe(true)
    // Install: Bagholder installs it there, the panel following it
    await fireEvent.click(getByRole('button', { name: 'Install' }))
    await vi.waitFor(() => expect(posted.find((p) => p.path === '/api/agents/install')?.body).toEqual({ provider: 'ngrok' }))
    set(up({ allowed: [grant()], tailscale: TS, installing: [{ provider: 'ngrok', failed: '' }] }))
    expect(container.textContent).toContain('Installing ngrok on box…')
    set(up({ allowed: [grant()], tailscale: TS, installing: [{ provider: 'ngrok', failed: 'It couldn’t be installed: offline' }] }))
    expect(container.textContent).toContain('It couldn’t be installed: offline')
    expect(getByRole('button', { name: 'Try again' })).toBeTruthy()
    set(up({ allowed: [grant()], tailscale: TS, programs: [{ provider: 'ngrok', path: '/home/x/agents-ngrok/ngrok' }] }))
    expect(container.textContent).not.toContain('isn’t installed')
    await fireEvent.input(queryByLabelText('ngrok authtoken')!, { target: { value: 'tok' } })
    expect(getByRole('link', { name: 'Open ngrok' }).getAttribute('href')).toBe('https://dashboard.ngrok.com/get-started/your-authtoken')
    expect((getByRole('button', { name: 'Continue' }) as HTMLButtonElement).disabled).toBe(false)
    // your own proxy: its address typed whole, starting empty, never Cloudflare's name
    await fireEvent.click(getByRole('radio', { name: /^Your own proxy/ }))
    const proxy = container.querySelector('input[placeholder="agents.yourdomain.com"]') as HTMLInputElement
    expect(proxy.value).toBe('')
    expect((getByRole('button', { name: 'Continue' }) as HTMLButtonElement).disabled).toBe(true)
  })

  it('Tailscale missing on a Linux computer: the command to run there, never a download in this browser', async () => {
    server()
    set(agents({ tailscale: { ...TS, installed: false, signedIn: false }, programs: [] }))
    const { container, getByRole } = render(AgentsPanel)
    flushSync()
    await fireEvent.click(getByRole('button', { name: 'Continue' }))
    expect(container.textContent).toContain('Tailscale isn’t installed on box')
    expect(container.textContent).toContain('curl -fsSL https://tailscale.com/install.sh | sh')
    expect(container.querySelector('a[href*="tailscale.com/download"]')).toBeNull()
    set(agents({ tailscale: { ...TS, signedIn: false } }))
    expect(container.textContent).toContain('sudo tailscale up')
    expect(getByRole('radio', { name: /^Tailscale/ }).getAttribute('aria-checked')).toBe('true')
  })

  it("a key's agent: the tunnel its key was made for, named as every agent's is, and listed on that tunnel", async () => {
    server()
    set(up({ allowed: [grant({ id: 'k1', name: 'Muse', way: 'api-key', who: '' })] }))
    const { container, getByRole } = render(AgentsPanel)
    flushSync()
    await fireEvent.click(getByRole('button', { name: /^Muse/ }))
    expect(container.querySelector('.ag-pill')?.textContent).toBe('Connected')
    expect(container.querySelector('.ag-facts')?.textContent).toContain('Tailscale')
    expect(getByRole('button', { name: `Copy ${TAILSCALE}/mcp` })).toBeTruthy()
    await fireEvent.click(getByRole('button', { name: 'Back' }))
    await fireEvent.click(getByRole('button', { name: /^Tailscale/ }))
    expect(getByRole('button', { name: 'Muse' })).toBeTruthy()
  })

  it('reconnects an agent whose tunnel is gone through the same tunnel step as connecting: its sign-in there replaces the old one', async () => {
    const posted = server()
    const two = [address({ provider: 'cloudflare', url: CLOUDFLARE }), address({ provider: 'ngrok', url: 'https://x.ngrok-free.dev' })]
    set(up({ allowed: [grant({ name: 'Grok Bot', who: 'www.cursor.com' })], addresses: two }))
    const { container, getByRole } = render(AgentsPanel)
    flushSync()
    await fireEvent.click(getByRole('button', { name: /^Grok Bot/ }))
    expect(container.textContent).toContain('Its tunnel is gone')
    await fireEvent.click(getByRole('button', { name: 'Reconnect' }))
    expect(container.querySelector('h5')?.textContent).toBe('Reconnect Grok Bot')
    expect(container.textContent).toContain('Which tunnel should make Bagholder’s MCP server reachable for Grok Bot?')
    expect(container.textContent).toContain('Your tunnels')
    expect(getByRole('radio', { name: /^Cloudflare/ }).getAttribute('aria-checked')).toBe('true')
    expect(getByRole('radio', { name: /^Your own proxy/ })).toBeTruthy()
    await fireEvent.click(getByRole('button', { name: 'Continue' }))
    expect(container.textContent).toContain(CLOUDFLARE + '/mcp')
    set(up({ addresses: two, allowed: [grant({ name: 'Grok Bot', who: 'www.cursor.com' }), grant({ id: 'g2', name: 'Cursor', who: 'www.cursor.com', at: CLOUDFLARE })] }))
    await vi.waitFor(() => expect(posted.find((p) => p.path === '/api/agents/rename')?.body).toEqual({ id: 'g2', name: 'Grok Bot' }))
    await vi.waitFor(() => expect(posted.find((p) => p.path === '/api/agents/cut-off')?.body).toEqual({ id: 'g1' }))
    expect(container.textContent).toContain('Grok Bot is connected')
  })

  it('adds an address beside the one there, never stopping it', async () => {
    const posted = server()
    const signedIn = { signedIn: true, signingIn: false, signInUrl: null, said: '' }
    set(up({ allowed: [grant({ name: 'Grok Bot' })], cloudflare: signedIn }))
    const { container, getByRole, queryByRole } = render(AgentsPanel)
    flushSync()
    await fireEvent.click(getByRole('button', { name: '+ Add tunnel' }))
    // only the kinds that can be added: Tailscale, set up already, is not offered
    expect(queryByRole('radio', { name: /^Tailscale/ })).toBeNull()
    await fireEvent.click(getByRole('radio', { name: /^Cloudflare/ }))
    expect(container.textContent).toContain('Your address will be agents.')
    await fireEvent.click(getByRole('button', { name: 'Continue' }))
    await vi.waitFor(() => expect(posted.find((p) => p.path === '/api/agents/tunnel/start')?.body).toEqual({ provider: 'cloudflare' }))
    expect(posted.find((p) => p.path === '/api/agents/tunnel')?.body).toMatchObject({ provider: 'cloudflare', name: 'agents' })
    expect(paths(posted)).not.toContain('/api/agents/tunnel/stop')
    expect(paths(posted)).not.toContain('/api/agents/tunnel/remove')
    set(up({ allowed: [grant({ name: 'Grok Bot' })], cloudflare: signedIn, addresses: [address({}), address({ provider: 'cloudflare', url: CLOUDFLARE })] }))
    await fireEvent.click(getByRole('button', { name: 'Continue' }))
    expect(container.textContent).toContain(TAILSCALE + '/mcp')
    expect(container.textContent).toContain(CLOUDFLARE + '/mcp')
    expect(paths(posted)).not.toContain('/api/agents/tunnel/remove')
  })

  it("says on an agent's page, before anything else, that it can't reach Bagholder when its address is gone", async () => {
    server()
    set(up({ allowed: [grant({ name: 'Grok Bot', at: 'https://gone.tail1234.ts.net:8443' })] }))
    const { container, getByRole } = render(AgentsPanel)
    flushSync()
    await fireEvent.click(getByRole('button', { name: /^Grok Bot/ }))
    expect(container.querySelector('.ag-pill')?.textContent).toBe('Can’t reach Bagholder')
    expect(container.textContent).toContain('Its tunnel is gone')
  })

  it('removes an address only once asked again, naming the agents that use it', async () => {
    const posted = server()
    set(up({ allowed: [grant({ name: 'Grok Bot' })] }))
    const { container, getByRole } = render(AgentsPanel)
    flushSync()
    await fireEvent.click(getByRole('button', { name: /^Tailscale/ }))
    // the tunnel's page: its address, the agents using it, and its state
    expect(container.textContent).toContain('Grok Bot')
    await fireEvent.click(getByRole('button', { name: 'Remove tunnel' }))
    expect(container.textContent).toContain('Remove Tailscale? Grok Bot will stop reaching Bagholder.')
    await fireEvent.click(getByRole('button', { name: 'Cancel' }))
    expect(paths(posted)).not.toContain('/api/agents/tunnel/remove')
    await fireEvent.click(getByRole('button', { name: 'Remove tunnel' }))
    await fireEvent.click(getByRole('button', { name: 'Remove' }))
    await vi.waitFor(() => expect(posted.find((p) => p.path === '/api/agents/tunnel/remove')?.body).toEqual({ provider: 'tailscale' }))
  })

  it("opens a tunnel's page from anywhere on its row but the copy icon, which copies alone", async () => {
    server()
    const copied: string[] = []
    vi.stubGlobal('navigator', { clipboard: { writeText: async (t: string) => void copied.push(t) } })
    set(up({ allowed: [grant()] }))
    const { container, getByRole } = render(AgentsPanel)
    flushSync()
    await fireEvent.click(getByRole('button', { name: `Copy ${TAILSCALE}/mcp` }))
    await vi.waitFor(() => expect(copied).toEqual([TAILSCALE + '/mcp']))
    expect(container.textContent).toContain('Your agents')
    // the address's line, not its icon: the row's
    await fireEvent.click(container.querySelector('.ag-rowbtn[role="button"] .ag-block')!)
    expect(container.textContent).toContain('Remove tunnel')
  })

  it('shows a check for two seconds once copied, then the copy icon again, as the header’s copy button does', async () => {
    vi.useFakeTimers()
    server()
    vi.stubGlobal('navigator', { clipboard: { writeText: async () => {} } })
    set(up({ allowed: [grant()] }))
    const { getByRole } = render(AgentsPanel)
    flushSync()
    const icon = () => getByRole('button', { name: `Copy ${TAILSCALE}/mcp` }).querySelector('path')?.getAttribute('d')
    expect(icon()).toBe(ICONS.copy)
    await fireEvent.click(getByRole('button', { name: `Copy ${TAILSCALE}/mcp` }))
    await vi.waitFor(() => expect(icon()).toBe(ICONS.check))
    await vi.advanceTimersByTimeAsync(1900)
    flushSync()
    expect(icon()).toBe(ICONS.check)
    await vi.advanceTimersByTimeAsync(200)
    flushSync()
    expect(icon()).toBe(ICONS.copy)
    vi.useRealTimers()
  })

  it('goes back the same way on every screen: the header’s ‹, never a button at the bottom', async () => {
    server()
    set(up({ allowed: [grant()] }))
    const { container, getByRole, queryAllByRole } = render(AgentsPanel)
    flushSync()
    expect(queryAllByRole('button', { name: 'Back' })).toHaveLength(0)
    await fireEvent.click(getByRole('button', { name: '+ Connect agent' }))
    expect(container.querySelector('.ag-foot')?.textContent).not.toContain('Back')
    await fireEvent.click(getByRole('button', { name: 'Back' }))
    expect(container.textContent).toContain('Your agents')
    await fireEvent.click(getByRole('button', { name: /^Claude/ }))
    await fireEvent.click(getByRole('button', { name: 'Back' }))
    await fireEvent.click(getByRole('button', { name: /^Tailscale/ }))
    await fireEvent.click(getByRole('button', { name: 'Back' }))
    expect(container.textContent).toContain('Your agents')
  })

  it("a tunnel's page: its address, its agents opening their pages, and a switch that turns it off", async () => {
    const posted = server()
    set(up({ allowed: [grant({ name: 'Grok Bot' })] }))
    const { container, getByRole } = render(AgentsPanel)
    flushSync()
    await fireEvent.click(getByRole('button', { name: /^Tailscale/ }))
    expect(getByRole('button', { name: `Copy ${TAILSCALE}/mcp` })).toBeTruthy()
    expect(getByRole('switch', { name: 'Tailscale on' }).getAttribute('aria-checked')).toBe('true')
    await fireEvent.click(getByRole('switch', { name: 'Tailscale on' }))
    await vi.waitFor(() => expect(posted.find((p) => p.path === '/api/agents/tunnel/stop')?.body).toEqual({ provider: 'tailscale' }))
    expect(container.textContent).not.toContain('Replace')
    await fireEvent.click(getByRole('button', { name: 'Grok Bot' }))
    expect(container.querySelector('.ag-pill')?.textContent).toBe('Connected')
    await fireEvent.click(getByRole('button', { name: 'Back' }))
    expect(container.textContent).toContain('Your agents')
  })
})
