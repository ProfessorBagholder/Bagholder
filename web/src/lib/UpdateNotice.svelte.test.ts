import { afterEach, describe, expect, it, vi } from 'vitest'
import { fireEvent, render } from '@testing-library/svelte'
import UpdateNotice from './UpdateNotice.svelte'
import type { Status } from './model'

// SPEC.md versions-and-the-update-check-14: a container copy's update notice
const status = (over: Partial<Status>): Status => ({ updateAvailable: true, updating: '', latestVersion: 'v9.9.9', updateUrl: 'https://example.com/x', canUpdate: false, updateBy: 'image', ...over }) as Status

afterEach(() => vi.restoreAllMocks())

describe('the update notice', () => {
  it('is a button where an updater installs it', async () => {
    const fetched = vi.fn(async () => new Response(JSON.stringify({ ok: true })))
    vi.stubGlobal('fetch', fetched)
    const { getByRole, queryByRole } = render(UpdateNotice, { status: status({ canUpdate: true, updateBy: 'container' }) })
    expect(queryByRole('link')).toBeNull()
    await fireEvent.click(getByRole('button', { name: 'Update to v9.9.9' }))
    await vi.waitFor(() => expect(fetched).toHaveBeenCalled())
    expect(String((fetched.mock.calls[0] as unknown[])[0])).toContain('/api/update')
  })

  it('says the command that updates a container with no updater, with a copy button, and links nowhere', async () => {
    const written = vi.fn(async () => {})
    vi.stubGlobal('navigator', { clipboard: { writeText: written } })
    const { getByRole, getByTestId, queryByRole } = render(UpdateNotice, { status: status({}) })
    expect(getByTestId('update-command').textContent).toBe('v9.9.9 available: docker compose pull && docker compose up -d')
    expect(queryByRole('link')).toBeNull()
    await fireEvent.click(getByRole('button', { name: 'Copy' }))
    expect(written).toHaveBeenCalledWith('docker compose pull && docker compose up -d')
  })

  it('shows nothing while an update installs, or with none on offer', () => {
    expect(render(UpdateNotice, { status: status({ updating: 'Updating to v9.9.9…', canUpdate: true }) }).container.textContent?.trim()).toBe('')
    expect(render(UpdateNotice, { status: status({ updateAvailable: false }) }).container.textContent?.trim()).toBe('')
  })
})
