// The view half of the model, a port of crates/model/src/view.rs filters, NAV analytics and
// build_view: one filter set applied to the base, and everything a page shows
// computed from the same filtered lists.
package com.bagholder.model

import java.util.Locale
import kotlin.math.abs
import kotlin.math.pow

data class EquityPoint(val d: String, val v: Double, val dep: Double?)

data class Range(var op: String = ">", var v: Double? = null)

/** One filter set, the shape clean_filters produces. */
class Filters {
    val lists: MutableMap<String, List<String>> = LIST_KEYS.associateWith { emptyList<String>() }.toMutableMap()
    val ranges: MutableMap<String, Range> = RANGE_KEYS.associateWith { Range() }.toMutableMap()
    var preset = "all"
    var years: List<String> = emptyList()
    var from = ""
    var to = ""
    var search = ""
    var benchmark = "SP500"

    val isActive: Boolean
        get() = lists.values.any { it.isNotEmpty() } || ranges.values.any { it.v != null } || preset != "all" || years.isNotEmpty() || from.isNotEmpty() || to.isNotEmpty() || search.isNotEmpty()

    companion object {
        val LIST_KEYS = listOf("account", "symbol", "grade", "tag", "kind", "exchange", "side", "result")
        val RANGE_KEYS = listOf("price", "hold", "pnl", "qty")

        /** clean_filters: a loose map (JSON) into a full filter set. */
        fun clean(raw: Map<String, Any?>?): Filters {
            val f = Filters()
            if (raw == null) return f
            fun str(v: Any?): String = if (v == null) "" else v.toString()
            (raw["lists"] as? Map<*, *>)?.let { lists ->
                for (k in LIST_KEYS) {
                    (lists[k] as? List<*>)?.let { vals -> f.lists[k] = vals.map { str(it) }.filter { it.isNotEmpty() } }
                }
            }
            (raw["ranges"] as? Map<*, *>)?.let { ranges ->
                for (k in RANGE_KEYS) {
                    (ranges[k] as? Map<*, *>)?.let { r ->
                        val op = str(r["op"])
                        f.ranges[k]!!.op = if (op == ">" || op == "<") op else ">"
                        val v = r["v"]
                        f.ranges[k]!!.v = when (v) {
                            null -> null
                            is Number -> v.toDouble()
                            else -> str(v).toDoubleOrNull()
                        }
                    }
                }
            }
            val preset = str(raw["preset"]).lowercase()
            f.preset = if (Model.PRESET_DAYS.containsKey(preset) || preset == "ytd" || preset == "all") preset else "all"
            (raw["years"] as? List<*>)?.let { ys -> f.years = ys.map { str(it).take(4) }.filter { Regex("^\\d{4}$").matches(it) }.toSet().sorted() }
            for (k in listOf("from", "to")) {
                val v = str(raw[k]).take(10)
                val ok = if (Regex("^\\d{4}-\\d{2}-\\d{2}$").matches(v)) v else ""
                if (k == "from") f.from = ok else f.to = ok
            }
            f.search = str(raw["search"]).trim()
            val b = str(raw["benchmark"]).trim().uppercase()
            f.benchmark = if (Model.BENCHMARK_LABELS.containsKey(b)) b else "SP500"
            return f
        }
    }
}

data class YearRow(val year: String, val r: Double, val days: Int, val from: String, val to: String, val flow: Double?, val endV: Double?, val spR: Double?)
data class Annualized(val rate: Double?, val years: Double, val count: Int, val first: String, val last: String)
data class Drawdown(val pct: Double?, val abs: Double?, val at: String, val peakAt: String)
class MonthBucket(val key: String, val label: String) { var value = 0.0; var count = 0; val tradeIds = mutableListOf<String>() }
data class SymbolRow(val symbol: String, val pnl: Double, val n: Int, val legs: Int, val winRate: Double, val avgHold: Double, val tradeIds: List<String>)
data class GradeBucket(val grade: String, val n: Int, val pnl: Double, val tradeIds: List<String>)
data class Grades(val buckets: List<GradeBucket>, val ungraded: Int, val graded: Int)
data class QueueRow(val id: String, val symbol: String, val date: String, val pnl: Double, val currency: String, val missing: String)
data class PositionsSummary(val count: Int, val book: Double, val mv: Double, val unreal: Double)
data class Options(
    val accounts: List<String>, val symbols: List<String>, val tags: List<String>, val exchanges: List<String>, val kinds: List<String>,
    val grades: List<String>, val sides: List<String>, val results: List<String>, val years: List<String>,
)
data class EquityView(val label: String, val series: List<EquityPoint>, val drawdown: Drawdown, val annualized: Annualized)

