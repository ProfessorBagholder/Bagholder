package com.bagholder.model

import org.json.JSONArray
import org.json.JSONObject
import java.io.File
import kotlin.math.abs
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertTrue

/** The shared model cases in ../../tests/cases, run through the Kotlin model.
 * The same files run through the Rust (crates/model/tests/cases.rs) and Swift
 * (ModelCasesTests) models; a rule changed in one place fails here.
 * tests/README.md describes the format: `expect` is the view for the
 * case's filters, floats rounded to six places. */
class ModelCasesTest {
    private val casesDir = File("../../tests/cases")

    private fun str(d: JSONObject, k: String) = d.optString(k, "")
    private fun num(d: JSONObject, k: String) = if (d.has(k) && !d.isNull(k)) d.optDouble(k, 0.0) else 0.0
    private fun numOrNull(d: JSONObject, k: String): Double? = if (d.has(k) && !d.isNull(k)) d.optDouble(k) else null

    private fun activity(d: JSONObject): Act {
        val a = Act()
        a.id = str(d, "id"); a.occurredAt = str(d, "occurredAt"); a.transactionDate = str(d, "transactionDate")
        a.accountId = str(d, "accountId"); a.fifoId = str(d, "fifoId"); a.accountType = str(d, "accountType")
        a.activityType = str(d, "activityType"); a.activitySubType = str(d, "activitySubType")
        a.description = str(d, "description"); a.direction = str(d, "direction"); a.symbol = str(d, "symbol")
        a.name = str(d, "name"); a.currency = str(d, "currency")
        a.quantity = num(d, "quantity"); a.unitPrice = num(d, "unitPrice"); a.commission = num(d, "commission")
        a.netCashAmount = num(d, "netCashAmount")
        a.category = str(d, "category"); a.rawType = str(d, "rawType"); a.aftType = str(d, "aftType")
        a.securityId = str(d, "securityId")
        return a
    }

    private fun security(d: JSONObject) = Security(
        id = str(d, "id"), symbol = str(d, "symbol"), name = str(d, "name"), underlyingId = str(d, "underlyingId"),
        primaryExchange = str(d, "primaryExchange"), primaryMic = str(d, "primaryMic"), currency = str(d, "currency"),
    )

    private fun navPoint(d: JSONObject) = NavPoint(str(d, "date"), numOrNull(d, "equity"), numOrNull(d, "netDeposits"))

    private fun closes(o: JSONObject?): Map<String, Double> {
        val out = HashMap<String, Double>()
        if (o != null) for (k in o.keys()) out[k] = o.getDouble(k)
        return out
    }

    private fun market(d: JSONObject): Market {
        val benchmarks = HashMap<String, Map<String, Double>>()
        d.optJSONObject("benchmarks")?.let { o -> for (k in o.keys()) benchmarks[k] = closes(o.getJSONObject(k)) }
        val dists = HashMap<String, List<Distribution>>()
        d.optJSONObject("distributions")?.let { o ->
            for (sym in o.keys()) {
                val rows = o.getJSONArray(sym)
                dists[sym] = (0 until rows.length()).map { i ->
                    val r = rows.getJSONObject(i)
                    Distribution(str(r, "exDate"), str(r, "payDate"), num(r, "amount"), str(r, "currency"))
                }
            }
        }
        val quotes = HashMap<String, Quote>()
        d.optJSONObject("quotes")?.let { o ->
            for (sym in o.keys()) {
                val q = o.getJSONObject(sym)
                quotes[sym] = Quote(numOrNull(q, "price"), numOrNull(q, "priceChange"), numOrNull(q, "percentChange"), str(q, "fetchedAt"), str(q, "exDividendDate"))
            }
        }
        return Market(closes(d.optJSONObject("fx")), dists, quotes, closes(d.optJSONObject("benchmark")), benchmarks)
    }

    private fun journal(d: JSONObject?): Map<String, JournalEntry> {
        val out = HashMap<String, JournalEntry>()
        if (d != null) for (k in d.keys()) {
            val e = d.getJSONObject(k)
            val tags = e.optJSONArray("tags") ?: JSONArray()
            out[k] = JournalEntry(str(e, "grade"), str(e, "thesis"), (0 until tags.length()).map { tags.getString(it) })
        }
        return out
    }

