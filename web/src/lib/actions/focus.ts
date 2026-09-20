// Give an element the keyboard when it appears. The `autofocus` attribute does not:
// a browser honours it on page load only, and ignores it once anything has had focus,
// which on this page is always.
export function focusOnMount(node: HTMLElement) {
  node.focus()
}
