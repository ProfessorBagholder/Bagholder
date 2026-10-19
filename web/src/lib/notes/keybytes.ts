/** A base64url key as the bytes `pushManager.subscribe` takes. The worker keeps its
 * own copy (`sw.ts`), so the page shares no file with it and the worker builds alone. */
export function keyBytes(b64url: string): Uint8Array {
  const b64 = b64url.replace(/-/g, '+').replace(/_/g, '/') + '='.repeat((4 - (b64url.length % 4)) % 4)
  return Uint8Array.from(atob(b64), (c) => c.charCodeAt(0))
}