    /** JSON filters as the loose map Filters.clean reads. */
    private fun loose(v: Any?): Any? = when (v) {
        is JSONObject -> v.keys().asSequence().associateWith { loose(v.get(it)) }
        is JSONArray -> (0 until v.length()).map { loose(v.get(it)) }
        JSONObject.NULL -> null
        else -> v
    }

    // What the Kotlin model produces, in the fixture's shape.

    private fun opt(v: Double?): Any = v ?: JSONObject.NULL

    private fun expect(v: View): Map<String, Any> {
        val trades = v.trades.sortedWith(compareBy({ it.entryDate }, { it.exitDate }, { it.symbol }))
        val k = v.kpi
        val cf = v.cashflow
        val out = LinkedHashMap<String, Any>()
        out["kpi"] = mapOf(
            "count" to k.count, "wins" to k.wins, "losses" to k.losses, "breakeven" to k.breakeven, "winRate" to opt(k.winRate), "realized" to k.realized,
            "expectancy" to opt(k.expectancy), "profitFactor" to opt(k.profitFactor), "avgHold" to opt(k.avgHold),
            "avgWin" to k.avgWin, "avgLoss" to k.avgLoss, "grossWin" to k.grossWin, "grossLoss" to k.grossLoss,
        )
        out["trades"] = trades.map { t ->
            mapOf(
                "id" to t.id, "symbol" to t.symbol, "kind" to t.kind, "currency" to t.currency, "side" to t.side, "qty" to t.qty, "mult" to t.mult,
                "entry" to t.entry, "exit" to t.exit, "entryDate" to t.entryDate, "exitDate" to t.exitDate, "holdDays" to t.holdDays,
                "pnl" to t.pnl, "pnlCad" to t.pnlCad, "pnlPct" to opt(t.pnlPct), "status" to t.status, "fees" to t.fees,
                "account" to t.account, "exchange" to t.exchange, "grade" to t.grade, "tags" to t.tags,
                "fills" to t.fills.sortedBy { it.whenAt }.map { it.sub },
            )
        }
        out["positions"] = v.positions.sortedWith(compareBy({ it.symbol }, { it.account })).map { p ->
            mapOf("id" to p.id, "symbol" to p.symbol, "kind" to p.kind, "currency" to p.currency, "account" to p.account, "exchange" to p.exchange,
                "qty" to p.qty, "avg" to p.avg, "cost" to p.cost, "held" to p.held, "alloc" to p.alloc, "short" to p.short,
                "dayChange" to opt(p.dayChange), "grade" to p.grade, "fills" to p.fills.sortedBy { it.whenAt }.map { it.sub })
        }
        out["positionsSummary"] = mapOf("count" to v.positionsSummary.count, "book" to v.positionsSummary.book, "mv" to v.positionsSummary.mv, "unreal" to v.positionsSummary.unreal)
        val pf = v.portfolio
        out["portfolio"] = mapOf(
            "marketValue" to pf.marketValue, "costBasis" to pf.costBasis, "unrealized" to pf.unrealized, "unrealizedPct" to opt(pf.unrealizedPct),
            "positionCount" to pf.positionCount, "accountCount" to pf.accountCount, "nav" to opt(pf.nav), "navAccounts" to pf.navAccounts,
            "marginUsed" to pf.marginUsed, "marginUsedBy" to pf.marginUsedBy, "marginUsedPct" to opt(pf.marginUsedPct),
            "availableMargin" to opt(pf.availableMargin), "availableMarginUnavailable" to pf.availableMarginUnavailable,
            "hasMargin" to pf.hasMargin, "cash" to pf.cash, "cashPct" to opt(pf.cashPct), "dayChange" to opt(pf.dayChange), "dayChangePct" to opt(pf.dayChangePct),
            "allocation" to pf.allocation.map { mapOf("id" to it.id, "symbol" to it.symbol, "account" to it.account, "value" to it.value, "share" to it.share) },
        )
        out["equity"] = mapOf(
            "label" to v.equity.label,
            "series" to v.equity.series.map { mapOf("d" to it.d, "v" to it.v) },
            "drawdown" to mapOf("pct" to opt(v.equity.drawdown.pct), "abs" to opt(v.equity.drawdown.abs), "at" to v.equity.drawdown.at, "peakAt" to v.equity.drawdown.peakAt),
            "annualized" to mapOf("rate" to opt(v.equity.annualized.rate), "years" to v.equity.annualized.years, "count" to v.equity.annualized.count,
                "first" to v.equity.annualized.first, "last" to v.equity.annualized.last),
        )
        out["years"] = v.years.map { y -> mapOf("year" to y.year, "r" to y.r, "days" to y.days, "from" to y.from, "to" to y.to, "flow" to opt(y.flow), "endV" to opt(y.endV), "spR" to opt(y.spR)) }
        out["benchmark"] = mapOf("key" to v.benchmarkKey, "label" to v.benchmarkLabel)
        out["monthly"] = v.monthly.map { mapOf("key" to it.key, "label" to it.label, "value" to it.value, "count" to it.count) }
        out["bySymbol"] = v.bySymbol.map { mapOf("symbol" to it.symbol, "pnl" to it.pnl, "n" to it.n, "legs" to it.legs, "winRate" to it.winRate, "avgHold" to it.avgHold) }
        out["grades"] = mapOf("buckets" to v.grades.buckets.map { mapOf("grade" to it.grade, "n" to it.n, "pnl" to it.pnl) }, "ungraded" to v.grades.ungraded, "graded" to v.grades.graded)
        out["queue"] = v.queue.map { mapOf("id" to it.id, "symbol" to it.symbol, "date" to it.date, "pnl" to it.pnl, "missing" to it.missing) }
        out["options"] = mapOf("accounts" to v.options.accounts, "symbols" to v.options.symbols, "tags" to v.options.tags, "exchanges" to v.options.exchanges,
            "kinds" to v.options.kinds, "years" to v.options.years)
        out["cashflowHoldings"] = cf.holdings.sortedBy { it.symbol }.map { h ->
            mapOf(
                "symbol" to h.symbol, "qty" to h.qty, "per" to opt(h.per), "freq" to (h.freq ?: JSONObject.NULL),
                "freqVerified" to h.freqVerified, "annual" to opt(h.annual), "yoc" to opt(h.yoc), "ytd" to h.ytd, "ttm" to h.ttm, "all" to h.all,
                "nextExDate" to h.nextExDate, "nextPayDate" to h.nextPayDate, "exPast" to h.exPast, "payPast" to h.payPast,
            )
        }
        out["cashflowTiles"] = cf.tiles.map { t ->
            val d = LinkedHashMap<String, Any>()
            d["label"] = t.label
            t.total?.let { d["total"] = it }
            t.perMonth?.let { d["perMonth"] = it }
            t.count?.let { d["count"] = it }
            if (t.label == "Margin used") {
                d["marginUsed"] = t.marginUsed ?: 0.0
                d["interestPerMonth"] = t.interestPerMonth ?: 0.0
                d["interestMonths"] = t.interestMonths ?: 0
            }
            if (t.label == "Yield on cost") {
                d["yield"] = opt(t.yield)
                d["projected"] = t.projected ?: 0.0
                d["earned"] = t.earned ?: 0.0
                d["book"] = t.book ?: 0.0
            }
            d
        }
        out["cashflowMonths"] = cf.months.map { mapOf("key" to it.key, "label" to it.label, "value" to it.value, "count" to it.count) }
        out["cashflowTotal"] = cf.total
        out["cashflowCount"] = cf.count
        out["cashflowSkipped"] = cf.skippedFilters
        return out
    }

