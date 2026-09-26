// A figure that changes rolls to its new value rather than cutting to it: each digit
// is a strip of 0 to 9 that slides, the ones column leading and each column to its
// left following 18 ms later, which is what makes it read as a counter rather than a
// wave. Only digits roll; a dollar sign, a separator or a minus is not a wheel. At
// rest the figure is ordinary text, so it can be read and copied as before; the strips
// exist only while the roll takes place, and go when the last of them has stopped.

const STAGGER_MS = 18
const LINE_EM = 1.15

const isDigit = (c: string) => c >= '0' && c <= '9'

/**
 * Two figures roll into one another only when they are the same shape: same length,
 * and the same non-digits in the same places. Anything else is a different number, not
 * the same number moved, and is simply set.
 */
export function comparable(a: string, b: string): boolean {
  if (!a || !b || a.length !== b.length || a === b) return false
  let digits = 0
  for (let i = 0; i < a.length; i++) {
    const da = isDigit(a[i])
    if (da !== isDigit(b[i])) return false
    if (!da && a[i] !== b[i]) return false
    if (da) digits++
  }
  return digits > 0
}

const still = () => typeof matchMedia === 'function' && matchMedia('(prefers-reduced-motion: reduce)').matches

function wheel(from: string, delay: number): HTMLElement {
  const w = document.createElement('span')
  w.className = 'rl-w'
  const strip = document.createElement('span')
  strip.className = 'rl-s'
  for (let d = 0; d <= 9; d++) {
    const b = document.createElement('b')
    b.textContent = String(d)
    strip.appendChild(b)
  }
  strip.style.transform = `translateY(-${(Number(from) * LINE_EM).toFixed(2)}em)`
  strip.style.transitionDelay = delay + 'ms'
  w.appendChild(strip)
  return w
}

export function roll(el: HTMLElement, text: string) {
  let shown = text
  let settle: (() => void) | null = null
  el.textContent = text

  function set(to: string) {
    settle?.()
    const from = shown
    shown = to
    if (still() || !el.isConnected || !comparable(from, to)) {
      el.textContent = to
      return
    }
    const plain = document.createElement('span')
    plain.className = 'rl-plain'
    plain.textContent = to
    const holder = document.createElement('span')
    holder.setAttribute('aria-hidden', 'true')
    const digits = [...to].filter(isDigit).length
    const moving: { strip: HTMLElement; to: string }[] = []
    let seen = 0
    for (let i = 0; i < to.length; i++) {
      if (!isDigit(to[i])) {
        holder.appendChild(document.createTextNode(to[i]))
        continue
      }
      seen++
      const w = wheel(from[i], (digits - seen) * STAGGER_MS)
      holder.appendChild(w)
      if (from[i] !== to[i]) moving.push({ strip: w.firstChild as HTMLElement, to: to[i] })
    }
    el.replaceChildren(plain, holder)
    void el.offsetHeight // the strips start on the old digits; settle that before asking for the new ones
    // the leftmost wheel that moves is the last to stop: its end is the roll's end
    const last = moving[0].strip
    const done = (e: TransitionEvent) => {
      if (e.target === last) settle?.()
    }
    settle = () => {
      last.removeEventListener('transitionend', done)
      last.removeEventListener('transitioncancel', done)
      settle = null
      el.textContent = shown
    }
    last.addEventListener('transitionend', done)
    last.addEventListener('transitioncancel', done)
    for (const m of moving) m.strip.style.transform = `translateY(-${(Number(m.to) * LINE_EM).toFixed(2)}em)`
  }

  return {
    update: set,
    destroy() {
      settle?.()
    },
  }
}
