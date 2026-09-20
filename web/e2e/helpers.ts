import type { APIRequestContext, Page } from '@playwright/test'

/**
 * Open the page on the real book, with the header's status changed as given. The
 * server's own stream is stood in for by one snapshot: what a status the test cannot
 * bring about for real (an update on offer, a server of another protocol) looks like.
 */
export async function openWithStatus(page: Page, request: APIRequestContext, status: Record<string, unknown>, hash = ''): Promise<void> {
  const model = await (await request.get('/api/model')).json()
  model.status = { ...model.status, ...status }
  const body = `event: hello\ndata: {"id":1}\n\nevent: snapshot\ndata: ${JSON.stringify({ doc: 'model', data: model })}\n\n`
  await page.route('**/api/events?*', (route) => route.fulfill({ status: 200, contentType: 'text/event-stream', body }))
  await page.goto('/' + hash)
}
