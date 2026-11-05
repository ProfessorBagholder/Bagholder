import { expect, test, type APIRequestContext } from '@playwright/test'

// SPEC versions-and-the-update-check-14, end to end (docs/plans/docker-update-button.md):
// a copy running the shipped compose with its updater, the button pressed in its page,
// the container replaced with the next image, and the page back on the new server with
// nothing more pressed and never loaded again by hand. Run in CI alone (tests.yml,
// container), which starts the copy, gives the updater the next image and names both
// versions; run there twice, from the latest release and from this build. A page that
// never opened its stream again stood at `Updating to …` for good (2.21.0).
const FROM = process.env.E2E_UPDATE_FROM ?? ''
const TO = process.env.E2E_UPDATE_TO ?? ''

/** The server's status, or nothing while no server answers. */
async function status(request: APIRequestContext): Promise<Record<string, unknown>> {
  const r = await request.get('/api/status', { headers: { 'X-Bagholder': '1' } }).catch(() => null)
  return r?.ok() ? r.json() : {}
}

test('pressing Update replaces the container, and the page comes back on the new server by itself', async ({ page, request }) => {
  test.setTimeout(6 * 60_000)
  expect(FROM && TO, 'E2E_UPDATE_FROM and E2E_UPDATE_TO name the versions before and after').toBeTruthy()
  // the copy has asked for the latest release and is offered it
  await expect.poll(() => status(request), { timeout: 120_000 }).toMatchObject({ version: FROM, updateAvailable: true, canUpdate: true })
  const was = await status(request)
  let loads = 0
  page.on('request', (r) => { if (r.resourceType() === 'document') loads++ })
  await page.goto('/')
  await expect(page.locator('#hdr .ver')).toHaveText('v' + FROM)
  const update = page.getByRole('button', { name: /^Update to v/ })
  const asked = page.waitForResponse((r) => r.url().endsWith('/api/update') && r.request().method() === 'POST')
  await update.click()
  expect((await asked).ok()).toBe(true)
  await expect(page.locator('#syncline')).toContainText('Updating to v')
  await expect(update).toHaveCount(0)
  // the container replaced: another server answers
  let now: Record<string, unknown> = {}
  await expect.poll(async () => { now = await status(request); return !!now.startedAt && now.startedAt !== was.startedAt }, { timeout: 240_000 }).toBe(true)
  // the page, with nothing pressed: on the new version, the update no longer under
  // way, and the release offered again, which only the new server's status can say
  await expect(page.locator('#hdr .ver')).toHaveText('v' + TO, { timeout: 120_000 })
  await expect(update).toBeVisible({ timeout: 120_000 })
  await expect(page.locator('#syncline')).not.toContainText('Updating')
  // loaded again by itself only for another build (its version or protocol), and once
  expect(loads).toBe(now.version !== was.version || now.protocol !== was.protocol ? 2 : 1)
})