    // Comparing.

    private fun diff(got: Any?, want: Any?, path: String, out: MutableList<String>) {
        when (got) {
            is Map<*, *> -> {
                if (want !is JSONObject) { out.add("$path: expected $want, got object"); return }
                val keys = (got.keys.map { it.toString() } + want.keys().asSequence().toList()).toSet().sorted()
                for (key in keys) {
                    if (!got.containsKey(key)) { out.add("$path.$key: missing on the Kotlin side"); continue }
                    if (!want.has(key)) { out.add("$path.$key: not in the case"); continue }
                    diff(got[key], want.get(key), "$path.$key", out)
                }
            }
            is List<*> -> {
                if (want !is JSONArray) { out.add("$path: expected $want, got list"); return }
                if (got.size != want.length()) out.add("$path: ${want.length()} expected, got ${got.size}")
                for (i in 0 until minOf(got.size, want.length())) diff(got[i], want.get(i), "$path[$i]", out)
            }
            JSONObject.NULL, null -> if (want != JSONObject.NULL) out.add("$path: expected $want, got null")
            is String -> if (got != want) out.add("$path: expected $want, got \"$got\"")
            is Boolean -> if (got != want) out.add("$path: expected $want, got $got")
            is Number -> {
                if (want !is Number) { out.add("$path: expected $want, got $got"); return }
                if (abs(got.toDouble() - want.toDouble()) > 2e-6) out.add("$path: expected $want, got $got")
            }
            else -> out.add("$path: cannot compare $got with $want")
        }
    }