/** Everything a page shows for one filter set. */
data class View(
    val today: String, val filters: Filters, val options: Options, val kpi: KPI, val equity: EquityView, val years: List<YearRow>,
    val benchmarkKey: String, val benchmarkLabel: String, val monthly: List<MonthBucket>, val bySymbol: List<SymbolRow>, val grades: Grades,
    val queue: List<QueueRow>, val trades: List<Trade>, val tradeTotal: Int, val positions: List<Position>, val positionsSummary: PositionsSummary,
    val portfolio: Portfolio, val cashflow: CashflowView, val unmatched: List<Unmatched>,
)

object ModelView {
    // MARK: NAV: equity series, yearly time-weighted returns, drawdown

    fun equitySeries(points: List<NavPoint>): List<EquityPoint> {
        val out = mutableListOf<EquityPoint>()
        for (p in points) {
            val d = p.date.take(10)
            val v = p.equity ?: continue
            if (d.isEmpty()) continue
            out.add(EquityPoint(d, v, p.netDeposits))
        }
        return out.sortedBy { it.d }
    }

    fun navOn(series: List<EquityPoint>, day: String): Double? {
        var v: Double? = null
        for (p in series) {
            if (p.d > day) break
            v = p.v
        }
        return v
    }

    fun depositsOn(series: List<EquityPoint>, day: String): Double? {
        var v: Double? = null
        for (p in series) {
            if (p.d > day) break
            if (p.dep != null) v = p.dep
        }
        return v
    }

    class YearReturn(val r: Double, val from: String, val to: String, val days: Int)

    /** Daily chain-linked return for one calendar year, net of deposits. */
    fun yearReturn(series: List<EquityPoint>, year: String, today: String): YearReturn? {
        val cal = "$year-01-01"
        val to = minOf("$year-12-31", today)
        if (series.isEmpty()) return null
        // A balance under 1% of the account's peak is pre-history (a few dollars
        // parked before the real start): a chain that began there would turn the
        // first big deposit into a wild return, so the chain starts at the first
        // point that clears the floor, and the year is measured from there.
        val floor = series.maxOf { it.v } * 0.01
        val startDay = Model.shiftDate(cal, -1)
        var start = navOn(series, startDay)
        var after = startDay
        if (!(start != null && start != 0.0 && start > floor)) {
            val first = series.firstOrNull { cal <= it.d && it.d <= to && it.v > floor } ?: return null
            start = first.v
            after = first.d
        }
        val pts = series.filter { after < it.d && it.d <= to }
        if (pts.isEmpty()) return null
        var prevEq = start!!
        var prevDep = depositsOn(series, after)
        var factor = 1.0
        for (p in pts) {
            val eq = p.v
            if (!(prevEq > 0)) return null
            var cf = 0.0
            val pd = prevDep
            if (p.dep != null && pd != null) cf = p.dep - pd
            factor *= 1 + (eq - prevEq - cf) / prevEq
            prevEq = eq
            if (p.dep != null) prevDep = p.dep
        }
        val r = factor - 1
        if (r.isNaN() || r.isInfinite()) return null
        val spanFrom = if (after == startDay) cal else after
        return YearReturn(r, spanFrom, to, Model.daysBetween(spanFrom, to))
    }

    /** The index over the same span the account's year covers: the calendar year,
     * or from `start` when the account was funded part way through it. */
    fun benchmarkReturn(bench: Map<String, Double>, year: String, today: String, start: String? = null): Double? {
        if (bench.isEmpty()) return null
        val days = bench.keys.sorted()
        val s = (start ?: "").take(10)
        val cal = if (s.isEmpty()) "$year-01-01" else s
        val to = minOf("$year-12-31", today)
        var prev: Double? = null
        var end: Double? = null
        for (d in days) {
            if (d < cal) prev = bench[d] else if (d <= to) end = bench[d]
        }
        if (prev == null) {
            val firsts = days.filter { cal <= it && it <= to }
            if (firsts.isEmpty()) return null
            prev = bench[firsts[0]]
        }
        val p = prev ?: return null
        val e = end ?: return null
        if (p == 0.0) return null
        return e / p - 1
    }

