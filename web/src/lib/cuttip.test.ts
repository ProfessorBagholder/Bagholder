import { afterEach, describe, expect, it } from 'vitest'
import { namedElement, startCutTip } from './cuttip'

// SPEC §4 Portfolio, Holdings: a row names what it does not show (its account) on hover,
// in the app's own tip, never the browser's.

let stop: (() => void) | undefined
afterEach(() => {
  stop?.()
  stop = undefined
  document.body.innerHTML = ''
})

const table = (tip: string | null) => {
  document.body.innerHTML = `<table><tbody><tr ${tip == null ? '' : `data-tip="${tip}"`}><td><span>ABC</span></td><td>1</td></tr></tbody></table><p id="away">x</p>`
  return document.querySelector('span')!
}

describe('a row that names what it does not show', () => {
  it('is found from anything inside it, and only where it names something', () => {
    expect(namedElement(table('An account'))?.tagName).toBe('TR')
    expect(namedElement(table(''))).toBeNull()
    expect(namedElement(table(null))).toBeNull()
    expect(namedElement(null)).toBeNull()
  })

  it("shows its name in the app's tip while the pointer is on it, and hides it when the pointer leaves", () => {
    const name = 'Account ' + Math.random()
    const cell = table(name)
    stop = startCutTip()
    cell.dispatchEvent(new MouseEvent('mouseover', { bubbles: true }))
    const tip = document.getElementById('cutTip')!
    expect(tip.hidden).toBe(false)
    expect(tip.querySelector('.tv')!.textContent).toBe(name)
    cell.dispatchEvent(new MouseEvent('mouseout', { bubbles: true, relatedTarget: document.getElementById('away') }))
    expect(tip.hidden).toBe(true)
  })
})
