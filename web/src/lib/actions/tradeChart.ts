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

export function tradeChart(node: HTMLElement, initial: TradeChartParams) {
  let chart: IChartApi | null = null

  function draw(p: TradeChartParams) {
    if (chart) {
      chart.remove()
      chart = null
    }
    const c = p.colors
    const step = STEP[p.tf] || 0
    const bars = p.bars
    // With real bars every execution is marked on its day, a zero-price fill (a
    // staking reward) included.
    const fills = (p.fills || [])
      .filter((f) => isFinite(Date.parse(f.when)))
      .sort((a, b) => (a.when < b.when ? -1 : a.when > b.when ? 1 : 0))

    chart = createChart(node, {
      autoSize: true,
      layout: {
        background: { color: 'transparent' },
        textColor: c.text,
        fontFamily: getComputedStyle(document.body).fontFamily,
        fontSize: 11,
        attributionLogo: true,
      },
      grid: { vertLines: { color: c.grid }, horzLines: { color: c.grid } },
      rightPriceScale: { borderColor: c.grid },
      timeScale: { borderColor: c.grid, timeVisible: step > 0, secondsVisible: false },
      crosshair: { mode: CrosshairMode.Normal },
      handleScroll: { mouseWheel: true, pressedMouseMove: true, horzTouchDrag: true, vertTouchDrag: false },
      handleScale: { mouseWheel: true, pinch: true, axisPressedMouseMove: true },
    })
    const series: ISeriesApi<'Candlestick'> = chart.addSeries(CandlestickSeries, {
      upColor: c.up,
      downColor: c.down,
      borderUpColor: c.up,
      borderDownColor: c.down,
      wickUpColor: c.up,
      wickDownColor: c.down,
      priceLineVisible: false,
      lastValueVisible: false,
    })
    const barTimes: (string | number)[] = bars.map((b) => (b.date != null ? (b.date as string) : localTime(b.time as number)))
    series.setData(
      bars.map((b) => ({
        time: (b.date != null ? b.date : localTime(b.time as number)) as Time,
        open: b.open as number,
        high: b.high as number,
        low: b.low as number,
        close: b.close as number,
      })),
    )
    const markers = fillMarkers(fills, bars, c)
    // Labels only once the view is zoomed in enough to read them: at most 40
    // executions in view. Arrows are always shown.
    const bare = markers.map((m) => ({ ...m, text: '' }))
    const sm: ISeriesMarkersPluginApi<Time> = createSeriesMarkers(
      series,
      bare as unknown as Parameters<ISeriesMarkersPluginApi<Time>['setMarkers']>[0],
    )
    const slot: Record<string, number> = {}
    barTimes.forEach((tm, i) => {
      slot[String(tm)] = i
    })
    const idx = markers
      .map((m) => slot[String(m.time)])
      .filter((i) => i != null)
      .sort((a, b) => a - b)
    let labelled = false
    const relabel = (r: LogicalRange | null) => {
      const want = !!r && idx.filter((i) => i >= r.from && i <= r.to).length <= 40
      if (want !== labelled) {
        labelled = want
        sm.setMarkers((want ? markers : bare) as unknown as Parameters<ISeriesMarkersPluginApi<Time>['setMarkers']>[0])
      }
    }
    const rk = p.rangeKey
    const kept = _chartRange[rk]
    const frame = () => {
      if (!idx.length) {
        chart!.timeScale().fitContent()
        return
      }
      const a = idx[0]
      const b = idx[idx.length - 1]
      const pad = Math.max(4, Math.round((b - a) * 0.15))
      chart!.timeScale().setVisibleLogicalRange({ from: a - pad, to: b + pad })
    }
    const active = chart
    const fit = () => {
      if (chart !== active) return
      if (kept) chart!.timeScale().setVisibleLogicalRange(kept)
      else frame()
      relabel(chart!.timeScale().getVisibleLogicalRange())
    }
    fit()
    // autoSize learns the card's width a frame after the redraw; fit again then
    requestAnimationFrame(() => requestAnimationFrame(fit))
    chart.timeScale().subscribeVisibleLogicalRangeChange((r) => {
      if (r && !p.provisional) {
        _chartRange[rk] = r
        relabel(r)
      } else if (r) relabel(r)
    })
  }

  draw(initial)

  return {
    update(next: TradeChartParams) {
      draw(next)
    },
    destroy() {
      if (chart) chart.remove()
      chart = null
    },
  }
}