    fun yearlyReturns(series: List<EquityPoint>, bench: Map<String, Double>, today: String): List<YearRow> {
        if (series.isEmpty()) return emptyList()
        val years = series.map { it.d.take(4) }.toSet().sorted()
        val peak = series.maxOf { it.v }
        val out = mutableListOf<YearRow>()
        for (y in years) {
            // a year in which the account never held more than 1% of its peak is
            // pre-history (a few hundred dollars parked before the real start)
            val yearPeak = series.filter { it.d.take(4) == y }.maxOfOrNull { it.v } ?: 0.0
            if (peak > 0 && yearPeak < peak * 0.01) continue
            val yr = yearReturn(series, y, today) ?: continue
            val startDep = depositsOn(series, Model.shiftDate("$y-01-01", -1))
            val endDep = depositsOn(series, yr.to)
            val flow = if (startDep != null && endDep != null) endDep - startDep else null
            out.add(YearRow(y, yr.r, yr.days, yr.from, yr.to, flow, navOn(series, yr.to),
                benchmarkReturn(bench, y, today, if (yr.from != "$y-01-01") yr.from else null)))
        }
        return out
    }

    fun annualized(years: List<YearRow>): Annualized {
        var prod = 1.0
        var days = 0
        val used = mutableListOf<String>()
        for (y in years) {
            if (y.r <= -1 || y.days < 30) continue
            prod *= 1 + y.r
            days += y.days
            used.add(y.year)
        }
        if (days == 0) return Annualized(null, 0.0, 0, "", "")
        val yrs = days / 365.25
        val rate = if (yrs >= 1.0 / 12.0) prod.pow(1 / yrs) - 1 else prod - 1
        return Annualized(rate, yrs, used.size, used.first(), used.last())
    }

    /** Net deposit change per day, moved one day later when the equity
     * series only reflects the money a day after the deposit record does. */
    fun pairedFlows(series: List<EquityPoint>): DoubleArray {
        val n = series.size
        val flows = DoubleArray(n)
        for (i in 1 until n) {
            val p = series[i]; val prev = series[i - 1]
            val dep = p.dep ?: continue
            val pd = prev.dep ?: continue
            val cf = dep - pd
            if (abs(cf) < Model.EPS) continue
            val changeToday = p.v - prev.v
            if (i + 1 < n) {
                val changeNext = series[i + 1].v - p.v
                if (abs(changeToday - cf) > abs(changeNext - cf) && abs(changeToday) < abs(cf) * 0.5) {
                    flows[i + 1] += cf
                    continue
                }
            }
            flows[i] += cf
        }
        return flows
    }

    /** Max drawdown of the flow-adjusted equity: daily returns are taken net
     * of deposits and withdrawals and chain-linked into an index, so money
     * moved in or out of the account is not counted as a gain or a loss. */
    fun drawdown(series: List<EquityPoint>): Drawdown {
        if (series.isEmpty()) return Drawdown(null, null, "", "")
        val peakV = series.maxOf { it.v }
        val floor = peakV * 0.01
        var idx = 1.0
        var prev: EquityPoint? = null
        var peakIdx = 0.0
        var peakAt = ""
        var peakEquity = 0.0
        var dd = 0.0; var ddAbs = 0.0
        var ddAt = ""; var ddPeakAt = ""
        val flows = pairedFlows(series)
        for ((i, p) in series.withIndex()) {
            val pr = prev
            if (pr != null && pr.v > floor && pr.v > 0) idx *= 1 + (p.v - pr.v - flows[i]) / pr.v
            prev = p
            if (p.v < floor) continue
            if (idx >= peakIdx) {
                peakIdx = idx
                peakAt = p.d
                peakEquity = p.v
            }
            if (peakIdx <= 0) continue
            val drop = idx / peakIdx - 1
            if (drop < dd) {
                dd = drop
                ddAbs = drop * peakEquity
                ddAt = p.d
                ddPeakAt = peakAt
            }
        }
        return Drawdown(dd, ddAbs, ddAt, ddPeakAt)
    }

    // MARK: filters

    fun dateBounds(f: Filters, today: String): Pair<String, String>? {
        if (f.from.isNotEmpty() || f.to.isNotEmpty()) return Pair(f.from.ifEmpty { "0000-01-01" }, f.to.ifEmpty { "9999-12-31" })
        if (f.years.isNotEmpty()) return null
        if (f.preset == "ytd") return Pair(today.take(4) + "-01-01", today)
        val days = Model.PRESET_DAYS[f.preset]
        if (days != null) return Pair(Model.shiftDate(today, -days), today)
        return null
    }

    fun inDateScope(f: Filters, today: String, day: String): Boolean {
        val b = dateBounds(f, today)
        if (b != null) return b.first <= day && day <= b.second
        if (f.years.isNotEmpty()) return f.years.contains(day.take(4))
        return true
    }

