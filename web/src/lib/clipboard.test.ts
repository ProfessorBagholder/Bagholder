import { afterEach, describe, expect, it, vi } from 'vitest'
import { copyText } from './clipboard'

// A copy button works on every page: the Clipboard API where the browser offers it,
// the older copy command on a page opened over plain http, where it offers none.

afterEach(() => {
  vi.unstubAllGlobals()
  vi.restoreAllMocks()
})

describe('copying', () => {
  it('uses the Clipboard API where the browser offers it', async () => {
    const writeText = vi.fn(async () => {})
    vi.stubGlobal('navigator', { clipboard: { writeText } })
    await copyText('app.example.com')
    expect(writeText).toHaveBeenCalledWith('app.example.com')
  })

  it('copies with the older command on a page with no Clipboard API, leaving nothing behind', async () => {
    vi.stubGlobal('navigator', {})
    let selected = ''
    const exec = vi.fn(() => {
      selected = (document.activeElement as HTMLTextAreaElement | null)?.value ?? document.querySelector('textarea')?.value ?? ''
      return true
    })
    Object.defineProperty(document, 'execCommand', { value: exec, configurable: true })
    await copyText('192.168.1.8:8765')
    expect(exec).toHaveBeenCalledWith('copy')
    expect(selected).toBe('192.168.1.8:8765')
    expect(document.querySelector('textarea')).toBeNull()
  })

  it('says so when the browser copies nothing', async () => {
    vi.stubGlobal('navigator', {})
    Object.defineProperty(document, 'execCommand', { value: () => false, configurable: true })
    await expect(copyText('x')).rejects.toThrow('the browser did not copy it')
  })
})
