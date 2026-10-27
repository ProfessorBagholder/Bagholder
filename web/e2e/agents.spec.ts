import { expect, test, type APIRequestContext } from '@playwright/test'
import { request as httpRequest } from 'node:http'
import { ready } from './helpers'

// The Agents panel (docs/plans/agents-from-anywhere.md §4, §6 "The page"): the tunnels
// side by side, Tailscale marked recommended and none chosen for the person; a sign-in
// waiting with its code, Allow and Deny, and who is asking said as the agent gave it,
// marked unchecked; a key shown once. The agents' address is reached on loopback with the
// public name a tunnel would send; nothing leaves the machine.

const PUBLIC = 'agents.example.com'
// a PKCE S256 challenge (any 43 characters of the alphabet do for a request)
const CHALLENGE = 'W15NQRG56e508qdyi6FrpLycmzoIRN-fBvEBGJkYnvE'

const write = (request: APIRequestContext, path: string, body: unknown) => request.post(path, { data: body, headers: { 'X-Bagholder': '1' } })

/** One request to the agents' listener, as the tunnel hands it on: the public name in Host. */
function agentsAsk(port: number, method: string, path: string, body?: string): Promise<{ status: number; text: string }> {
  return new Promise((done, fail) => {
    const req = httpRequest({ host: '127.0.0.1', port, method, path, headers: { host: PUBLIC, ...(body ? { 'content-type': 'application/json', 'content-length': Buffer.byteLength(body) } : {}) } }, (res) => {
      let text = ''
      res.setEncoding('utf8')
      res.on('data', (c) => (text += c))
      res.on('end', () => done({ status: res.statusCode ?? 0, text }))
    })
    req.on('error', fail)
    if (body) req.write(body)
    req.end()
  })
}

test.afterEach(async ({ request }) => {
  // what the test made goes, and the address closes: the next test's book is as it was
  await write(request, '/api/data/clear', { kinds: ['agents'] })
})

test('the Agents panel offers every tunnel side by side, a waiting sign-in with its code, and a key shown once', async ({ page, request }) => {
  expect((await write(request, '/api/agents/switch', { on: true })).ok()).toBe(true)
  await page.goto('/')
  await ready(page)
  await page.getByRole('button', { name: 'Menu' }).click()
  await page.getByRole('button', { name: /^Agents/ }).click()
  const panel = page.locator('#modalDlg')
  await expect(panel.getByRole('heading', { name: 'Agents' })).toBeVisible()

  // every tunnel, side by side; Tailscale recommended; none chosen for the person
  for (const t of ['Tailscale Funnel', 'Cloudflare Tunnel', 'ngrok', 'zrok', 'My own reverse proxy']) {
    await expect(panel.getByRole('radio', { name: t })).toBeVisible()
    await expect(panel.getByRole('radio', { name: t })).not.toBeChecked()
  }
  await expect(panel.locator('label.agent-tunnel', { hasText: 'Tailscale Funnel' })).toContainText('Recommended')
  await expect(panel.locator('label.agent-tunnel', { hasText: 'Cloudflare Tunnel' })).not.toContainText('Recommended')

  // the person's own proxy, by its public name
  expect((await write(request, '/api/agents/tunnel', { provider: 'own', name: PUBLIC })).ok()).toBe(true)
  const st = await (await request.get('/api/status')).json()
  const port: number = st.agents.port
  expect(port).toBeGreaterThan(0)

  // an agent registers itself and asks to sign in
  const reg = await agentsAsk(port, 'POST', '/oauth/register', JSON.stringify({ redirect_uris: ['https://agent.example/cb'], client_name: 'Totally Claude' }))
  expect(reg.status).toBe(201)
  const client = JSON.parse(reg.text).client_id as string
  const q = new URLSearchParams({ response_type: 'code', client_id: client, redirect_uri: 'https://agent.example/cb', state: 's', code_challenge: CHALLENGE, code_challenge_method: 'S256', resource: `https://${PUBLIC}/mcp` })
  const asked = await agentsAsk(port, 'GET', `/oauth/authorize?${q}`)
  expect(asked.status).toBe(200)
  const code = /letter-spacing:\.2em'>([A-Z]{6})</.exec(asked.text)?.[1]
  expect(code).toBeTruthy()

  // the header says it waits, and the panel shows who asks, as the agent said, unchecked
  await expect(page.getByRole('button', { name: `An agent is asking · ${code}` })).toBeVisible()
  const card = panel.locator('.agent-ask')
  await expect(card).toContainText('agent.example')
  await expect(card).toContainText('“Totally Claude”, name given by the agent, not checked')
  await expect(card).toContainText(`its page shows code ${code}`)
  await expect(card.getByRole('button', { name: 'Allow' })).toBeVisible()
  await card.getByRole('button', { name: 'Deny' }).click()
  await expect(card).toHaveCount(0)
  await expect(page.getByRole('button', { name: /An agent is asking/ })).toHaveCount(0)

  // a key, shown this once
  await panel.getByLabel('Key name').fill('Muse')
  await panel.getByLabel('Way in').selectOption('api')
  await panel.getByRole('button', { name: 'Make key' }).click()
  const shown = panel.locator('.agent-key')
  await expect(shown).toContainText('The key for Muse')
  const key = (await shown.locator('code').textContent()) ?? ''
  expect(key.startsWith('bhj_')).toBe(true)
  // closed and opened again: the key is not shown again
  await panel.getByRole('button', { name: 'Close' }).click()
  await page.getByRole('button', { name: 'Menu' }).click()
  await page.getByRole('button', { name: /^Agents/ }).click()
  await expect(page.locator('#modalDlg')).not.toContainText(key)
  await expect(page.locator('#modalDlg')).toContainText('API key')
})