    fun tradeMatches(t: Trade, f: Filters, today: String): Boolean {
        val s = f.search.uppercase()
        if (s.isNotEmpty() && !t.symbol.uppercase().contains(s) && !t.underlying.uppercase().contains(s) && !t.name.uppercase().contains(s)) return false
        val L = f.lists
        L["account"]!!.let { if (it.isNotEmpty() && !it.contains(t.account)) return false }
        L["symbol"]!!.let { if (it.isNotEmpty() && !it.contains(t.symbol) && !it.contains(t.underlying)) return false }
        L["grade"]!!.let { if (it.isNotEmpty() && !it.contains(t.grade.ifEmpty { "Ungraded" })) return false }
        L["tag"]!!.let { tg ->
            if (tg.isNotEmpty()) {
                val tags = t.tags.ifEmpty { listOf("untagged") }
                if (tags.none { tg.contains(it) }) return false
            }
        }
        L["kind"]!!.let { if (it.isNotEmpty() && !it.contains(t.kind)) return false }
        L["exchange"]!!.let { if (it.isNotEmpty() && !it.contains(t.exchange)) return false }
        L["side"]!!.let { if (it.isNotEmpty() && !it.contains(t.side)) return false }
        L["result"]!!.let {
            if (it.isNotEmpty()) {
                val res = if (t.pnlCad > 0) "Winners" else if (t.pnlCad < 0) "Losers" else "Breakeven"
                if (!it.contains(res)) return false
            }
        }
        for ((key, v) in listOf("price" to t.entry, "hold" to t.holdDays.toDouble(), "pnl" to t.pnlCad, "qty" to t.qty)) {
            val r = f.ranges[key]!!
            val rv = r.v ?: continue
            if (r.op == ">" && !(v > rv)) return false
            if (r.op == "<" && !(v < rv)) return false
        }
        return inDateScope(f, today, t.exitDate)
    }

    fun positionMatches(p: Position, f: Filters): Boolean {
        val s = f.search.uppercase()
        if (s.isNotEmpty() && !p.symbol.uppercase().contains(s) && !p.name.uppercase().contains(s)) return false
        val L = f.lists
        L["account"]!!.let { if (it.isNotEmpty() && !it.contains(p.account)) return false }
        L["symbol"]!!.let { if (it.isNotEmpty() && !it.contains(p.symbol) && !it.contains(p.underlying)) return false }
        L["kind"]!!.let { if (it.isNotEmpty() && !it.contains(p.kind)) return false }
        L["exchange"]!!.let { if (it.isNotEmpty() && !it.contains(p.exchange)) return false }
        return true
    }

    // MARK: the dashboard cards

    fun bySymbol(trades: List<Trade>): List<SymbolRow> {
        class G { var pnl = 0.0; var n = 0; var wins = 0; var hold = 0; var legs = 0; val ids = mutableListOf<String>() }
        val by = LinkedHashMap<String, G>()
        for (t in trades) {
            val g = by.getOrPut(t.underlying) { G() }
            g.pnl += t.pnlCad
            g.n += 1
            g.legs += t.legCount
            g.hold += t.holdDays
            g.ids.add(t.id)
            if (t.pnlCad > 0) g.wins += 1
        }
        return by.entries.map { (k, g) ->
            SymbolRow(k, g.pnl, g.n, g.legs, if (g.n > 0) g.wins.toDouble() / g.n else 0.0, if (g.n > 0) g.hold.toDouble() / g.n else 0.0, g.ids)
        }.sortedByDescending { it.pnl }
    }

    fun monthly(trades: List<Trade>): List<MonthBucket> {
        val by = HashMap<String, MonthBucket>()
        for (t in trades) {
            val k = t.exitDate.take(7)
            if (k.length < 7) continue
            val b = by.getOrPut(k) { MonthBucket(k, Model.monthLabel(k)) }
            b.value += t.pnlCad
            b.count += 1
            b.tradeIds.add(t.id)
        }
        return by.keys.sorted().map { by[it]!! }
    }

    fun gradeBuckets(trades: List<Trade>): Grades {
        val buckets = Model.GRADES.map { g ->
            val rows = trades.filter { it.grade == g }
            GradeBucket(g, rows.size, rows.sumOf { it.pnlCad }, rows.map { it.id })
        }
        val ungraded = trades.count { it.grade.isEmpty() }
        return Grades(buckets, ungraded, trades.size - ungraded)
    }

