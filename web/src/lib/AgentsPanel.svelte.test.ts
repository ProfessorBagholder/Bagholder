import { afterEach, describe, expect, it, vi } from 'vitest'
import { render } from '@testing-library/svelte'
import { flushSync } from 'svelte'
import AgentsPanel from './AgentsPanel.svelte'
import { status } from './subs.svelte'
import type { Status } from './model'
import type { AgentsStatus, Waiting } from './generated/agents'

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
