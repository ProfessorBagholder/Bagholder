// Scrollbars appear while scrolling. A list that scrolls inside its card (`.scroll`,
// `.scroll-xy`) draws no thumb at rest; while it moves, and for a moment after, it does
// (app.css: `.scrolling`). The moment after is the one place this file keeps time: there
// is no "scrolling stopped" event to wait for instead (`scrollend` is not in Safari).

const LINGER_MS = 700

export function startScrollbars(): () => void {
  const timers = new WeakMap<Element, ReturnType<typeof setTimeout>>()
  const onScroll = (e: Event) => {
    const el = e.target
    if (!(el instanceof Element) || !(el.classList.contains('scroll') || el.classList.contains('scroll-xy'))) return
    el.classList.add('scrolling')
    clearTimeout(timers.get(el))
    timers.set(el, setTimeout(() => el.classList.remove('scrolling'), LINGER_MS))
  }
  document.addEventListener('scroll', onScroll, true)
  return () => document.removeEventListener('scroll', onScroll, true)
}
