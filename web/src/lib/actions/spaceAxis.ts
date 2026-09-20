// Two labels on a value axis never overlap: going down the axis, a label that would
// touch the one kept above it is hidden. Measured, because whether two labels touch
// depends on the card's height; measured again when the axis is resized or its ticks
// change.
export function spaceAxis(axis: HTMLElement, _ticks?: unknown) {
  const space = () => {
    const labels = Array.from(axis.children) as HTMLElement[]
    for (const el of labels) el.hidden = false
    let last: DOMRect | null = null
    for (const x of labels.map((el) => ({ el, r: el.getBoundingClientRect() })).sort((a, b) => a.r.top - b.r.top)) {
      if (last && x.r.top < last.bottom + 2) x.el.hidden = true
      else last = x.r
    }
  }
  const ro = new ResizeObserver(space)
  ro.observe(axis)
  space()
  return { update: space, destroy: () => ro.disconnect() }
}
