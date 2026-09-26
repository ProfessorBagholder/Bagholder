// The end of a long list coming into view, inside the box that scrolls it: the
// list asks for its next rows (`subs.more`). An element placed after the last row.

export function atEnd(node: HTMLElement, reached: () => void): { update: (r: () => void) => void; destroy: () => void } {
  let fn = reached
  // the box that scrolls the list: a table's, or a card's own list
  const root = (node.closest('.scroll-xy') ?? node.closest('.scroll')) as HTMLElement | null
  const io = new IntersectionObserver((entries) => {
    if (entries.some((e) => e.isIntersecting)) fn()
  }, { root, rootMargin: '0px 0px 400px 0px' })
  io.observe(node)
  return {
    update: (r) => {
      fn = r
    },
    destroy: () => io.disconnect(),
  }
}
