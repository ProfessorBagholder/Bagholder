import { createChart, LineSeries, createSeriesMarkers, CrosshairMode, type IChartApi, type ISeriesApi, type ISeriesMarkersPluginApi, type Time, type LogicalRange } from 'lightweight-charts'
import type { Fill } from '../model'
import { fillPoints, type ChartColors, type FillPoint } from '../trade/chart'

// SPEC §6, Trade chart: when no history source covers the instrument, the priced
// executions themselves are plotted on a zoomable time axis. The series carries only
// the axis (one real fill price per instant, its line and points hidden, the price
// scale spanning every fill); the executions are markers at their own prices. Framed
// like the bar chart, labelled once no more than 40 are in view.
export interface FillsChartParams {
  fills: Fill[]
  colors: ChartColors
}

type Marks = Parameters<ISeriesMarkersPluginApi<Time>['setMarkers']>[0]

export function fillsChart(node: HTMLElement, initial: FillsChartParams) {
  let chart: IChartApi | null = null
  let series: ISeriesApi<'Line'> | null = null
  let marks: ISeriesMarkersPluginApi<Time> | null = null
  let points: FillPoint[] = []
  let slots: number[] = []
  let labelled: boolean | null = null
  let low = 0
  let high = 0
  // framed once the chart has a width, then the view is the person's until other fills arrive
  let framed = false
  let shown: FillsChartParams | null = null

  const options = (c: ChartColors, timed: boolean) => ({
    layout: { background: { color: 'transparent' }, textColor: c.text, fontFamily: getComputedStyle(document.body).fontFamily, fontSize: 11, attributionLogo: true },
    grid: { vertLines: { color: c.grid }, horzLines: { color: c.grid } },
    // room above the highest and below the lowest for the arrows and their labels
    rightPriceScale: { borderColor: c.grid, scaleMargins: { top: 0.2, bottom: 0.2 } },
    timeScale: { borderColor: c.grid, timeVisible: timed, secondsVisible: false, lockVisibleTimeRangeOnResize: true },
  })

  const relabel = (r: LogicalRange | null) => {
    const want = !!r && slots.filter((i) => i >= r.from && i <= r.to).length <= 40
    if (want === labelled) return
    labelled = want
    marks!.setMarkers(points.map((p) => ({ time: p.time as Time, position: p.position, price: p.price, shape: p.shape, color: p.color, text: want ? p.text : '' })) as Marks)
  }

  function show(p: FillsChartParams) {
    const same = !!shown && shown.fills.length === p.fills.length && shown.fills.every((f, i) => f.id === p.fills[i].id && f.when === p.fills[i].when && f.price === p.fills[i].price && f.qty === p.fills[i].qty)
    const recolour = !!shown && JSON.stringify(shown.colors) !== JSON.stringify(p.colors)
    shown = p
    if (same && !recolour) return
    points = fillPoints(p.fills, p.colors)
    const timed = points.length > 0 && typeof points[0].time === 'number'
    if (!chart) {
      chart = createChart(node, {
        autoSize: true,
        ...options(p.colors, timed),
        crosshair: { mode: CrosshairMode.Normal },
        handleScroll: { mouseWheel: true, pressedMouseMove: true, horzTouchDrag: true, vertTouchDrag: false },
        handleScale: { mouseWheel: true, pinch: true, axisPressedMouseMove: true },
      })
      series = chart.addSeries(LineSeries, {
        lineVisible: false,
        pointMarkersVisible: false,
        priceLineVisible: false,
        lastValueVisible: false,
        crosshairMarkerVisible: false,
        autoscaleInfoProvider: () => ({ priceRange: { minValue: low, maxValue: high } }),
      })
      marks = createSeriesMarkers(series, [])
      chart.timeScale().subscribeVisibleLogicalRangeChange(relabel)
      chart.timeScale().subscribeSizeChange((w) => {
        if (w > 0 && !framed) frame()
      })
    } else chart.applyOptions(options(p.colors, timed))
    low = Math.min(...points.map((x) => x.price))
    high = Math.max(...points.map((x) => x.price))
    // one instant, one place on the axis: the first execution's price there
    const data: { time: Time; value: number }[] = []
    slots = []
    for (const x of points) {
      if (!data.length || data[data.length - 1].time !== x.time) data.push({ time: x.time as Time, value: x.price })
      slots.push(data.length - 1)
    }
    series!.setData(data)
    labelled = null
    if (same) relabel(chart.timeScale().getVisibleLogicalRange())
    else {
      framed = false
      frame()
    }
    node.setAttribute('role', 'img')
    node.setAttribute('aria-label', `Executions chart: ${points.length} executions at their prices`)
  }

  // the executions with a margin of about 15% of their span either side
  function frame() {
    if (!chart || chart.timeScale().width() <= 0) return
    const last = slots.length ? slots[slots.length - 1] : 0
    const pad = Math.max(1, Math.round(last * 0.15))
    chart.timeScale().setVisibleLogicalRange({ from: -pad, to: last + pad })
    framed = true
    relabel(chart.timeScale().getVisibleLogicalRange())
  }

  show(initial)
  return {
    update(next: FillsChartParams) {
      show(next)
    },
    destroy() {
      chart?.remove()
      chart = null
    },
  }
}
