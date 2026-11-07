// The page's motion in script: the app's one curve for anything that travels, the
// person's reduced-motion setting, and how a panel hung from a header button opens
// (`drop`) and goes (`lift`).

import type { TransitionConfig } from 'svelte/transition'

/** The four points of the app's curve for anything that travels: `--ease-travel` in
 * app.css is `cubic-bezier` of these (motion.test.ts holds the two together). */
export const TRAVEL = [0.22, 1, 0.36, 1] as const

/**
 * A CSS `cubic-bezier(x1, y1, x2, y2)` as a function of time: the curve's progress at
 * `t` in [0, 1], its `x` solved for `t` as a browser solves it (Newton's method, then
 * halving where the slope is too flat), so a Svelte transition eases exactly as the
 * stylesheet's curve does.
 */
export function cubicBezier(x1: number, y1: number, x2: number, y2: number): (t: number) => number {
  // each coordinate as a polynomial in s: 3(1-s)²s·p1 + 3(1-s)s²·p2 + s³
  const cx = 3 * x1, bx = 3 * (x2 - x1) - cx, ax = 1 - cx - bx
  const cy = 3 * y1, by = 3 * (y2 - y1) - cy, ay = 1 - cy - by
  const x = (s: number) => ((ax * s + bx) * s + cx) * s
  const y = (s: number) => ((ay * s + by) * s + cy) * s
  const dx = (s: number) => (3 * ax * s + 2 * bx) * s + cx
  const solve = (t: number) => {
    let s = t
    for (let i = 0; i < 8; i++) {
      const e = x(s) - t
      if (Math.abs(e) < 1e-7) return s
      const d = dx(s)
      if (Math.abs(d) < 1e-6) break
      s -= e / d
    }
    let lo = 0, hi = 1
    s = t
    while (hi - lo > 1e-7) {
      if (x(s) < t) lo = s
      else hi = s
      s = (lo + hi) / 2
    }
    return s
  }
  return (t: number) => (t <= 0 ? 0 : t >= 1 ? 1 : y(solve(t)))
}

/** The app's curve for anything that travels, for a transition in script. */
export const travel = cubicBezier(...TRAVEL)

/** Whether the person has asked their system for less motion. */
export const still = () => typeof matchMedia === 'function' && matchMedia('(prefers-reduced-motion: reduce)').matches

/** How long a panel takes to open, and to go. */
export const DROP_MS = 180
export const LIFT_MS = 120

// Whether the person's last press was on the keyboard rather than a pointer: what the
// keyboard opens or closes shows or goes at once, with no motion, since motion makes
// a keyboard action feel slow and delayed (a click, a tap or an outside click moves).
let keyed = false
if (typeof window !== 'undefined') {
  window.addEventListener('keydown', () => (keyed = true), true)
  // a pointer that moves (a hover opens a menu's list) or presses
  window.addEventListener('pointermove', () => (keyed = false), true)
  window.addEventListener('pointerdown', () => (keyed = false), true)
}

/**
 * A panel hung from a header button, or one of the menu's lists, opening: it fades in,
 * growing from just short of its full size out of its top right corner (where it hangs
 * from its button, or meets its row), on the app's curve, which starts at once and
 * settles. With less motion asked for, it only fades. Opened from the keyboard, it is
 * there at once.
 */
export function drop(node: Element): TransitionConfig {
  // asked open again while it was going: it is a live panel again
  node.removeAttribute('aria-hidden')
  ;(node as HTMLElement).inert = false
  if (keyed) return { duration: 0 }
  const calm = still()
  return {
    duration: DROP_MS,
    easing: travel,
    css: (t, u) => (calm ? `opacity:${t}` : `opacity:${t};transform-origin:top right;transform:scale(${1 - 0.05 * u})`),
  }
}

/**
 * The same panel going: it only fades, quicker than it came, and is no longer part of
 * the page while it does: it takes no clicks (one in those moments reaches what is under
 * it), no keys and no focus, and a screen reader no longer reads it. Closed from the
 * keyboard, it is gone at once.
 */
export function lift(node: Element): TransitionConfig {
  ;(node as HTMLElement).inert = true
  node.setAttribute('aria-hidden', 'true')
  if (keyed) return { duration: 0 }
  return { duration: LIFT_MS, easing: travel, css: (t) => `pointer-events:none;opacity:${t}` }
}