    fun reviewQueue(trades: List<Trade>): List<QueueRow> {
        val out = mutableListOf<QueueRow>()
        for (t in trades) {
            val noGrade = t.grade.isEmpty()
            val noThesis = t.thesis.trim().isEmpty()
            if (!(noGrade || noThesis)) continue
            out.add(QueueRow(t.id, t.symbol, t.exitDate, t.pnlCad, "CAD",
                if (noGrade && noThesis) "no grade or thesis" else if (noGrade) "no grade" else "no thesis"))
        }
        return out.sortedByDescending { it.date }
    }

    // MARK: the Cashflow page for one filter set

    private class Rate(val per: Double, val freq: Int, val annual: Double, val verified: Boolean, val source: String)

    fun cashflowView(base: Base, f: Filters, positionsAll: List<Position>, marginUsed: Double = 0.0, hasMargin: Boolean = true): CashflowView {
        val today = base.today
        val accts = f.lists["account"]!!
        val symbolsF = f.lists["symbol"]!!
        val search = f.search.uppercase()

        fun inScope(r: CashRow): Boolean {
            if (accts.isNotEmpty() && !accts.contains(r.account)) return false
            if (search.isNotEmpty() && !r.symbol.uppercase().contains(search)) return false
            if (symbolsF.isNotEmpty() && !symbolsF.contains(r.symbol)) return false
            return inDateScope(f, today, r.date)
        }

        val everything = base.cashflow.filter { inScope(it) }
        val recs = everything.filter { it.kind == "Dividend" }
        val skipped = listOf("grade", "tag", "kind", "exchange", "side", "result").filter { f.lists[it]!!.isNotEmpty() } +
            Filters.RANGE_KEYS.filter { f.ranges[it]!!.v != null }

        val keys = mutableListOf<String>()
        val bucket = HashMap<String, DoubleArray>()   // [sum, n]
        if (recs.isNotEmpty()) {
            val monthsSeen = recs.map { it.date.take(7) }.toSet().sorted()
            val first = monthsSeen.first()
            var last = monthsSeen.last()
            // the chart runs to the current month (or the end of the date filter), with
            // an empty bar for a month that has not paid yet
            var endDay = today
            val bounds = dateBounds(f, today)
            if (bounds != null) endDay = minOf(bounds.second, today)
            else if (f.years.isNotEmpty()) endDay = minOf(f.years.max() + "-12-31", today)
            last = maxOf(last, endDay.take(7))
            var y = first.substring(0, 4).toInt()
            var m = first.substring(5, 7).toInt()
            while (true) {
                val k = String.format(Locale.ROOT, "%04d-%02d", y, m)
                if (k > last) break
                keys.add(k)
                bucket[k] = doubleArrayOf(0.0, 0.0)
                m += 1
                if (m > 12) { m = 1; y += 1 }
            }
        }
        for (r in recs) {
            val b = bucket[r.date.take(7)] ?: continue
            b[0] += r.amountCad
            b[1] += 1
        }
        val months = keys.map { MonthBar(it, Model.monthLabel(it), bucket[it]!![0], bucket[it]!![1].toInt()) }

        val payers = base.cashflow.filter { it.kind == "Dividend" }.map { it.symbol }.toSet()
        val held = positionsAll.filter { payers.contains(it.symbol) && !it.short }
            .filter { (accts.isEmpty() || accts.contains(it.account)) && (search.isEmpty() || it.symbol.uppercase().contains(search)) }
        val forYoc = base.cashflow.filter { it.kind == "Dividend" && (accts.isEmpty() || accts.contains(it.account)) && (search.isEmpty() || it.symbol.uppercase().contains(search)) }
        val lastRec = recs.firstOrNull()?.date ?: today
        var cm = lastRec.substring(5, 7).toInt() - 11
        var cy = lastRec.substring(0, 4).toInt()
        while (cm <= 0) { cm += 12; cy -= 1 }
        val cut = String.format(Locale.ROOT, "%04d-%02d", cy, cm)
        val thisYear = today.take(4)

        fun sumFor(sym: String, pred: (CashRow) -> Boolean) = forYoc.filter { it.symbol == sym && pred(it) }.sumOf { it.amountCad }

        val pub = base.distributions
        val quotes = base.quotes

        fun rateFor(sym: String): Rate? {
            // Preferred: the fund's own declared record (TMX Money): the latest
            // distribution that has gone ex, and payments per year from the gaps
            // between its recent ex-dates, so a schedule change shows at once.
            val declared = (pub[sym] ?: emptyList()).filter { it.exDate <= today }.sortedByDescending { it.exDate }
            if (declared.isNotEmpty()) {
                val per = declared[0].amount
                val freq = Model.paymentsPerYear((pub[sym] ?: emptyList()).map { it.exDate })
                if (per != 0.0 && freq != null) return Rate(per, freq, per * freq, true, "declared")
            }
            // Otherwise this holding's own payment rows.
            val rs = forYoc.filter { it.symbol == sym && (it.per ?: 0.0) != 0.0 }.sortedByDescending { it.date }
            val per = rs.firstOrNull()?.per ?: return null
            if (per == 0.0) return null
            val freq = Model.paymentsPerYear(forYoc.filter { it.symbol == sym }.map { it.date })
            val verified = freq != null
            val fq = freq ?: 12
            return Rate(per, fq, per * fq, verified, "payments")
        }

        /** (ex-date, pay date, ex passed, pay passed): the next distribution still
         * to be paid, whether or not it has gone ex, else the last known one. */
        fun distributionDates(sym: String): List<Any> {
            fun payOf(d: Distribution): String { val p = d.payDate.take(10); return if (p.isEmpty()) d.exDate else p }
            val recs2 = (pub[sym] ?: emptyList()).sortedWith(compareBy({ payOf(it) }, { it.exDate }))
            val unpaid = recs2.filter { payOf(it) >= today }
            val pick = unpaid.firstOrNull() ?: recs2.lastOrNull()
            val ex: String
            val pay: String
            if (pick != null) {
                ex = pick.exDate
                pay = pick.payDate.take(10)
            } else {
                ex = (quotes[sym]?.exDividendDate ?: "").take(10)
                val paid = forYoc.filter { it.symbol == sym }.map { it.date }.sorted()
                pay = paid.lastOrNull() ?: ""
            }
            return listOf(ex, pay, ex.isNotEmpty() && ex < today, pay.isNotEmpty() && pay < today)
        }

        fun lastPrice(p: Position): Pair<Double, String> {
            val px = quotes[p.symbol]?.price
            if (px != null && px > 0) return Pair(px, "close")
            return Pair(p.last, "fill")
        }

        val holdings = mutableListOf<Holding>()
        for (p in held) {
            val r = rateFor(p.symbol)
            val basis = p.cost
            val avg = p.avg
            val (lastPx, priceSource) = lastPrice(p)
            val dd = distributionDates(p.symbol)
            val h = Holding()
            h.id = p.id
            h.symbol = p.symbol
            h.account = p.account
            h.qty = p.qty
            h.per = r?.per
            h.freq = r?.freq
            h.freqVerified = r?.verified ?: false
            h.rateSource = r?.source ?: ""
            h.cost = basis
            h.avg = avg
            h.last = lastPx
            h.priceSource = priceSource
            h.ytd = sumFor(p.symbol) { it.date.take(4) == thisYear }
            h.ttm = sumFor(p.symbol) { it.date.take(7) >= cut }
            h.all = sumFor(p.symbol) { true }
            h.nextExDate = dd[0] as String
            h.nextPayDate = dd[1] as String
            h.exPast = dd[2] as Boolean
            h.payPast = dd[3] as Boolean
            h.yob = r?.let { it.per * p.qty }
            h.annual = r?.let { it.annual * p.qty }
            h.yoc = if (r != null && avg != 0.0) r.annual / avg else null
            h.currentYield = if (r != null && lastPx != 0.0) r.annual / lastPx else null
            holdings.add(h)
        }
        val verified = holdings.filter { it.annual != null }
        val basisAll = verified.sumOf { it.cost }
        val earnedAll = verified.sumOf { it.ttm }
        val annualAll = verified.sumOf { it.annual!! }
        val total = recs.sumOf { it.amountCad }
        val thisYr = thisYear.toInt()
        val tiles = mutableListOf<Tile>()
        for (y in listOf(thisYr - 2, thisYr - 1, thisYr)) {
            val ys = y.toString()
            val rs = recs.filter { it.date.take(4) == ys }
            val sm = rs.sumOf { it.amountCad }
            var paid = keys.count { it.take(4) == ys && bucket[it]!![1] > 0.0 }
            if (paid == 0) paid = 1
            tiles.add(Tile(label = if (y == thisYr) "$y YTD" else ys, total = sm, perMonth = sm / paid, count = rs.size))
        }
        var monthsInScope = keys.count { bucket[it]!![1] > 0.0 }
        if (monthsInScope == 0) monthsInScope = 1
        tiles.add(Tile(label = "All time", total = total, perMonth = total / monthsInScope, count = recs.size))
        if (hasMargin) {
            // margin used is the Portfolio tab's figure; under it the average margin interest per charged month
            val charges = everything.filter { it.kind == "Interest charge" }
            val chargeMonths = charges.map { it.date.take(7) }.toSet().size
            val charged = charges.sumOf { -it.amountCad }
            tiles.add(Tile(label = "Margin used", marginUsed = marginUsed, interestPerMonth = if (chargeMonths > 0) charged / chargeMonths else 0.0, interestMonths = chargeMonths))
        } else {
            // without a margin account: the trailing twelve months, averaged over the months that paid
            val since = Model.shiftDate(today, -365)
            val window = recs.filter { it.date > since && it.date <= today }
            val sm = window.sumOf { it.amountCad }
            val paid = maxOf(1, window.map { it.date.take(7) }.toSet().size)
            tiles.add(Tile(label = "Last 12 months", total = sm, perMonth = sm / paid, count = window.size))
        }
        tiles.add(Tile(label = "Yield on cost", yield = if (basisAll != 0.0) annualAll / basisAll else null, projected = annualAll / 12, earned = earnedAll, book = basisAll))
        val other = everything.filter { it.kind != "Dividend" }
        return CashflowView(
            tiles = tiles, months = months, holdings = holdings, rows = recs, other = other, total = total, count = recs.size,
            interest = other.filter { it.kind == "Interest" }.sumOf { it.amountCad },
            withholding = other.filter { it.kind == "Withholding tax" }.sumOf { it.amountCad },
            skippedFilters = skipped,
        )
    }

