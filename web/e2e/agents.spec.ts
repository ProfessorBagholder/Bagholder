import { expect, test, type APIRequestContext } from '@playwright/test'
import { request as httpRequest } from 'node:http'
import { createHash } from 'node:crypto'
import { ready } from './helpers'

// The Agents panel (brief 81 §3; docs/plans/agents-from-anywhere.md §4, §6 "The page"):
// opened on your agents once a way in is set up; a sign-in waiting with its code, who is
// asking said as the agent gave it, marked; the code typed on the agent's sign-in page
// connects it, and End it refuses one. The agents' address is reached on loopback with
// the public name a tunnel would send; nothing leaves the machine.

const PUBLIC = 'agents.example.com'
// the agent's PKCE secret, and the S256 challenge it asks with
const VERIFIER = 'a-made-up-pkce-verifier-of-more-than-forty-three-characters'
const CHALLENGE = createHash('sha256').update(VERIFIER).digest('base64url')

const write = (request: APIRequestContext, path: string, body: unknown) => request.post(path, { data: body, headers: { 'X-Bagholder': '1' } })

/** One request to the agents' listener, as the tunnel hands it on: the public name in Host. */
function agentsAsk(port: number, method: string, path: string, body?: string, type = 'application/json'): Promise<{ status: number; text: string }> {
  return new Promise((done, fail) => {
    const req = httpRequest({ host: '127.0.0.1', port, method, path, headers: { host: PUBLIC, ...(body ? { 'content-type': type, 'content-length': Buffer.byteLength(body) } : {}) } }, (res) => {
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

test('a sign-in waits with its code in the panel, is connected by typing it on its page, and End it refuses one', async ({ page, request }) => {
  expect((await write(request, '/api/agents/switch', { on: true })).ok()).toBe(true)
  // the person's own proxy, by its public name
  expect((await write(request, '/api/agents/tunnel', { provider: 'own', name: PUBLIC })).ok()).toBe(true)
  const st = await (await request.get('/api/status')).json()
  const port: number = st.agents.port
  expect(port).toBeGreaterThan(0)
  await page.goto('/')
  await ready(page)
  await page.getByRole('button', { name: 'Menu' }).click()
  await page.getByRole('button', { name: /^Agents/ }).click()
  const panel = page.locator('#modalDlg')
  await expect(panel.getByRole('heading', { name: 'Agents' })).toBeVisible()
  await expect(panel).toContainText('No agents yet.')

  // an agent registers itself and asks to sign in
  const reg = await agentsAsk(port, 'POST', '/oauth/register', JSON.stringify({ redirect_uris: ['https://agent.example/cb'], client_name: 'Totally Claude' }))
  expect(reg.status).toBe(201)
  const client = JSON.parse(reg.text).client_id as string
  const signIn = async () => {
    const q = new URLSearchParams({ response_type: 'code', client_id: client, redirect_uri: 'https://agent.example/cb', state: 's', code_challenge: CHALLENGE, code_challenge_method: 'S256', resource: `https://${PUBLIC}/mcp` })
    const asked = await agentsAsk(port, 'GET', `/oauth/authorize?${q}`)
    expect(asked.status).toBe(200)
    // its page asks for the code, and never shows it
    expect(asked.text).toContain('Enter the code shown in Bagholder')
    return /var id="([0-9a-f]{32})"/.exec(asked.text)?.[1] ?? ''
  }
  const id = await signIn()
  expect(id).toBeTruthy()

  // the panel shows who asks, as the agent said, marked, with the code to type
  const card = panel.locator('.ag-asks')
  await expect(card).toContainText('agent.example')
  await expect(card).toContainText('calls itself “Totally Claude”')
  const code = ((await card.locator('.ag-digits').textContent()) ?? '').replace(/\s/g, '')
  expect(code).toMatch(/^\d{6}$/)
  await expect(page.getByRole('button', { name: `An agent is asking · ${code}` })).toBeVisible()

  // typed on its sign-in page, the agent is connected
  const typed = await agentsAsk(port, 'POST', '/oauth/authorize/code', JSON.stringify({ request: id, code }))
  expect(JSON.parse(typed.text).state).toBe('allowed')
  await expect(card).toHaveCount(0)
  // sent back with its code, it takes its token, as an agent does: then it is listed
  const back = JSON.parse((await agentsAsk(port, 'GET', `/oauth/authorize/status?request=${id}`)).text).redirect as string
  const issued = new URL(back).searchParams.get('code') ?? ''
  expect(issued).toBeTruthy()
  const form = new URLSearchParams({ grant_type: 'authorization_code', code: issued, redirect_uri: 'https://agent.example/cb', client_id: client, code_verifier: VERIFIER, resource: `https://${PUBLIC}/mcp` }).toString()
  const token = await agentsAsk(port, 'POST', '/oauth/token', form, 'application/x-www-form-urlencoded')
  expect(token.status, token.text).toBe(200)
  await expect(panel.getByRole('button', { name: /^Totally Claude/ })).toBeVisible()

  // another sign-in, ended here: refused
  await signIn()
  await expect(card).toHaveCount(1)
  await card.getByRole('button', { name: 'End it' }).click()
  await expect(card).toHaveCount(0)
  await expect(page.getByRole('button', { name: /An agent is asking/ })).toHaveCount(0)
})
