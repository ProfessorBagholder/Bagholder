import { afterEach, describe, expect, it, vi } from 'vitest'
import { fireEvent, render } from '@testing-library/svelte'
import { flushSync } from 'svelte'
import AgentsPanel from './AgentsPanel.svelte'
import { status } from './subs.svelte'
import type { Status } from './model'
import type { AgentsStatus, TailscaleSays, Waiting } from './generated/agents'

// The plan's page check for who is asking (docs/plans/agents-from-anywhere.md §6, "The
// page"; brief 73, 4): a metadata-document client by its client_id's host, the name from
// its document beside it and never marked; a registered one by its redirect host, its
// name marked as the agent's own word. Rendered from the status the server sends: the
// e2e run cannot make a metadata-document client, whose document Bagholder fetches only
// from a public address, and a test run has none.

const asking = (w: Partial<Waiting>): Waiting => ({ id: 'r', code: 'ABCDEF', who: '', name: '', checked: false, redirectHost: '', madeAt: '2026-10-27T12:00:00Z', ...w })

function agents(waiting: Waiting[]): AgentsStatus {
  return {
    on: true,
    port: 40000,
    setup: null,
    address: '',
    programs: [],
    tailscale: null,
    tunnel: { running: null, command: [], lines: [], exited: null },
    ready: null,
    waiting,
    allowed: [],
    refused: [],
    error: '',
  }
}

describe('the Agents panel says who is asking', () => {
  afterEach(() => {
    status.data = null as unknown as Status
    vi.unstubAllGlobals()
  })

  it('a metadata-document client by its client_id host, its name unmarked; a registered one marked', () => {
    vi.stubGlobal('fetch', vi.fn(async () => new Response(JSON.stringify({ ok: true, lines: [] }))))
    status.data = {
      agents: agents([
        asking({ id: 'r1', code: 'ABCDEF', who: 'claude.ai', name: 'Claude', checked: true, redirectHost: 'claude.ai' }),
        asking({ id: 'r2', code: 'GHJKLM', who: 'agent.example', name: 'Totally Claude', checked: false, redirectHost: 'agent.example' }),
      ]),
    } as unknown as Status
    const { container } = render(AgentsPanel)
    flushSync()
    const cards = [...container.querySelectorAll('.agent-ask')].map((c) => c.textContent ?? '')
    expect(cards).toHaveLength(2)
    expect(cards[0]).toContain('claude.ai (Claude) is asking to read your trade history')
    expect(cards[0]).not.toContain('not checked')
    expect(cards[0]).toContain('its page shows code ABCDEF')
    expect(cards[1]).toContain('agent.example (“Totally Claude”, name given by the agent, not checked)')
  })
})

describe('the Agents panel with Tailscale chosen', () => {
  afterEach(() => {
    status.data = null as unknown as Status
    vi.unstubAllGlobals()
  })

  // brief 80 §5.5, §5.6: Funnel serves on 443 unless the person chooses 8443, a
  // program answering on 443 here or not; on Linux, the operator step is shown, and
  // not done until known done
  it('keeps 443 with a program on 443, offers 8443, and shows the operator step not done', async () => {
    vi.stubGlobal('fetch', vi.fn(async () => new Response(JSON.stringify({ ok: true, lines: [] }))))
    const tailscale: TailscaleSays = { installed: true, signedIn: true, https: true, funnel: true, name: 'box.tail1234.ts.net', address: '100.64.0.1', httpsPortTaken: true, said: '', operator: false }
    status.data = { agents: { ...agents([]), tailscale } } as unknown as Status
    const { container, getByRole } = render(AgentsPanel)
    flushSync()
    await fireEvent.click(getByRole('radio', { name: /Tailscale Funnel/ }))
    flushSync()
    const port = container.querySelector('select[aria-label="Funnel port"]') as HTMLSelectElement
    expect(port.value).toBe('443')
    expect(container.textContent).toContain('Free 443, or choose 8443.')
    const step = [...container.querySelectorAll('li')].find((li) => li.textContent?.includes('tailscale set --operator'))
    expect(step?.textContent).toBeTruthy()
    expect(step?.textContent).not.toContain('Done')
  })
})

describe('Copy steps for an agent', () => {
  afterEach(() => {
    status.data = null as unknown as Status
    vi.unstubAllGlobals()
  })

  // brief 80 §8.1: copying never makes a key; it uses the key the person made for
  // that agent and way in, or says to make one; a key for the other way in never serves
  it('uses the key made for that agent and way in, never makes one', async () => {
    const posted: string[] = []
    const fetch = vi.fn(async (url: string | URL | Request, init?: RequestInit) => {
      const path = String(url)
      if (init?.method === 'POST') posted.push(path)
      if (path.includes('/api/agents/keys')) {
        const asked = JSON.parse(String(init?.body ?? '{}'))
        return new Response(JSON.stringify({ ok: true, id: 'k1', name: asked.name, way: asked.way, key: `bh${asked.way}_made` }))
      }
      return new Response(JSON.stringify({ ok: true, lines: [] }))
    })
    vi.stubGlobal('fetch', fetch)
    const copied: string[] = []
    vi.stubGlobal('navigator', { clipboard: { writeText: async (t: string) => void copied.push(t) } })
    status.data = { agents: { ...agents([]), address: 'https://agents.example.com', ready: { state: 'ready' } } } as unknown as Status
    const { container, getByRole, getByLabelText } = render(AgentsPanel)
    flushSync()
    await fireEvent.click(getByRole('button', { name: /(Copy steps for|Copied:) Muse/ }))
    expect(posted.filter((p) => p.includes('/api/agents/keys'))).toEqual([])
    expect(copied.at(-1)).toContain('make one named Muse for the JSON API')
    // a key made for Muse over MCP does not serve its JSON API steps
    await fireEvent.input(getByLabelText('Key name'), { target: { value: 'Muse' } })
    await fireEvent.change(getByLabelText('Way in'), { target: { value: 'mcp' } })
    await fireEvent.click(getByRole('button', { name: 'Make key' }))
    await vi.waitFor(() => expect(container.textContent).toContain('bhmcp_made'))
    await fireEvent.click(getByRole('button', { name: /(Copy steps for|Copied:) Muse/ }))
    expect(copied.at(-1)).not.toContain('bhmcp_made')
    // the one made for its JSON API does
    await fireEvent.input(getByLabelText('Key name'), { target: { value: 'Muse' } })
    await fireEvent.change(getByLabelText('Way in'), { target: { value: 'api' } })
    await fireEvent.click(getByRole('button', { name: 'Make key' }))
    await vi.waitFor(() => expect(container.textContent).toContain('bhapi_made'))
    await fireEvent.click(getByRole('button', { name: /(Copy steps for|Copied:) Muse/ }))
    expect(copied.at(-1)).toContain('Authorization: Bearer bhapi_made')
    expect(posted.filter((p) => p.includes('/api/agents/keys'))).toHaveLength(2)
  })
})