    // MARK: build_view

    fun buildView(base: Base, f: Filters): View {
        val today = base.today
        val tradesAll = base.trades
        val trades = tradesAll.filter { tradeMatches(it, f, today) }
        val positionsAll = base.positions
        val positions = positionsAll.filter { positionMatches(it, f) }

        val accts = f.lists["account"]!!
        var series = base.equity
        var seriesLabel = "All accounts"
        if (accts.size == 1) {
            base.equityByAccount[accts[0]]?.let { series = it; seriesLabel = accts[0] }
        }
        val benchKey = f.benchmark
        val years = yearlyReturns(series, base.benchmarks[benchKey] ?: emptyMap(), today)
        val ann = annualized(years)
        val dd = drawdown(series)
        var shown = series
        val bounds = dateBounds(f, today)
        if (bounds != null) shown = series.filter { bounds.first <= it.d && it.d <= bounds.second }
        else if (f.years.isNotEmpty()) shown = series.filter { f.years.contains(it.d.take(4)) }
        if (shown.isNotEmpty()) {
            val peak = shown.maxOf { it.v }
            val firstIdx = shown.indexOfFirst { it.v > peak * 0.01 }.let { if (it < 0) 0 else it }
            shown = shown.drop(firstIdx)
        }

        val options = Options(
            accounts = (tradesAll.map { it.account } + positionsAll.map { it.account } + base.cashflow.map { it.account }).toSet().sorted(),
            symbols = (tradesAll.map { it.symbol } + positionsAll.map { it.symbol }).toSet().sorted(),
            tags = tradesAll.flatMap { it.tags }.toSet().sorted(),
            exchanges = (tradesAll.map { it.exchange } + positionsAll.map { it.exchange }).filter { it.isNotEmpty() }.toSet().sorted(),
            kinds = Model.KINDS.filter { k -> tradesAll.any { it.kind == k } || positionsAll.any { it.kind == k } },
            grades = Model.GRADES + "Ungraded",
            sides = listOf("SELL", "COVER"),
            results = listOf("Winners", "Losers", "Breakeven"),
            years = tradesAll.filter { it.exitDate.isNotEmpty() }.map { it.exitDate.take(4) }.toSet().sorted().reversed(),
        )
        val portfolio = portfolioView(base, f, positions)
        return View(
            today = today, filters = f, options = options, kpi = Model.kpi(trades),
            equity = EquityView(seriesLabel, shown, dd, ann), years = years,
            benchmarkKey = benchKey, benchmarkLabel = Model.BENCHMARK_LABELS[benchKey] ?: "S&P 500",
            monthly = monthly(trades), bySymbol = bySymbol(trades), grades = gradeBuckets(trades), queue = reviewQueue(trades),
            trades = trades, tradeTotal = tradesAll.size, positions = positions,
            positionsSummary = PositionsSummary(positions.size, positions.sumOf { abs(it.cost) }, positions.sumOf { if (it.short) -it.mv else it.mv }, positions.sumOf { it.unreal }),
            portfolio = portfolio,
            cashflow = cashflowView(base, f, positionsAll, portfolio.marginUsed, portfolio.hasMargin), unmatched = base.unmatched,
        )
    }

