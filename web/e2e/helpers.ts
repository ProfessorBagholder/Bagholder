import { expect, type APIRequestContext, type Page } from '@playwright/test'

/** The page has its model and has finished arriving: the shell alone is there before that. */
export async function ready(page: Page): Promise<void> {
  await expect(page.locator('#page > [data-arrived]')).toBeVisible()
}

/**
 * Open the page on the real book, with the header's status changed as given. The
 * server's own stream is stood in for by one snapshot: what a status the test cannot
 * bring about for real (an update on offer, a server of another protocol) looks like.
 * `docs` are documents (the orders, a listing's filings) sent the same way.
 */
export async function openWithStatus(
  page: Page,
  request: APIRequestContext,
  status: Record<string, unknown>,
  hash = '',
  change: (model: Record<string, unknown>) => void = () => {},
  docs: Record<string, unknown> = {},
): Promise<void> {
  const model = await (await request.get('/api/model')).json()
  model.status = { ...model.status, ...status }
  change(model)
  // a document is sent with every (re)connection, so one the page asks for later reaches it on the next
  const others = Object.entries(docs).map(([doc, data]) => `event: snapshot\ndata: ${JSON.stringify({ doc, data })}\n\n`).join('')
  const body = `retry: 200\nevent: hello\ndata: {"id":1}\n\nevent: snapshot\ndata: ${JSON.stringify({ doc: 'model', data: model })}\n\n${others}`
  await page.route('**/api/events?*', (route) => route.fulfill({ status: 200, contentType: 'text/event-stream', body }))
  await page.goto('/' + hash)
}
