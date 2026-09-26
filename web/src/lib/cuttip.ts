// What the reader cannot read: text cut by an ellipsis is shown whole, in a tip, while
// the pointer is on it; and what a row names without showing (a holding's account, its
// `data-tip`), in the same tip. The only hover the page has besides the charts' own.
//
// One listener for the whole page, because any cell of any table may be the cut one and
// which ones are changes with the window's width.

function tipNode(): HTMLElement {
  let tip = document.getElementById('cutTip')
  if (!tip) {
    tip = document.createElement('div')
    tip.id = 'cutTip'
    tip.className = 'tip'
    tip.hidden = true
    tip.innerHTML = '<div class="tv"></div><div class="tl"></div>'
    document.body.appendChild(tip)
  }
  return tip
}

// Whole pixels cannot answer "is it cut": scrollWidth and clientWidth are both rounded, and
// a column is rarely a whole number wide, so a header with 42.66 px of room reports 43
// either way and text of 43.05 px reads as fitting while the browser draws the ellipsis.
// Both sides are measured in fractions instead: the room from the box less its padding
// and border, the text from a range over the contents, which the browser lays out in full
// however little of it it paints.
function textWider(el: Element): boolean {
  if (el.scrollWidth > el.clientWidth) return true
  const st = getComputedStyle(el)
  const num = (v: string) => parseFloat(v) || 0
  const room = el.getBoundingClientRect().width - num(st.paddingLeft) - num(st.paddingRight) - num(st.borderLeftWidth) - num(st.borderRightWidth)
  try {
    const r = document.createRange()
    r.selectNodeContents(el)
    return r.getBoundingClientRect().width > room + 0.05
  } catch {
    return false
  }
}

function cutsText(el: Element): boolean {
  const st = getComputedStyle(el)
  // an inline box cannot clip whatever its overflow says: the block around it is the cut line
  if (st.display === 'inline') return false
  return st.textOverflow === 'ellipsis' && st.overflowX !== 'visible'
}

/** The innermost box that clips the text under `node`, when its text is in fact cut. */
export function cutElement(node: EventTarget | null): Element | null {
  for (let el = node instanceof Element ? node : null; el && el !== document.body; el = el.parentElement) {
    if (!el.textContent?.trim() || !cutsText(el)) continue
    // A row holding the cut line is not itself cut: its own box fits.
    if (Array.from(el.querySelectorAll('*')).some(cutsText)) return null
    return textWider(el) ? el : null
  }
  return null
}

/**
 * A row that names what it does not show, in its `data-tip` (a holding's account): read in
 * the same tip, while the pointer is on the row and no cut text under it wants the tip.
 */
export function namedElement(node: EventTarget | null): HTMLElement | null {
  const el = node instanceof Element ? (node.closest('[data-tip]') as HTMLElement | null) : null
  return el && el.dataset.tip ? el : null
}

function show(el: Element, text = el.textContent!.trim()): void {
  const tip = tipNode()
  tip.querySelector('.tv')!.textContent = text
  // a tile says what it is under its symbol, the one line a reader cannot get at any other way
  const under = (el.closest('[data-tip-sub]') as HTMLElement | null)?.dataset.tipSub ?? ''
  const tl = tip.querySelector('.tl') as HTMLElement
  tl.textContent = under
  tl.hidden = !under
  tip.hidden = false
  const r = el.getBoundingClientRect()
  const left = Math.max(6, Math.min(window.innerWidth - tip.offsetWidth - 6, r.left + r.width / 2 - tip.offsetWidth / 2))
  const above = r.top - tip.offsetHeight - 6
  tip.style.left = left + 'px'
  tip.style.top = (above >= 6 ? above : Math.min(window.innerHeight - tip.offsetHeight - 6, r.bottom + 6)) + 'px'
}

function hide(): void {
  const tip = document.getElementById('cutTip')
  if (tip) tip.hidden = true
}

/** Start showing cut text under the pointer. Returns what stops it. */
export function startCutTip(): () => void {
  const over = (e: MouseEvent) => {
    const cut = cutElement(e.target)
    const named = cut ? null : namedElement(e.target)
    if (cut) show(cut)
    else if (named) show(named, named.dataset.tip)
    else hide()
  }
  const out = (e: MouseEvent) => {
    if (!e.relatedTarget || !(cutElement(e.relatedTarget) || namedElement(e.relatedTarget))) hide()
  }
  document.addEventListener('mouseover', over)
  document.addEventListener('mouseout', out)
  window.addEventListener('scroll', hide, true)
  return () => {
    document.removeEventListener('mouseover', over)
    document.removeEventListener('mouseout', out)
    window.removeEventListener('scroll', hide, true)
    hide()
  }
}
