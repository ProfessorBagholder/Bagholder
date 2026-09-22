import {
  createChart,
  CandlestickSeries,
  createSeriesMarkers,
  CrosshairMode,
  type IChartApi,
  type ISeriesApi,
  type ISeriesMarkersPluginApi,
  type Time,
  type LogicalRange,
} from 'lightweight-charts'
import type { Fill } from '../model'
import { fillMarkers, localTime, STEP, type Bar, type ChartColors } from '../trade/chart'

// The trade chart as a Svelte `use:` action — a faithful port of ledger.html's
// drawTradeChart: candlesticks with each execution marked (up arrow below for a
// buy, down arrow above for a sell), the trade framed with ~15% padding either
// side, and labels shown on the arrows only once ≤40 executions are in view.
// The kept visible range persists per trade|tf across redraws, as the original's
// _chartRange does.
export interface TradeChartParams {
  bars: Bar[]
  fills: Fill[]
  tf: string
  colors: ChartColors
  rangeKey: string
  provisional?: boolean
}

const _chartRange: Record<string, LogicalRange> = {}

// What of the chart depends on what it was given, so an update redoes only that.
const sameFills = (a: Fill[], b: Fill[]) => a.length === b.length && a.every((f, i) => f.id === b[i].id && f.when === b[i].when && f.qty === b[i].qty && f.price === b[i].price)
const sameColors = (a: ChartColors, b: ChartColors) => a.text === b.text && a.grid === b.grid && a.up === b.up && a.down === b.down

export function tradeChart(node: HTMLElement, initial: TradeChartParams) {
  // The chart is made once, for the life of the element. An update is told apart
  // by what changed: other bars are new data on the same series, other fills are
  // new markers, another theme is new options -- and an update that changes none
  // of them (the card around it was given a new price) touches nothing. It used to
  // throw the chart away and build it again every time.
  let shown: TradeChartParams | null = null
  let chart: IChartApi | null = null
  let series: ISeriesApi<'Candlestick'> | null = null
  let marks: ISeriesMarkersPluginApi<Time> | null = null
  let relabel: (r: LogicalRange | null) => void = () => {}

  const orderedFills = (p: TradeChartParams) =>
    (p.fills || []).filter((f) => isFinite(Date.parse(f.when))).sort((a, b) => (a.when < b.when ? -1 : a.when > b.when ? 1 : 0))

  function chartOptions(p: TradeChartParams) {
    const c = p.colors
    return {
      layout: { background: { color: 'transparent' }, textColor: c.text, fontFamily: getComputedStyle(document.body).fontFamily, fontSize: 11, attributionLogo: true },
      grid: { vertLines: { color: c.grid }, horzLines: { color: c.grid } },
      rightPriceScale: { borderColor: c.grid },
      timeScale: { borderColor: c.grid, timeVisible: (STEP[p.tf] || 0) > 0, secondsVisible: false },
    }
  }
  const seriesOptions = (c: ChartColors) => ({ upColor: c.up, downColor: c.down, borderUpColor: c.up, borderDownColor: c.down, wickUpColor: c.up, wickDownColor: c.down })

  function make(p: TradeChartParams) {
    chart = createChart(node, {
      autoSize: true,
      ...chartOptions(p),
      crosshair: { mode: CrosshairMode.Normal },
      handleScroll: { mouseWheel: true, pressedMouseMove: true, horzTouchDrag: true, vertTouchDrag: false },
      handleScale: { mouseWheel: true, pinch: true, axisPressedMouseMove: true },
    })
    series = chart.addSeries(CandlestickSeries, { ...seriesOptions(p.colors), priceLineVisible: false, lastValueVisible: false })
    marks = createSeriesMarkers(series, [])
    chart.timeScale().subscribeVisibleLogicalRangeChange((r) => {
      if (r && shown && !shown.provisional) _chartRange[shown.rangeKey] = r
      if (r) relabel(r)
    })
  }

  // With real bars every execution is marked on its day, a zero-price fill (a
  // staking reward) included. Labels only once the view is zoomed in enough to
  // read them: at most 40 executions in view. Arrows are always shown.
  function mark(p: TradeChartParams): number[] {
    type Marks = Parameters<ISeriesMarkersPluginApi<Time>['setMarkers']>[0]
    const markers = fillMarkers(orderedFills(p), p.bars, p.colors)
    const bare = markers.map((m) => ({ ...m, text: '' }))
    const slot: Record<string, number> = {}
    p.bars.forEach((b, i) => {
      slot[String('date' in b ? (b.date as string) : localTime(b.time as number))] = i
    })
    const idx = markers.map((m) => slot[String(m.time)]).filter((i) => i != null).sort((a, b) => a - b)
    let labelled = false
    marks!.setMarkers(bare as unknown as Marks)
    relabel = (r) => {
      const want = !!r && idx.filter((i) => i >= r.from && i <= r.to).length <= 40
      if (want !== labelled) {
        labelled = want
        marks!.setMarkers((want ? markers : bare) as unknown as Marks)
      }
    }
    return idx
  }

  function frame(p: TradeChartParams, idx: number[]) {
    const fit = () => {
      if (!chart || shown !== p) return
      const kept = _chartRange[p.rangeKey]
      if (kept) chart.timeScale().setVisibleLogicalRange(kept)
      else if (!idx.length) chart.timeScale().fitContent()
      else {
        const a = idx[0]
        const b = idx[idx.length - 1]
        const pad = Math.max(4, Math.round((b - a) * 0.15))
        chart.timeScale().setVisibleLogicalRange({ from: a - pad, to: b + pad })
      }
      relabel(chart.timeScale().getVisibleLogicalRange())
    }
    fit()
    // autoSize learns the card's width a frame after it is made; fit again then
    requestAnimationFrame(() => requestAnimationFrame(fit))
  }

  function show(p: TradeChartParams) {
    const was = shown
    if (!chart) make(p)
    const bars = !was || was.bars !== p.bars || was.tf !== p.tf
    const fills = bars || !sameFills(orderedFills(was!), orderedFills(p))
    const colors = !!was && !sameColors(was.colors, p.colors)
    const view = bars || was!.rangeKey !== p.rangeKey
    shown = p
    if (colors || (was && was.tf !== p.tf)) chart!.applyOptions(chartOptions(p))
    if (colors) series!.applyOptions(seriesOptions(p.colors))
    if (bars) {
      series!.setData(
        p.bars.map((b) => ({
          time: ('date' in b ? b.date : localTime(b.time as number)) as Time,
          open: b.open as number,
          high: b.high as number,
          low: b.low as number,
          close: b.close as number,
        })),
      )
    }
    if (fills || colors || view) {
      const idx = mark(p)
      if (view) frame(p, idx)
      // a canvas says nothing to a screen reader: what it shows is said in words
      node.setAttribute('role', 'img')
      node.setAttribute('aria-label', `Price chart, ${p.tf.toUpperCase()}: ${p.bars.length} bars, ${idx.length} of ${orderedFills(p).length} executions marked`)
    }
  }

  show(initial)

  return {
    update(next: TradeChartParams) {
      show(next)
    },
    destroy() {
      if (chart) chart.remove()
      chart = null
    },
  }
}
