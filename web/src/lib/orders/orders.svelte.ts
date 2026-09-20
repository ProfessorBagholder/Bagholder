// Orders and brackets the app placed, polled while the panel is open.

export interface Order {
  id: string; createdAt: string; account: string; symbol: string; currency: string
  side: string; type: string; quantity: number; limitPrice: number | null; stopPrice: number | null
  tif: string; status: string; filledQty: number | null; avgFill: number | null; role: string; exchange: string
  error?: string
}
export interface Bracket {
  id: string; symbol: string; quantity: number; slKind: string; slPrice: number | null
  slTrail: number | null; slTrailUnit: string; tpPrice: number | null; status: string; outcome: string
}

export const ordersStore = $state<{ orders: Order[]; brackets: Bracket[]; loaded: boolean }>({ orders: [], brackets: [], loaded: false })

const POLL_MS = 10000
let timer: ReturnType<typeof setTimeout> | undefined
let seq = 0

export async function loadOrders(): Promise<void> {
  const my = ++seq
  try {
    const r = await fetch('/api/orders')
    const d = await r.json()
    if (my !== seq) return
    if (d.ok) {
      ordersStore.orders = d.orders ?? []
      ordersStore.brackets = d.brackets ?? []
    }
  } catch {
    /* leave last */
  }
  ordersStore.loaded = true
}

export function startOrdersPoll(): void {
  loadOrders()
  clearTimeout(timer)
  const tick = () => { timer = setTimeout(async () => { await loadOrders(); tick() }, POLL_MS) }
  tick()
}
export function stopOrdersPoll(): void {
  seq++
  clearTimeout(timer)
}

// A pending order can be cancelled; a filled/cancelled/rejected one cannot.
export function cancellable(o: Order): boolean {
  return /pending|new|submitted|placed|partial|contingent/i.test(o.status)
}
export async function cancelOrder(id: string): Promise<void> {
  try {
    await fetch('/api/order/cancel', { method: 'POST', headers: { 'content-type': 'application/json' }, body: JSON.stringify({ id }) })
  } finally {
    loadOrders()
  }
}