    @Test
    fun everyCaseMatches() {
        val files = (casesDir.listFiles() ?: emptyArray()).filter { it.extension == "json" }.sortedBy { it.name }
        assertTrue(files.isNotEmpty(), "no cases found at ${casesDir.absolutePath}")
        for (file in files) {
            val doc = JSONObject(file.readText())
            val snapshot = doc.getJSONObject("snapshot")
            val rows = snapshot.getJSONArray("activities")
            val acts = (0 until rows.length()).map { activity(rows.getJSONObject(it)) }
            val secRows = snapshot.optJSONArray("securities") ?: JSONArray()
            val secs = (0 until secRows.length()).map { security(secRows.getJSONObject(it)) }
            val navRows = snapshot.optJSONArray("navHistory") ?: JSONArray()
            val nav = (0 until navRows.length()).map { navPoint(navRows.getJSONObject(it)) }
            val navByAccount = HashMap<String, List<NavPoint>>()
            snapshot.optJSONObject("navByAccount")?.let { o ->
                for (nick in o.keys()) {
                    val pts = o.getJSONArray(nick)
                    navByAccount[nick] = (0 until pts.length()).map { navPoint(pts.getJSONObject(it)) }
                }
            }
            val accRows = snapshot.optJSONArray("accounts") ?: JSONArray()
            val accounts = (0 until accRows.length()).map { i -> val a = accRows.getJSONObject(i); AccountInfo(a.optString("id"), a.optString("nickname"), a.optString("currency"), if (a.isNull("netLiquidationValue")) null else a.optDouble("netLiquidationValue"), a.optString("unifiedAccountType"), a.optString("status")) }
            val balRows = snapshot.optJSONArray("balances") ?: JSONArray()
            val balances = (0 until balRows.length()).map { i -> val b = balRows.getJSONObject(i); BalanceRow(b.optString("accountId"), b.optString("securityId"), b.optDouble("quantity", 0.0)) }
            val marginRows = snapshot.optJSONArray("margin") ?: JSONArray()
            val margin = (0 until marginRows.length()).map { i -> val m = marginRows.getJSONObject(i); MarginRow(m.optString("accountId"), if (m.isNull("buyingPower")) null else m.optDouble("buyingPower"), m.optString("currency", "CAD"), m.optString("unavailable")) }
            val base = Model.buildBase(acts, secs, market(doc.getJSONObject("market")), doc.getString("today"), nav, navByAccount, journal(doc.optJSONObject("journal")), accounts, balances, margin)
            @Suppress("UNCHECKED_CAST")
            val filters = Filters.clean(loose(doc.optJSONObject("filters")) as? Map<String, Any?>)
            val view = ModelView.buildView(base, filters)
            val problems = mutableListOf<String>()
            diff(expect(view), doc.getJSONObject("expect"), file.name, problems)
            assertTrue(problems.isEmpty(), problems.joinToString("\n"))
        }
    }

    @Test
    fun dateArithmetic() {
        assertEquals(31, Model.daysBetween("2026-01-10", "2026-02-10"))
        assertEquals(0, Model.daysBetween("2026-02-10", "2026-01-10"))
        assertEquals("2026-02-28", Model.shiftDate("2026-03-01", -1))
        assertEquals("2024-02-29", Model.shiftDate("2024-02-28", 1))
        assertEquals("2025-08-29", Model.optionExpiry("LUNR 29AUG25 11.50 CALL"))
        assertEquals("2026-01-02", Model.optionExpiry("BBAI 02JAN26 5.50 PUT"))
        assertEquals("", Model.optionExpiry("AAPL"))
    }
}
