/** Text to the clipboard on any page: the Clipboard API where the browser offers it
 * (https, or this computer's own 127.0.0.1), else the older copy command, which the
 * browser also takes on a page opened over plain http on the local network. */
export async function copyText(text: string): Promise<void> {
  if (navigator.clipboard?.writeText) return navigator.clipboard.writeText(text)
  const field = document.createElement('textarea')
  field.value = text
  field.setAttribute('readonly', '')
  field.style.position = 'fixed'
  field.style.opacity = '0'
  document.body.append(field)
  field.select()
  const copied = document.execCommand('copy')
  field.remove()
  if (!copied) throw new Error('the browser did not copy it')
}
