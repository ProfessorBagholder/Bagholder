// What Escape unwinds, innermost first.
//
// Whatever can be dismissed says so while it can (`escapable`, from an effect, which
// takes it back when the thing closes), and one key handler asks them in turn, the
// most recently opened first, until one says it did something. A card does not need
// to know what else is open, and the app does not need to know the cards.

type Handler = () => boolean

const open: Handler[] = []

/** While in force, Escape asks `dismiss`; it answers whether it dismissed anything. Returns what withdraws it. */
export function escapable(dismiss: Handler): () => void {
  open.push(dismiss)
  return () => {
    const i = open.lastIndexOf(dismiss)
    if (i >= 0) open.splice(i, 1)
  }
}

/** Ask each in turn, the most recently opened first. True when one of them took the key. */
export function dismissInnermost(): boolean {
  for (let i = open.length - 1; i >= 0; i--) {
    if (open[i]()) return true
  }
  return false
}