    /** crates/model/src/view.rs portfolio_view: CAD aggregates over the accounts the filter has on, every account when it has none. */
    fun portfolioView(base: Base, f: Filters, positions: List<Position>): Portfolio {
        val fx = base.fx; val today = base.today
        fun cad(amount: Double, currency: String) = Model.toCad(fx, amount, currency, today)
        val names = f.lists["account"] ?: emptyList()
        // closed accounts hold nothing and count for nothing here
        val accounts = base.accounts.filter { it.status.lowercase() != "closed" && (names.isEmpty() || names.contains(it.name)) }
        val ids = accounts.map { it.id }.toSet()
        val nameOf = accounts.associate { it.id to it.name }
        val out = Portfolio()
        out.marketValue = positions.sumOf { cad(if (it.short) -it.mv else it.mv, it.currency) }
        out.costBasis = positions.sumOf { cad(abs(it.cost), it.currency) }
        out.unrealized = positions.sumOf { cad(it.unreal, it.currency) }
        out.unrealizedPct = if (out.costBasis != 0.0) out.unrealized / out.costBasis else null
        out.positionCount = positions.size
        out.accountCount = positions.map { it.account }.toSet().size
        val navs = accounts.mapNotNull { a -> a.nav?.let { cad(it, a.currency) } }
        out.nav = if (navs.isEmpty()) null else navs.sum()
        out.navAccounts = navs.size
        val used = LinkedHashMap<String, Double>()
        for (b in base.balances) {
            val ccy = base.cashCurrencies[b.securityId] ?: continue
            if (b.accountId in ids && b.quantity < 0) used[ccy] = (used[ccy] ?: 0.0) + (-b.quantity)
        }
        out.marginUsed = used.entries.sumOf { cad(it.value, it.key) }
        // the positive cash balances, the other side of the same rows
        val cashBy = LinkedHashMap<String, Double>()
        for (b in base.balances) {
            val ccy = base.cashCurrencies[b.securityId] ?: continue
            if (b.accountId in ids && b.quantity > 0) cashBy[ccy] = (cashBy[ccy] ?: 0.0) + b.quantity
        }
        out.cash = cashBy.entries.sumOf { cad(it.value, it.key) }
        out.cashPct = out.nav?.let { if (it != 0.0) out.cash / it else null }
        // the day's change: each quoted position's, summed, over what those positions were worth at the previous close
        val quoted = positions.filter { it.dayChange != null }
        if (quoted.isNotEmpty()) {
            val dc = quoted.sumOf { cad(it.dayChange!!, it.currency) }
            val prev = quoted.sumOf { cad(if (it.short) -it.mv else it.mv, it.currency) } - dc
            out.dayChange = dc
            out.dayChangePct = if (prev != 0.0) dc / prev else null
        }
        out.marginUsedBy = used.entries.sortedBy { it.key }.associate { it.key to Math.round(it.value * 100) / 100.0 }
        out.marginUsedPct = if (out.marketValue != 0.0) out.marginUsed / out.marketValue else null
        val avail = mutableListOf<Double>()
        val unavailable = mutableListOf<String>()
        // only a margin account's buying power is margin available; any other row is cash to buy with
        val marginIds = accounts.filter { it.type.uppercase().contains("MARGIN") }.map { it.id }.toSet()
        out.hasMargin = marginIds.isNotEmpty()
        for (m in base.margin) {
            if (m.accountId !in marginIds) continue
            val bp = m.buyingPower
            if (bp != null) avail.add(cad(bp, m.currency.ifEmpty { "CAD" })) else unavailable.add(nameOf[m.accountId] ?: m.accountId)
        }
        out.availableMargin = if (avail.isEmpty()) null else avail.sum()
        out.availableMarginUnavailable = unavailable.sorted()
        val alloc = positions.mapNotNull { p -> val v = cad(p.mv, p.currency); if (v > 0) AllocationRow(p.id, p.symbol, p.account, v) else null }.sortedByDescending { it.value }
        val total = alloc.sumOf { it.value }
        for (a in alloc) a.share = if (total != 0.0) a.value / total else 0.0
        out.allocation = alloc
        return out
    }
}
