// The derived model, a port of model.py (the reference), function for
// function. Everything a screen shows comes from here so that one list of
// trades feeds every tile, table and chart, and so the numbers can be tested:
// ModelCasesTest runs tests/cases through it and compares with the Python.
//
// Pipeline
//     activities  -> normalizeActivities  (crypto, options, stock-dividend notices)
//                 -> synthesizeAssignmentShares
//                 -> matchFifo            (FIFO lots per account+symbol+currency,
//                                          round-trip ids, option rolls)
//                 -> synthesizeExpiries   (then matchFifo again)
//                 -> applyFx              (P&L in CAD on the fill dates)
//                 -> buildTrades          (round trips)
//                 -> buildPositions       (open lots rolled up per symbol+account)
//                 -> buildCashflow        (dividends, interest, withholding tax)
//     kpi(trades), cashflowView(base)
//
// Currency: per-trade numbers are native. Anything that adds trades together
// uses the CAD value converted on the fill date with the Bank of Canada rate.
package com.bagholder.model

import java.time.LocalDate
import java.time.ZoneId
import java.time.ZonedDateTime
import java.time.format.DateTimeFormatter
import java.util.Locale
import kotlin.math.abs
import kotlin.math.min
import kotlin.math.max
import kotlin.math.round

/** A normalized activity. Mutable, as in the Python: fills, lots and the
 * trade's fill list all see the row the matcher amended (a multileg's
 * inferred quantity and side, for one). */
class Act {
    var id = ""; var occurredAt = ""; var transactionDate = ""; var accountId = ""; var fifoId = ""; var accountType = ""
    var activityType = ""; var activitySubType = ""; var description = ""; var direction = ""; var symbol = ""; var name = ""; var currency = ""
    var quantity = 0.0; var unitPrice = 0.0; var commission = 0.0; var netCashAmount = 0.0
    var category = ""; var rawType = ""; var aftType = ""; var securityId = ""
    var kind = ""
    var flags: MutableList<String> = mutableListOf()

    fun copy(): Act {
        val a = Act()
        a.id = id; a.occurredAt = occurredAt; a.transactionDate = transactionDate; a.accountId = accountId
        a.fifoId = fifoId; a.accountType = accountType; a.activityType = activityType; a.activitySubType = activitySubType
        a.description = description; a.direction = direction; a.symbol = symbol; a.name = name; a.currency = currency
        a.quantity = quantity; a.unitPrice = unitPrice; a.commission = commission; a.netCashAmount = netCashAmount
        a.category = category; a.rawType = rawType; a.aftType = aftType; a.securityId = securityId
        a.kind = kind; a.flags = flags.toMutableList()
        return a
    }
}

data class Security(
    val id: String = "", val symbol: String = "", val name: String = "", val underlyingId: String = "",
    val primaryExchange: String = "", val primaryMic: String = "", val currency: String = "",
)

data class Distribution(val exDate: String = "", val payDate: String = "", val amount: Double = 0.0, val currency: String = "")

data class Quote(
    val price: Double? = null, val priceChange: Double? = null, val percentChange: Double? = null,
    val fetchedAt: String = "", val exDividendDate: String = "",
)

data class Market(
    val fx: Map<String, Double> = emptyMap(),
    val distributions: Map<String, List<Distribution>> = emptyMap(),
    val quotes: Map<String, Quote> = emptyMap(),
    val benchmark: Map<String, Double> = emptyMap(),
    val benchmarks: Map<String, Map<String, Double>> = emptyMap(),
)

/** One day of Wealthsimple's NAV history. */
data class NavPoint(val date: String = "", val equity: Double? = null, val netDeposits: Double? = null)

/** A journal entry, keyed by the round trip that opened the trade or position. */
data class JournalEntry(val grade: String = "", val thesis: String = "", val tags: List<String> = emptyList())

class Lot(
    var qty: Double, var price: Double, var date: String, var whenAt: String, var commission: Double, var direction: String,
    var accountId: String, var accountType: String, var symbol: String, var name: String, var currency: String, var kind: String,
    var activityId: String, var securityId: String, var rt: String?, var flags: MutableList<String>,
) {
    fun copy() = Lot(qty, price, date, whenAt, commission, direction, accountId, accountType, symbol, name, currency, kind, activityId, securityId, rt, flags.toMutableList())
}

class Slice {
    var id = ""; var rt: String? = null
    var accountId = ""; var accountType = ""; var account = ""; var symbol = ""; var name = ""; var currency = ""; var kind = ""; var side = ""
    var quantity = 0.0; var entryPrice = 0.0; var exitPrice = 0.0
    var entryDate = ""; var exitDate = ""; var entryWhen = ""; var exitWhen = ""
    var holdDays = 0
    var commission = 0.0; var entryCommission = 0.0; var exitCommission = 0.0; var pnl = 0.0; var pnlCad = 0.0; var feesCad = 0.0
    var openDirection = ""; var buyActivityId = ""; var sellActivityId = ""; var securityId = ""
    var flags: MutableList<String> = mutableListOf()
}

data class Unmatched(
    val symbol: String, val currency: String, val side: String, val quantity: Double, val price: Double, val date: String,
    val description: String, val accountId: String, val account: String, val activityId: String,
)

class FillRow {
    var id = ""; var whenAt = ""; var date = ""; var time = ""; var side = ""; var sub = ""
    var qty = 0.0; var price = 0.0; var amount = 0.0; var fees = 0.0
    var currency = ""
    var flags: List<String> = emptyList()
}

class Trade {
    var id = ""; var status = "closed"; var symbol = ""; var underlying = ""; var name = ""; var exchange = ""; var kind = ""; var currency = ""
    var account = ""; var accountId = ""; var securityId = ""; var side = ""; var openDirection = ""
    var qty = 0.0; var mult = 1.0; var entry = 0.0; var exit = 0.0
    var entryDate = ""; var exitDate = ""; var entryWhen = ""; var exitWhen = ""
    var holdDays = 0
    var pnl = 0.0; var pnlCad = 0.0; var fees = 0.0; var feesCad = 0.0
    var pnlPct: Double? = null
    var legCount = 0
    var fills: List<FillRow> = emptyList()
    var flags: List<String> = emptyList()
    var grade = ""; var thesis = ""
    var tags: List<String> = emptyList()
}

data class PositionLot(val opened: String, val qty: Double, val price: Double, val basis: Double, val held: Int, val flags: List<String>, val activityId: String)

class Position {
    var id = ""; var symbol = ""; var underlying = ""; var name = ""; var exchange = ""; var kind = ""; var account = ""; var accountId = ""
    var currency = ""; var securityId = ""
    var short = false
    var qty = 0.0; var mult = 1.0; var avg = 0.0; var cost = 0.0; var fees = 0.0; var last = 0.0
    var lastAt = ""; var priceSource = ""
    var priceChange: Double? = null; var percentChange: Double? = null
    var mv = 0.0; var unreal = 0.0
    var unrealPct: Double? = null
    var held = 0
    var opened = ""
    var rt: String? = null
    var lots: List<PositionLot> = emptyList()
    var alloc = 0.0
    var dayChange: Double? = null
    var fills: List<FillRow> = emptyList()
    var grade = ""; var thesis = ""
    var tags: List<String> = emptyList()
}

/** What Wealthsimple states per account: its net liquidation value, in its currency. */
class AccountInfo(val id: String, val name: String, val currency: String, val nav: Double?, val type: String = "", val status: String = "")

class BalanceRow(val accountId: String, val securityId: String, val quantity: Double)

/** Wealthsimple's buying power for an account, or why it has none. */
class MarginRow(val accountId: String, val buyingPower: Double?, val currency: String = "CAD", val unavailable: String = "")

class AllocationRow(val id: String, val symbol: String, val account: String, val value: Double) { var share = 0.0 }

/** The Portfolio tiles: CAD aggregates over the accounts in scope (model.py portfolio_view). */
class Portfolio {
    var allocation: List<AllocationRow> = emptyList()
    var marketValue = 0.0; var costBasis = 0.0; var unrealized = 0.0
    var unrealizedPct: Double? = null
    var positionCount = 0; var accountCount = 0
    var nav: Double? = null
    var navAccounts = 0
    var marginUsed = 0.0
    var marginUsedBy: Map<String, Double> = emptyMap()
    var marginUsedPct: Double? = null
    var availableMargin: Double? = null
    var availableMarginUnavailable: List<String> = emptyList()
    var hasMargin = false
    var cash = 0.0
    var cashPct: Double? = null; var dayChange: Double? = null; var dayChangePct: Double? = null
}

class CashRow {
    var id = ""; var date = ""; var time = ""; var symbol = ""; var name = ""; var kind = ""; var account = ""; var accountId = ""
    var qty: Double? = null; var per: Double? = null
    var amount = 0.0; var currency = ""; var amountCad = 0.0
}

class Holding {
    var id = ""; var symbol = ""; var account = ""
    var qty = 0.0
    var per: Double? = null
    var freq: Int? = null
    var freqVerified = false
    var rateSource = ""
    var cost = 0.0; var avg = 0.0; var last = 0.0
    var priceSource = ""
    var ytd = 0.0; var ttm = 0.0; var all = 0.0
    var nextExDate = ""; var nextPayDate = ""
    var exPast = false; var payPast = false
    var yob: Double? = null; var annual: Double? = null; var yoc: Double? = null; var currentYield: Double? = null
}

data class Tile(
    val label: String, val total: Double? = null, val perMonth: Double? = null, val count: Int? = null,
    val yield: Double? = null, val projected: Double? = null, val earned: Double? = null, val book: Double? = null,
    val marginUsed: Double? = null, val interestPerMonth: Double? = null, val interestMonths: Int? = null,
)

data class MonthBar(val key: String, val label: String, val value: Double, val count: Int)

data class KPI(
    val realized: Double, val count: Int, val wins: Int, val losses: Int, val breakeven: Int, val winRate: Double?,
    val grossWin: Double, val grossLoss: Double, val profitFactor: Double?, val profitFactorInfinite: Boolean,
    val expectancy: Double?, val avgWin: Double, val avgLoss: Double, val fees: Double, val avgHold: Double?, val openCount: Int,
)

data class CashflowView(
    val tiles: List<Tile>, val months: List<MonthBar>, val holdings: List<Holding>, val rows: List<CashRow>, val other: List<CashRow>,
    val total: Double, val count: Int, val interest: Double, val withholding: Double,
    val skippedFilters: List<String> = emptyList(),
)

class Base {
    var today = ""
    var fx: Map<String, Double> = emptyMap()
    var benchmark: Map<String, Double> = emptyMap()
    var benchmarks: Map<String, Map<String, Double>> = emptyMap()
    var equity: List<EquityPoint> = emptyList()
    var equityByAccount: Map<String, List<EquityPoint>> = emptyMap()
    var journal: Map<String, JournalEntry> = emptyMap()
    var distributions: Map<String, List<Distribution>> = emptyMap()
    var quotes: Map<String, Quote> = emptyMap()
    var activities: List<Act> = emptyList()
    var closed: List<Slice> = emptyList()
    var openLots: List<Lot> = emptyList()
    var unmatched: List<Unmatched> = emptyList()
    var trades: List<Trade> = emptyList()
    var positions: List<Position> = emptyList()
    var cashflow: List<CashRow> = emptyList()
    var accounts: List<AccountInfo> = emptyList()
    var balances: List<BalanceRow> = emptyList()
    var margin: List<MarginRow> = emptyList()
    var cashCurrencies: Map<String, String> = emptyMap()
}

object Model {
    const val EPS = 1e-10
    const val FX_FALLBACK = 1.35
    val MONTHS = listOf("Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec")
    val KINDS = listOf("Shares", "Options", "Crypto", "Futures")
    val SCHEDULES = listOf(52, 26, 24, 12, 6, 4, 2, 1)
    val GRADES = listOf("A", "B", "C", "F")
    val BENCHMARK_LABELS = mapOf("SP500" to "S&P 500", "TSX" to "S&P/TSX", "TSX60" to "TSX 60")
    val PRESET_DAYS = mapOf("1d" to 1, "1w" to 7, "1m" to 30, "3m" to 90, "6m" to 180, "1y" to 365, "5y" to 1826)
    val TIME_ZONE: ZoneId = ZoneId.of("America/Edmonton")

    // MARK: small helpers

    private val SPACE_RE = Regex("[\\s\u00a0\u2000-\u200b\u202f\u205f\u3000]+")
    private val COMPACT_RE = Regex("[\\s_\\-]+")
    private val RIGHT_RE = Regex("\\b(PUT|CALL)\\b")
    private val SHORT_RIGHT_RE = Regex("\\s[CP]$")
    private val OCC_RE = Regex("^[A-Z][A-Z0-9.]{0,9} \\d{6}[CP]\\d+")
    private val OCC_UNDER_RE = Regex("^([A-Z][A-Z0-9.]{0,9}) \\d{6}[CP]\\d+")
    private val WS_UNDER_RE = Regex("^([A-Z][A-Z0-9.]{0,9}) \\d{1,2}[A-Z]{3}\\d{2}\\b")
    private val PUT_OCC_RE = Regex(" \\d{6}P\\d+")
    private val EXPIRY_RE = Regex("^\\S+ (\\d{2})([A-Z]{3})(\\d{2}) ")
    private val STRIKE_RE = Regex(" (\\d+(?:\\.\\d+)?) (CALL|PUT)$")
    private val REMOVAL_RE = Regex("CODECHANGE|SYMBOLCHANGE|TICKERCHANGE|LISTINGSTATUS|SECURITYSWAP")
    private val LISTING_RE = Regex("^(.+)\\.(TO|V|CN|NE)$", RegexOption.IGNORE_CASE)
    private val ISO_RE = Regex("^\\d{4}-\\d{2}-\\d{2}$")

    fun compact(s: String) = COMPACT_RE.replace(s.trim().uppercase(), "")

    fun normAccountName(s: String) = SPACE_RE.replace(s, " ").trim()

    private fun spaced(symbol: String) = SPACE_RE.replace(symbol.trim().uppercase(), " ")

    fun isOptionSymbol(symbol: String): Boolean {
        val u = spaced(symbol)
        if (u.isEmpty()) return false
        if (RIGHT_RE.containsMatchIn(u) || SHORT_RIGHT_RE.containsMatchIn(u)) return true
        if (OCC_RE.containsMatchIn(u)) return true
        return false
    }

    fun underlyingSymbol(symbol: String): String {
        val s = symbol.trim()
        if (s.isEmpty()) return "—"
        val u = spaced(s)
        if (RIGHT_RE.containsMatchIn(u) || SHORT_RIGHT_RE.containsMatchIn(u)) {
            val first = u.split(" ")[0]
            return if (first.isEmpty()) s else first
        }
        OCC_UNDER_RE.find(u)?.let { return it.groupValues[1] }
        WS_UNDER_RE.find(u)?.let { return it.groupValues[1] }
        return s
    }

    fun optionMultiplier(symbol: String) = if (isOptionSymbol(symbol)) 100.0 else 1.0

    fun optionRight(symbol: String): String {
        val u = spaced(symbol)
        if (u.endsWith(" PUT") || u.endsWith(" P") || PUT_OCC_RE.containsMatchIn(u)) return "PUT"
        return "CALL"
    }

    fun isMultileg(a: Act) = compact(a.rawType).contains("MULTILEG")

    fun rollKey(a: Act) = fifoAccount(a) + "\u0001" + underlyingSymbol(a.symbol) + "\u0001" + optionRight(a.symbol)

    private fun parseDay(iso: String): LocalDate? {
        val s = iso.take(10)
        if (!ISO_RE.matches(s)) return null
        return try { LocalDate.parse(s) } catch (e: Exception) { null }
    }

    fun daysBetween(a: String, b: String): Int {
        val da = parseDay(a) ?: return 0
        val db = parseDay(b) ?: return 0
        return max(0L, db.toEpochDay() - da.toEpochDay()).toInt()
    }

    fun shiftDate(iso: String, days: Int): String {
        val d = parseDay(iso) ?: return iso.take(10)
        return d.plusDays(days.toLong()).toString()
    }

    fun todayLocal(): String = LocalDate.now(TIME_ZONE).toString()

    /** ISO instant -> (YYYY-MM-DD, HH:MM) in the app's local time zone. */
    fun whenParts(occurred: String): Pair<String, String> {
        val s = occurred.trim()
        if (s.isEmpty()) return Pair("", "")
        if (!s.contains("T")) return Pair(s.take(10), "")
        val dt = try {
            ZonedDateTime.parse(if (s.endsWith("Z")) s else s).withZoneSameInstant(TIME_ZONE)
        } catch (e: Exception) {
            try {
                java.time.LocalDateTime.parse(s).atZone(ZoneId.of("UTC")).withZoneSameInstant(TIME_ZONE)
            } catch (e2: Exception) {
                return Pair(s.take(10), "")
            }
        }
        return Pair(dt.toLocalDate().toString(), dt.format(DateTimeFormatter.ofPattern("HH:mm")))
    }

    fun fmt8(v: Double): String = String.format(Locale.ROOT, "%.8f", v)

    /** store.trade_side. */
    fun tradeSide(a: Act): String {
        val sub = a.activitySubType.uppercase().replace(" ", "").replace("_", "").replace("-", "")
        if (sub in listOf("BUY", "BUYTOOPEN", "BTO", "BUYTOCLOSE", "BTC") || sub.startsWith("BUY")) return "BUY"
        if (sub in listOf("SELL", "SELLTOOPEN", "STO", "SELLTOCLOSE", "STC") || sub.startsWith("SELL")) return "SELL"
        val typ = a.activityType.uppercase().replace(" ", "").replace("_", "").replace("-", "")
        if (typ.startsWith("BUY")) return "BUY"
        if (typ.startsWith("SELL")) return "SELL"
        if (a.quantity > 0) return "BUY"
        if (a.quantity < 0) return "SELL"
        return ""
    }

    // MARK: activity normalization

    fun isCryptoActivity(a: Act) = compact(a.rawType).startsWith("CRYPTO") || compact(a.activityType).startsWith("CRYPTO")

    fun kindOf(a: Act): String {
        if (a.kind in KINDS) return a.kind
        if (isCryptoActivity(a)) return "Crypto"
        if (isOptionSymbol(a.symbol)) return "Options"
        return "Shares"
    }

    fun isIntentionalOpen(a: Act): Boolean {
        val fields = listOf(compact(a.activityType), compact(a.activitySubType))
        if (fields.any { it.contains("TOOPEN") }) return true
        return fields.any { it == "STO" || it == "BTO" }
    }

    fun isCloseOnly(a: Act): Boolean {
        val fields = listOf(compact(a.activityType), compact(a.activitySubType))
        if (fields.any { it.contains("TOCLOSE") || it == "BTC" || it == "STC" }) return true
        return fields.any { it.contains("EXPIR") || it.contains("ASSIGN") || it.contains("EXERCISE") }
    }

    fun openingDirection(a: Act, side: String): String? {
        if (side == "BUY") return if (isCloseOnly(a)) null else "LONG"
        if (side == "SELL") {
            if (isOptionSymbol(a.symbol)) return if (isCloseOnly(a)) null else "SHORT"
            if (isIntentionalOpen(a)) return "SHORT"
            return null
        }
        return null
    }

    /** Copy of the row with crypto and option events expressed as trade fills. */
    fun normalizeActivity(activity: Act): Act {
        val a = activity.copy()
        a.accountType = normAccountName(a.accountType)
        val rt = compact(a.rawType)
        val at = compact(a.activityType)
        val cash = a.netCashAmount
        val qty = abs(a.quantity)
        a.flags = mutableListOf()

        val swapSub = compact(a.activitySubType)
        if (((rt.startsWith("CRYPTO") || at.startsWith("CRYPTO")) && swapSub.contains("SWAP")) || rt == "SWAPMARKETORDER") {
            a.category = "other"; a.kind = "Crypto"; a.flags.add("missing-swap-legs"); return a
        }

        if (rt == "CRYPTOBUY" || at == "CRYPTOBUY") {
            a.category = "trade"; a.activityType = "Trade"; a.activitySubType = "BUY"; a.kind = "Crypto"
            a.quantity = qty
            a.netCashAmount = -abs(cash)
            return a
        }
        if (rt == "CRYPTOSELL" || at == "CRYPTOSELL") {
            a.category = "trade"; a.activityType = "Trade"; a.activitySubType = "SELL"; a.kind = "Crypto"
            a.quantity = -qty
            a.netCashAmount = abs(cash)
            return a
        }
        if (rt == "CRYPTOTRANSFER" || at == "CRYPTOTRANSFER") {
            val sub = compact(a.activitySubType)
            a.category = "trade"; a.activityType = "Trade"; a.kind = "Crypto"
            a.flags.add("transfer")
            if (sub.contains("OUT") || cash < 0) {
                a.activitySubType = "SELL"
                a.flags.add("transfer-out")
                a.quantity = -qty
                a.netCashAmount = abs(cash)
            } else {
                a.activitySubType = "BUY"
                a.quantity = qty
                a.netCashAmount = -abs(cash)
            }
            return a
        }
        if (rt == "CRYPTOSTAKINGREWARD" || at == "CRYPTOSTAKINGREWARD") {
            a.category = "trade"; a.activityType = "Trade"; a.activitySubType = "BUY"; a.kind = "Crypto"
            a.flags.add("reward")
            a.quantity = qty
            a.unitPrice = 0.0
            a.netCashAmount = 0.0
            return a
        }
        if (rt.startsWith("CRYPTO")) {
            a.category = "other"
            a.kind = "Crypto"
            return a
        }

        if (at == "STKDIS" && rt == "DIVIDEND" && abs(cash) < EPS) {
            // A distribution posted in units with no cash is a pending notice,
            // not a share delivery: Wealthsimple's balance does not grow by it.
            a.category = "other"
            a.flags.add("pending-distribution")
            return a
        }

        val raw = rt + at
        if (raw.contains("MULTILEG")) {
            a.category = "trade"
            if (cash < 0 || compact(a.direction) == "DEBIT") {
                a.activityType = "OPTIONS_BUY"
                a.activitySubType = "BUYTOCLOSE"
            } else {
                a.activityType = "OPTIONS_SELL"
                a.activitySubType = "SELLTOOPEN"
            }
        } else if (raw.contains("EXPIR") || raw.contains("ASSIGN") || raw.contains("EXERCISE")) {
            a.category = "option_event"
            if (raw.contains("ASSIGN")) {
                a.activityType = "ASSIGN"
                a.activitySubType = "BUYTOCLOSE"
                a.unitPrice = 0.0
            } else if (raw.contains("SHORTEXPIR")) {
                a.activityType = "EXPIR"
                a.activitySubType = "BUY"
            } else if (raw.contains("EXPIR")) {
                a.activityType = "EXPIR"
                a.activitySubType = "SELL"
            } else {
                a.activityType = "EXERCISE"
                a.activitySubType = "SELL"
            }
            if (raw.contains("ASSIGN") || abs(cash) < 1e-12) {
                a.unitPrice = 0.0
            }
            if (qty > 0) {
                a.quantity = if (a.activitySubType == "SELL") -qty else qty
            }
        }
        a.kind = kindOf(a)
        return a
    }

    fun normalizeActivities(activities: List<Act>) = activities.map { normalizeActivity(it) }

    /** Net +N/-N name-change rows on one day; leftover +N opens at $0. */
    fun foldStkdis(activities: List<Act>): List<Act> {
        val rest = mutableListOf<Act>()
        class Group(var pos: Double, var neg: Double, val sample: Act)
        val groups = LinkedHashMap<String, Group>()
        for (a in activities) {
            if (compact(a.activityType) != "STKDIS") {
                rest.add(a)
                continue
            }
            val k = a.symbol + "\u0001" + a.transactionDate + "\u0001" + a.currency
            val g = groups.getOrPut(k) { Group(0.0, 0.0, a) }
            val q = a.quantity
            if (a.activitySubType == "SELL" || q < 0) g.neg += abs(q) else g.pos += abs(q)
        }
        for (g in groups.values) {
            val net = g.pos - g.neg
            if (net > EPS) {
                val a = g.sample.copy()
                a.quantity = net; a.activitySubType = "BUY"; a.unitPrice = 0.0; a.netCashAmount = 0.0; a.category = "trade"
                rest.add(a)
            }
        }
        return rest
    }

    /** Wealthsimple posts a share split as a CORPORATE_ACTION with quantity 0
     * and no ratio. Infer the ratio from the fill prices on either side and
     * return (account, symbol, date) -> factor. */
    fun splitMarkers(activities: List<Act>): Map<String, Double> {
        val out = LinkedHashMap<String, Double>()
        val byBook = HashMap<String, MutableList<Act>>()
        for (a in activities) {
            if ((a.category != "trade" && a.category != "option_event") || a.symbol.isEmpty()) continue
            byBook.getOrPut(fifoAccount(a) + "\u0001" + a.symbol) { mutableListOf() }.add(a)
        }
        for (a in activities) {
            if (compact(a.activityType) != "STKDIS" || compact(a.rawType) != "CORPORATEACTION") continue
            if (abs(a.quantity) > EPS) continue
            val day = a.transactionDate
            val key = fifoAccount(a) + "\u0001" + a.symbol
            val priced = (byBook[key] ?: emptyList()).filter { it.unitPrice > 0 && compact(it.activityType) != "STKDIS" }
                .sortedWith(compareBy({ it.transactionDate }, { it.occurredAt }))
            val before = priced.filter { it.transactionDate < day }.map { it.unitPrice }.takeLast(3).sorted()
            val after = priced.filter { it.transactionDate >= day }.map { it.unitPrice }.take(3).sorted()
            if (before.isEmpty() || after.isEmpty()) continue
            val pre = before[before.size / 2]
            val post = after[after.size / 2]
            if (!(pre > 0) || !(post > 0)) continue
            val ratio = post / pre
            val n: Int
            val factor: Double
            if (ratio >= 1.5) {
                n = round(ratio).toInt()
                factor = 1.0 / n
            } else if (ratio <= 1 / 1.5) {
                n = round(1 / ratio).toInt()
                factor = n.toDouble()
            } else {
                continue
            }
            if (n < 2 || abs(ratio - (1 / factor)) / (1 / factor) > 0.35) continue
            out[fifoAccount(a) + "\u0001" + a.symbol + "\u0001" + day] = factor
        }
        return out
    }

    fun fifoAccount(a: Act): String {
        val nick = normAccountName(a.accountType)
        if (nick.isNotEmpty()) return nick
        if (a.fifoId.isNotEmpty()) return a.fifoId
        return a.accountId
    }

    fun bookKey(a: Act) = fifoAccount(a) + "::" + a.symbol + "::" + a.currency

    class ReplacementIndex(val removed: MutableMap<String, String> = HashMap(), val trades: MutableMap<String, MutableList<String>> = HashMap())

    fun replacementIndex(activities: List<Act>): ReplacementIndex {
        val idx = ReplacementIndex()
        for (a in activities) {
            val key = fifoAccount(a) + "\u0001" + a.symbol + "\u0001" + a.currency
            val t = compact(a.activityType)
            val d = a.transactionDate
            if (t == "STKDIS") {
                val sub = compact(a.activitySubType)
                if (sub == "SELL" || a.quantity < 0) {
                    if (d.isNotEmpty() && (idx.removed[key] == null || d < idx.removed[key]!!)) idx.removed[key] = d
                }
                continue
            }
            val raw = compact(a.rawType) + compact(a.aftType)
            if (REMOVAL_RE.containsMatchIn(raw)) {
                if (d.isNotEmpty() && (idx.removed[key] == null || d < idx.removed[key]!!)) idx.removed[key] = d
            }
            if ((a.category == "trade" || a.category == "option_event") && tradeSide(a).isNotEmpty()) {
                idx.trades.getOrPut(key) { mutableListOf() }.add(d)
            }
        }
        return idx
    }

    fun tickerWasReplaced(index: ReplacementIndex, account: String, symbol: String, currency: String, byDate: String): Boolean {
        val key = account + "\u0001" + symbol + "\u0001" + currency
        val removedOn = index.removed[key] ?: return false
        if (removedOn > byDate) return false
        return (index.trades[key] ?: emptyList()).none { it > removedOn }
    }

    // MARK: option quantity inference (WS multileg rows often carry qty 0)

    class Fill(var a: Act, var side: String, var qty: Double) {
        var rollDirection: String? = null
        var rtBefore: String? = null
    }

    private fun setFillSide(f: Fill, side: String, sub: String) {
        f.side = side
        f.a.activitySubType = sub
        val q = abs(f.a.quantity)
        if (q > 0) f.a.quantity = if (side == "SELL") -q else q
    }

    private fun resolveOptionFillSide(f: Fill, rem: MutableMap<String, Double>) {
        val a = f.a
        val raw = compact(a.rawType) + compact(a.activityType)
        val expirish = raw.contains("EXPIR") || raw.contains("ASSIGN") || raw.contains("EXERCISE")
        if (expirish) {
            if (raw.contains("ASSIGN") || raw.contains("SHORTEXPIR")) {
                setFillSide(f, "BUY", if (raw.contains("ASSIGN")) "BUYTOCLOSE" else "BUY")
            } else if (raw.contains("EXPIR") && !raw.contains("SHORT")) {
                setFillSide(f, "SELL", "SELL")
            } else if (f.side == "BUY" && rem["LONG"]!! > EPS && rem["SHORT"]!! <= EPS) {
                setFillSide(f, "SELL", "SELL")
            } else if (f.side == "SELL" && rem["SHORT"]!! > EPS && rem["LONG"]!! <= EPS) {
                setFillSide(f, "BUY", "BUY")
            }
            return
        }
        if (!(raw.contains("MULTILEG") || isCloseOnly(a))) return
        if (f.side == "BUY") {
            a.activitySubType = if (rem["SHORT"]!! > EPS) "BUYTOCLOSE" else "BUYTOOPEN"
        } else if (f.side == "SELL") {
            a.activitySubType = if (rem["LONG"]!! > EPS) "SELLTOCLOSE" else "SELLTOOPEN"
        }
    }

    private fun isCleanOptionQty(cash: Double, qty: Double): Boolean {
        if (!(qty > 0)) return false
        val px = abs(cash) / (qty * 100.0)
        if (px < 0) return false
        if (abs(px * 100 - round(px * 100)) < 1e-6) return true
        if (abs(px * 10000 - round(px * 10000)) < 1e-4) return true
        return false
    }

    private fun inferStandaloneOptionQty(cash: Double): Double {
        val absCash = abs(cash)
        if (!(absCash > 0)) return 0.0
        val maxQty = min(10000, max(1, round(absCash).toInt()))
        for (q in 1..maxQty) {
            if (isCleanOptionQty(absCash, q.toDouble())) return q.toDouble()
        }
        return 1.0
    }

    fun inferZeroQtyOptionFills(fills: List<Fill>) {
        val remaining = HashMap<String, MutableMap<String, Double>>()
        fun remOf(a: Act) = remaining.getOrPut(bookKey(a)) { mutableMapOf("LONG" to 0.0, "SHORT" to 0.0) }
        val zerosByBook = HashMap<String, MutableList<Int>>()
        val pools = HashMap<String, MutableMap<String, Double>>()
        fun poolOf(a: Act) = pools.getOrPut(rollKey(a)) { mutableMapOf("LONG" to 0.0, "SHORT" to 0.0) }

        for ((i, f) in fills.withIndex()) {
            val a = f.a
            val qty = abs(a.quantity)
            val cash = a.netCashAmount
            if (!isOptionSymbol(a.symbol) || f.side.isEmpty()) continue
            if (qty == 0.0 && abs(cash) > 1e-9) {
                zerosByBook.getOrPut(bookKey(a)) { mutableListOf() }.add(i)
            }
        }

        for ((i, f) in fills.withIndex()) {
            val a = f.a
            if (f.side.isEmpty()) continue
            val rem = remOf(a)
            if (isOptionSymbol(a.symbol) && isMultileg(a)) {
                // A roll: this row closes what the contract holds (or what an earlier
                // roll carried forward), and the same quantity moves to the next contract.
                val pool = poolOf(a)
                val direction = if (rem["SHORT"]!! > EPS) "SHORT" else if (rem["LONG"]!! > EPS) "LONG" else (if (pool["SHORT"]!! >= pool["LONG"]!!) "SHORT" else "LONG")
                val openSz = rem[direction]!! + pool[direction]!!
                var qty = abs(a.quantity)
                val cash = a.netCashAmount
                if (qty == 0.0) {
                    val k = bookKey(a)
                    val upcoming = (zerosByBook[k] ?: emptyList()).count { it > i }
                    if (rem[direction]!! > EPS && upcoming == 0) {
                        qty = rem[direction]!!
                    } else if (openSz > EPS) {
                        var picked = 0.0
                        val cap = max(1, (openSz + 1e-9).toInt())
                        for (q in 1..cap) {
                            if (isCleanOptionQty(cash, q.toDouble())) { picked = q.toDouble(); break }
                        }
                        qty = if (picked > 0) picked else inferStandaloneOptionQty(cash)
                        if (qty > openSz) qty = openSz
                    } else {
                        qty = inferStandaloneOptionQty(cash)
                    }
                    a.unitPrice = if (qty > 0) abs(cash) / (qty * 100.0) else 0.0
                }
                f.side = if (direction == "SHORT") "BUY" else "SELL"
                a.activitySubType = if (direction == "SHORT") "BUYTOCLOSE" else "SELLTOCLOSE"
                a.quantity = if (direction == "SHORT") qty else -qty
                f.qty = qty
                f.rollDirection = direction
                val closed = min(qty, rem[direction]!!)
                rem[direction] = rem[direction]!! - closed
                pool[direction] = pool[direction]!! - min(qty - closed, pool[direction]!!)
                if (closed > EPS || qty > EPS) {
                    pool[direction] = pool[direction]!! + qty
                }
                continue
            }
            if (isOptionSymbol(a.symbol)) {
                resolveOptionFillSide(f, rem)
            }
            var qty = abs(a.quantity)
            val cash = a.netCashAmount
            if (isOptionSymbol(a.symbol) && qty == 0.0) {
                val closingDir = if (f.side == "BUY") "SHORT" else "LONG"
                val openSz = rem[closingDir] ?: 0.0
                if (abs(cash) > 1e-9) {
                    val k = bookKey(a)
                    val upcoming = (zerosByBook[k] ?: emptyList()).count { it > i }
                    if (openSz > 0 && upcoming == 0) {
                        qty = openSz
                    } else if (openSz > 0) {
                        var picked = 0.0
                        val cap = max(1, (openSz + 1e-9).toInt())
                        for (q in 1..cap) {
                            if (isCleanOptionQty(cash, q.toDouble())) { picked = q.toDouble(); break }
                        }
                        qty = if (picked > 0) picked else inferStandaloneOptionQty(cash)
                        if (qty > openSz) qty = openSz
                    } else {
                        qty = inferStandaloneOptionQty(cash)
                    }
                    a.unitPrice = if (qty > 0) abs(cash) / (qty * 100.0) else 0.0
                } else if (openSz > 0 && isCloseOnly(a)) {
                    qty = openSz
                    a.unitPrice = 0.0
                }
                if (qty > 0) {
                    a.quantity = if (f.side == "SELL") -qty else qty
                    f.qty = qty
                }
            }
            if (isOptionSymbol(a.symbol) && (compact(a.rawType).contains("ASSIGN") || compact(a.activityType).contains("ASSIGN"))) {
                a.unitPrice = 0.0
            }
            if (f.qty > 0) {
                val closingDir = if (f.side == "BUY") "SHORT" else "LONG"
                val opening = openingDirection(a, f.side)
                var left = f.qty
                val closeAmt = min(left, rem[closingDir] ?: 0.0)
                rem[closingDir] = rem[closingDir]!! - closeAmt
                left -= closeAmt
                if (left > EPS && isOptionSymbol(a.symbol)) {
                    val pool = poolOf(a)
                    val pooled = min(left, pool[closingDir]!!)
                    pool[closingDir] = pool[closingDir]!! - pooled
                    left -= pooled
                }
                if (left > EPS && opening != null) {
                    rem[opening] = rem[opening]!! + left
                }
            }
        }
    }

    // MARK: FIFO matching

    fun stableTradeId(accountId: String, symbol: String, currency: String, entryDate: String, exitDate: String, quantity: Double, entryPrice: Double, exitPrice: Double, side: String) =
        listOf(accountId, symbol, currency, entryDate, exitDate, fmt8(quantity), fmt8(entryPrice), fmt8(exitPrice), side).joinToString("|")

    fun stableTradeId(t: Slice) = stableTradeId(t.accountId, t.symbol, t.currency, t.entryDate, t.exitDate, t.quantity, t.entryPrice, t.exitPrice, t.side)

    fun sliceMemberKey(t: Slice): String {
        if (t.buyActivityId.isNotEmpty() && t.sellActivityId.isNotEmpty()) {
            return listOf(t.buyActivityId, t.sellActivityId, fmt8(t.quantity)).joinToString("|")
        }
        return t.id
    }

    fun fillRank(f: Fill): Int {
        val a = f.a
        val t = compact(a.activityType)
        val s = compact(a.activitySubType)
        val blob = t + s
        if ((blob.contains("TOOPEN") || t == "STO" || s == "STO") && f.side == "SELL") return 0
        if (isCloseOnly(a) && f.side == "BUY") return 1
        if (f.side == "BUY") return 2
        if (blob.contains("TOCLOSE") || t == "STC" || s == "STC") return 3
        return 4
    }

    private val fillOrder: Comparator<Fill> = compareBy({ it.a.transactionDate }, { fillRank(it) }, { it.a.occurredAt }, { it.a.id })

    fun makeSlice(lot: Lot, fill: Fill, a: Act, matched: Double, symbol: String? = null): Slice {
        val fillQty = fill.qty
        val exitCommission = if (fillQty > 0) a.commission * (matched / fillQty) else 0.0
        val entryCommission = if (lot.qty > 0) lot.commission * (matched / lot.qty) else 0.0
        val commission = entryCommission + exitCommission
        val sym = symbol ?: lot.symbol
        val mult = optionMultiplier(sym)
        val exitPx = a.unitPrice
        val rawPnl = if (lot.direction == "LONG") (exitPx - lot.price) * matched * mult else (lot.price - exitPx) * matched * mult
        val t = Slice()
        t.rt = lot.rt
        t.accountId = lot.accountId
        t.accountType = lot.accountType
        t.account = lot.accountType
        t.symbol = sym
        t.name = if (symbol != null) (if (a.name.isEmpty()) lot.name else a.name) else lot.name
        t.currency = lot.currency
        t.kind = lot.kind
        t.side = fill.side
        t.quantity = matched
        t.entryPrice = lot.price
        t.exitPrice = exitPx
        t.entryDate = lot.date
        t.exitDate = a.transactionDate
        t.entryWhen = lot.whenAt
        t.exitWhen = a.occurredAt
        t.holdDays = daysBetween(lot.date, a.transactionDate)
        t.commission = commission
        t.entryCommission = entryCommission
        t.exitCommission = exitCommission
        t.pnl = rawPnl - commission
        t.pnlCad = rawPnl - commission
        t.openDirection = lot.direction
        t.buyActivityId = lot.activityId
        t.sellActivityId = a.id
        t.securityId = if (lot.securityId.isNotEmpty()) lot.securityId else a.securityId
        t.flags = (lot.flags.toSet() + a.flags.toSet()).sorted().toMutableList()
        t.id = stableTradeId(t)
        return t
    }

    /** Sells that exceed the lots by a residue are rounding, not a short. */
    fun dust(remaining: Double, fill: Fill, a: Act): Boolean {
        val qty = fill.qty
        val px = abs(a.unitPrice)
        if (remaining <= 1e-6 * max(1.0, qty)) return true
        if ((if (a.kind.isEmpty()) kindOf(a) else a.kind) == "Crypto" && remaining <= 0.01 * qty) return true
        return px > 0 && remaining * px * optionMultiplier(a.symbol) < 0.01
    }

    class RollPool {
        val lots: MutableMap<String, MutableList<Lot>> = mutableMapOf("LONG" to mutableListOf(), "SHORT" to mutableListOf())
        var rt: String? = null
    }

    class FifoResult(val closed: List<Slice>, val open: List<Lot>, val unmatched: List<Unmatched>)

    /** FIFO per (account, symbol, currency). Returns closed slices, open lots, unmatched. */
    fun matchFifo(input: List<Act>): FifoResult {
        val normalized = input.filter { !it.flags.contains("pending-distribution") }
        val folded = foldStkdis(normalized)
        val fills = mutableListOf<Fill>()
        for (a in folded) {
            if ((a.category != "trade" && a.category != "option_event") || a.symbol.isEmpty()) continue
            val side = tradeSide(a)
            if (side.isEmpty()) continue
            fills.add(Fill(a, side, abs(a.quantity)))
        }
        fills.sortWith(fillOrder)
        inferZeroQtyOptionFills(fills)
        val usable = fills.filter { it.qty > 0 }

        val books = LinkedHashMap<String, MutableList<Lot>>()
        val rtOpen = HashMap<String, String?>()
        val closed = mutableListOf<Slice>()
        val unmatched = mutableListOf<Unmatched>()
        val rolled = LinkedHashMap<String, RollPool>()
        val rolledKeys = HashSet<String>()

        fun rolledOf(a: Act) = rolled.getOrPut(rollKey(a)) { RollPool() }

        /** Close carried-forward legs against this fill; they take this contract's
         * symbol. When this chain has been rolled, a buy-back beyond the known
         * shorts also closes the chain's older contracts (nearest expiry first). */
        fun closeRolled(fill: Fill, a: Act, remainingIn: Double, closingDir: String): Double {
            var remaining = remainingIn
            val pool = rolledOf(a)
            val lots = pool.lots[closingDir]!!
            val key = bookKey(a)
            var rt = fill.rtBefore ?: pool.rt
            if (rt == null && lots.isNotEmpty()) rt = lots[0].rt ?: ("rt:" + lots[0].activityId)
            if (rt != null) pool.rt = rt
            while (remaining > EPS && lots.isNotEmpty()) {
                val lot = lots[0]
                lot.symbol = a.symbol
                lot.rt = rt ?: lot.rt ?: ("rt:" + lot.activityId)
                val matched = min(lot.qty, remaining)
                closed.add(makeSlice(lot, fill, a, matched))
                lot.qty -= matched
                remaining -= matched
                if (lot.qty <= EPS) lots.removeAt(0)
            }
            if (remaining > EPS && rolledKeys.contains(rollKey(a))) {
                val others = mutableListOf<Pair<String, String>>()
                for ((k2, b2) in books) {
                    if (k2 == key || b2.isEmpty()) continue
                    val bits = k2.split("::")
                    if (bits[0] != fifoAccount(a) || bits[2] != a.currency) continue
                    if (!isOptionSymbol(bits[1]) || underlyingSymbol(bits[1]) != underlyingSymbol(a.symbol) || optionRight(bits[1]) != optionRight(a.symbol)) continue
                    others.add(Pair(optionExpiry(bits[1]), k2))
                }
                for ((_, k2) in others.sortedWith(compareBy({ it.first }, { it.second }))) {
                    val b2 = books[k2]!!
                    while (remaining > EPS && b2.isNotEmpty() && b2[0].direction == closingDir) {
                        val lot = b2[0]
                        val matched = min(lot.qty, remaining)
                        val s = makeSlice(lot, fill, a, matched, a.symbol)
                        s.flags = (s.flags.toSet() + "rolled-in").sorted().toMutableList()
                        if (rt != null) s.rt = rt
                        closed.add(s)
                        lot.qty -= matched
                        remaining -= matched
                        if (lot.qty <= EPS) b2.removeAt(0)
                    }
                    if (b2.isEmpty()) rtOpen[k2] = null
                }
            }
            if (pool.lots["LONG"]!!.isEmpty() && pool.lots["SHORT"]!!.isEmpty() && (books[key] ?: emptyList<Lot>()).isEmpty()) {
                pool.rt = null
            }
            return remaining
        }

        val replaced = replacementIndex(normalized)
        val splits = splitMarkers(normalized)
        val pendingSplits = HashMap<String, MutableList<Pair<String, Double>>>()
        for ((k, factor) in splits) {
            val bits = k.split("\u0001")
            pendingSplits.getOrPut(bits[0] + "::" + bits[1]) { mutableListOf() }.add(Pair(bits[2], factor))
        }

        fun applySplits(key: String, day: String) {
            val skey = key.split("::").take(2).joinToString("::")
            val todo = pendingSplits[skey] ?: return
            val keep = mutableListOf<Pair<String, Double>>()
            for ((splitDay, factor) in todo.sortedWith(compareBy({ it.first }, { it.second }))) {
                if (splitDay <= day) {
                    for (lot in books[key] ?: emptyList<Lot>()) {
                        lot.qty *= factor
                        lot.price /= factor
                        val label = "split " + (if (factor < 1) "1:" + round(1 / factor).toInt() else "" + round(factor).toInt() + ":1")
                        if (!lot.flags.contains(label)) lot.flags.add(label)
                    }
                } else {
                    keep.add(Pair(splitDay, factor))
                }
            }
            if (keep.isEmpty()) pendingSplits.remove(skey) else pendingSplits[skey] = keep
        }

        fun closeAgainst(key: String, fill: Fill, a: Act, remainingIn: Double, symbolOverride: String? = null): Double {
            var remaining = remainingIn
            val book = books[key]!!
            val closingDir = if (fill.side == "BUY") "SHORT" else "LONG"
            while (remaining > EPS && book.isNotEmpty() && book[0].direction == closingDir) {
                val lot = book[0]
                val matched = min(lot.qty, remaining)
                closed.add(makeSlice(lot, fill, a, matched, symbolOverride))
                lot.commission *= if (lot.qty > 0) (lot.qty - matched) / lot.qty else 0.0
                lot.qty -= matched
                remaining -= matched
                if (lot.qty <= EPS) book.removeAt(0)
            }
            if (book.isEmpty()) rtOpen[key] = null
            return remaining
        }

        for (fill in usable) {
            val a = fill.a
            val key = bookKey(a)
            val book = books.getOrPut(key) { mutableListOf() }
            applySplits(key, a.transactionDate)
            val direction = fill.rollDirection
            if (isOptionSymbol(a.symbol) && isMultileg(a) && direction != null) {
                // Roll: close this contract (book, then carried-forward legs) and carry
                // the same quantity to the unposted new leg. A debit belongs to the
                // closed leg's exit, a credit to the new leg's entry.
                val cash = a.netCashAmount
                val per = if (fill.qty > 0) abs(cash) / (fill.qty * 100.0) else 0.0
                val debit = cash < 0
                val exitPx = if ((direction == "SHORT") == debit) per else 0.0
                val entryPx = if ((direction == "SHORT") != debit) per else 0.0
                a.unitPrice = exitPx
                val before = closed.size
                fill.rtBefore = if (book.isNotEmpty()) rtOpen[key] else null
                var remaining = closeAgainst(key, fill, a, fill.qty)
                remaining = closeRolled(fill, a, remaining, direction)
                val moved = fill.qty - remaining
                rolledKeys.add(rollKey(a))
                if (moved > EPS) {
                    for (i in before until closed.size) {
                        if (!closed[i].flags.contains("rolled")) closed[i].flags.add("rolled")
                    }
                    val chainRt = fill.rtBefore ?: rolledOf(a).rt ?: (if (closed.size > before) closed[before].rt else null)
                    rolledOf(a).rt = chainRt
                    rolledOf(a).lots[direction]!!.add(Lot(
                        moved, entryPx, a.transactionDate, a.occurredAt, 0.0, direction,
                        a.accountId, fifoAccount(a), a.symbol, a.name, a.currency, "Options", a.id, "", chainRt, mutableListOf("rolled-in")))
                }
                if (remaining > EPS) {
                    // nothing to roll: this multileg simply opened a position
                    val opening = if (debit) "LONG" else "SHORT"
                    a.unitPrice = per
                    fill.side = if (opening == "LONG") "BUY" else "SELL"
                    if (book.isEmpty() || rtOpen[key] == null) rtOpen[key] = "rt:" + a.id
                    book.add(Lot(
                        remaining, per, a.transactionDate, a.occurredAt, 0.0, opening,
                        a.accountId, fifoAccount(a), a.symbol, a.name, a.currency, "Options", a.id, a.securityId, rtOpen[key], a.flags.toMutableList()))
                }
                continue
            }
            if (a.flags.contains("transfer-out")) {
                // coins sent out of the account leave at cost: off the open lots
                // first-in first-out, no slice, no P&L, not a fill of the trade
                var remaining = fill.qty
                while (remaining > EPS && book.isNotEmpty() && book[0].direction == "LONG") {
                    val lot = book[0]
                    val matched = min(lot.qty, remaining)
                    lot.commission *= if (lot.qty > 0) (lot.qty - matched) / lot.qty else 0.0
                    lot.qty -= matched
                    remaining -= matched
                    if (lot.qty <= EPS) book.removeAt(0)
                }
                if (book.isEmpty()) rtOpen[key] = null
                continue
            }
            fill.rtBefore = if (book.isNotEmpty()) rtOpen[key] else null
            var remaining = closeAgainst(key, fill, a, fill.qty)
            if (remaining > EPS && isOptionSymbol(a.symbol)) {
                remaining = closeRolled(fill, a, remaining, if (fill.side == "BUY") "SHORT" else "LONG")
            }
            if (remaining > EPS && fill.side == "SELL") {
                for ((dk, dbook) in books) {
                    if (dbook.isEmpty() || dk == key) continue
                    val bits = dk.split("::")
                    if (bits[0] != fifoAccount(a) || bits[2] != a.currency) continue
                    if (!tickerWasReplaced(replaced, bits[0], bits[1], bits[2], a.transactionDate)) continue
                    remaining = closeAgainst(dk, fill, a, remaining, a.symbol)
                    if (remaining <= EPS) break
                }
            }
            if (remaining > EPS && fill.side == "SELL" && openingDirection(a, fill.side) == null && dust(remaining, fill, a)) {
                remaining = 0.0
            }
            if (remaining > EPS) {
                val opening = openingDirection(a, fill.side)
                if (opening != null) {
                    if (book.isEmpty() || rtOpen[key] == null) rtOpen[key] = "rt:" + a.id
                    book.add(Lot(
                        remaining, a.unitPrice, a.transactionDate, a.occurredAt,
                        if (fill.qty > 0) a.commission * (remaining / fill.qty) else 0.0, opening,
                        a.accountId, fifoAccount(a), a.symbol, a.name, a.currency, if (a.kind.isEmpty()) kindOf(a) else a.kind,
                        a.id, a.securityId, rtOpen[key], a.flags.toMutableList()))
                } else {
                    unmatched.add(Unmatched(a.symbol, a.currency, fill.side, remaining, a.unitPrice, a.transactionDate, a.description, a.accountId, fifoAccount(a), a.id))
                }
            }
        }

        for (key in books.keys.toList()) applySplits(key, "9999-12-31")
        for (pool in rolled.values) {
            for (direction in listOf("LONG", "SHORT")) {
                for (lot in pool.lots[direction]!!) {
                    if (lot.qty <= EPS) continue
                    // the closing leg of this roll was never posted; the credit (or
                    // nothing, for a debit roll) is what it earned
                    val pa = Act()
                    pa.id = "roll-out:" + lot.activityId
                    pa.unitPrice = 0.0
                    pa.commission = 0.0
                    pa.transactionDate = lot.date
                    pa.occurredAt = lot.whenAt
                    pa.name = lot.name
                    pa.flags = mutableListOf("rolled-out")
                    val pseudo = Fill(pa, if (direction == "SHORT") "BUY" else "SELL", lot.qty)
                    lot.rt = lot.rt ?: ("rt:" + lot.activityId)
                    val s = makeSlice(lot, pseudo, pa, lot.qty)
                    s.sellActivityId = ""
                    closed.add(s)
                }
            }
        }
        val openLots = mutableListOf<Lot>()
        for (book in books.values) {
            for (lot in book) {
                if (lot.qty <= 1e-6) continue
                // crypto residue from in-kind fees: a lot worth under a dollar is not a position
                if (lot.kind == "Crypto" && !lot.flags.contains("reward") && lot.qty * lot.price < 1.0) continue
                openLots.add(lot.copy())
            }
        }
        closed.sortWith(compareBy({ it.exitDate }, { it.id }))
        val folded2 = foldOptionRolls(closed, openLots)
        return FifoResult(folded2, openLots, unmatched)
    }

    /** Same-day cover + new short on the same underlying is a roll: fold the
     * cover's P&L into the far contract's basis and drop the cover row. */
    fun foldOptionRolls(closed: MutableList<Slice>, openLots: MutableList<Lot>): List<Slice> {
        if (closed.isEmpty()) return closed
        fun rollBook(t: Slice) = listOf(if (t.account.isEmpty()) t.accountType else t.account, t.currency, underlyingSymbol(t.symbol)).joinToString("::")
        fun dayOf(s: String) = s.take(10)

        val covers = closed.filter { it.openDirection == "SHORT" && isOptionSymbol(it.symbol) }
            .sortedWith(compareBy({ dayOf(it.entryDate) }, { dayOf(it.exitDate) }, { it.id }))
        val drop = HashSet<String>()
        for (cover in covers) {
            if (drop.contains(cover.id)) continue
            val d = dayOf(cover.exitDate)
            if (d.isEmpty()) continue
            val under = underlyingSymbol(cover.symbol)
            if (under.isEmpty() || under == "—") continue
            val ck = rollBook(cover)
            val closedCands = closed.filter { t ->
                t.id != cover.id && !drop.contains(t.id) && t.openDirection == "SHORT" && isOptionSymbol(t.symbol) &&
                    t.symbol != cover.symbol && rollBook(t) == ck && dayOf(t.entryDate) == d
            }
            val openCands = openLots.filter { l ->
                l.direction == "SHORT" && isOptionSymbol(l.symbol) && l.symbol != cover.symbol &&
                    listOf(l.accountType, l.currency, underlyingSymbol(l.symbol)).joinToString("::") == ck && dayOf(l.date) == d
            }
            val cq = abs(cover.quantity)
            if (closedCands.isNotEmpty()) {
                val row = closedCands.sortedWith(compareBy({ abs(abs(it.quantity) - cq) }, { it.symbol }))[0]
                val qty = abs(row.quantity)
                if (!(qty > 0)) continue
                val adj = cover.pnl / (qty * optionMultiplier(row.symbol))
                row.entryPrice += adj
                val mult = optionMultiplier(row.symbol)
                val raw = (if (row.openDirection == "SHORT") row.entryPrice - row.exitPrice else row.exitPrice - row.entryPrice) * qty * mult
                row.pnl = raw - row.commission
                row.pnlCad = row.pnl
                row.id = stableTradeId(row)
                if (!row.flags.contains("rolled")) row.flags.add("rolled")
            } else if (openCands.isNotEmpty()) {
                val row = openCands.sortedWith(compareBy({ abs(abs(it.qty) - cq) }, { it.symbol }))[0]
                val qty = abs(row.qty)
                if (!(qty > 0)) continue
                val adj = cover.pnl / (qty * optionMultiplier(row.symbol))
                row.price += adj
                if (!row.flags.contains("rolled")) row.flags.add("rolled")
            } else {
                continue
            }
            drop.add(cover.id)
        }
        if (drop.isNotEmpty()) return closed.filter { !drop.contains(it.id) }
        return closed
    }

    /** 'LUNR 29AUG25 11.50 CALL' -> '2025-08-29'. */
    fun optionExpiry(symbol: String): String {
        val u = spaced(symbol)
        val m = EXPIRY_RE.find(u) ?: return ""
        val day = m.groupValues[1]
        val mon = m.groupValues[2]
        val yr = m.groupValues[3]
        val month = MONTHS.indexOf(mon.lowercase().replaceFirstChar { it.uppercase() })
        if (month < 0) return ""
        return String.format(Locale.ROOT, "20%s-%02d-%s", yr, month + 1, day)
    }

    /** An assigned short option delivers shares, but Wealthsimple posts only
     * the option row (with the strike cash on it). Add the share leg. */
    fun synthesizeAssignmentShares(activities: List<Act>, securities: Securities): List<Act> {
        val out = mutableListOf<Act>()
        for (a in activities) {
            if (a.category != "option_event" || compact(a.activityType) != "ASSIGN") continue
            val symbol = a.symbol
            if (!isOptionSymbol(symbol)) continue
            val contracts = abs(a.quantity)
            if (contracts <= 0) continue
            val shares = contracts * 100
            val cash = a.netCashAmount
            var strike = if (abs(cash) > EPS) abs(cash) / shares else 0.0
            if (strike <= 0) {
                STRIKE_RE.find(spaced(symbol))?.let { strike = it.groupValues[1].toDoubleOrNull() ?: 0.0 }
            }
            if (strike <= 0) continue
            val up = symbol.uppercase().trimEnd()
            val isCall = up.endsWith("CALL") || up.endsWith(" C")
            val sell = if (abs(cash) <= EPS) isCall else cash > 0
            val under = underlyingSymbol(symbol)
            val sec = securities.byId[a.securityId]
            val n = Act()
            n.id = "assign-shares:" + a.id
            n.occurredAt = if (a.occurredAt.isEmpty()) a.transactionDate + "T21:30:00+00:00" else a.occurredAt
            n.transactionDate = a.transactionDate
            n.accountId = a.accountId
            n.fifoId = if (a.fifoId.isEmpty()) a.accountId else a.fifoId
            n.accountType = a.accountType
            n.activityType = "Trade"
            n.activitySubType = if (sell) "SELL" else "BUY"
            n.description = (if (sell) "Called away" else "Put to you") + ": $shares $under @ $strike"
            n.direction = if (sell) "CREDIT" else "DEBIT"
            n.symbol = under
            n.name = under
            n.currency = a.currency
            n.quantity = if (sell) -shares else shares
            n.unitPrice = strike
            n.commission = 0.0
            n.netCashAmount = if (sell) shares * strike else -shares * strike
            n.category = "trade"
            n.rawType = "OPTIONS_ASSIGN_SHARES"
            n.securityId = sec?.underlyingId ?: ""
            n.kind = "Shares"
            n.flags = mutableListOf("assignment")
            out.add(n)
        }
        return out
    }

    /** Wealthsimple does not always post an expiry row. An option lot still
     * open after its expiry date is closed at $0 on that date. */
    fun synthesizeExpiries(openLots: List<Lot>, today: String): List<Act> {
        val out = mutableListOf<Act>()
        val seen = HashSet<String>()
        for (lot in openLots) {
            val exp = optionExpiry(lot.symbol)
            if (exp.isEmpty() || exp >= today) continue
            val key = lot.accountType + "\u0001" + lot.symbol + "\u0001" + lot.currency
            if (!seen.add(key)) continue
            val qty = openLots.filter { it.accountType + "\u0001" + it.symbol + "\u0001" + it.currency == key && it.direction == lot.direction }.sumOf { it.qty }
            if (qty <= EPS) continue
            val short = lot.direction == "SHORT"
            val n = Act()
            n.id = "expiry:${lot.accountType}|${lot.symbol}|${lot.currency}"
            n.occurredAt = exp + "T21:30:00+00:00"
            n.transactionDate = exp
            n.accountId = lot.accountId
            n.fifoId = lot.accountId
            n.accountType = lot.accountType
            n.activityType = "EXPIR"
            n.activitySubType = if (short) "BUY" else "SELL"
            n.description = "Expired (assumed): " + lot.symbol
            n.symbol = lot.symbol
            n.name = lot.name
            n.currency = lot.currency
            n.quantity = if (short) qty else -qty
            n.unitPrice = 0.0
            n.commission = 0.0
            n.netCashAmount = 0.0
            n.category = "option_event"
            n.rawType = if (short) "OPTIONS_SHORT_EXPIRY" else "OPTIONS_EXPIRY"
            n.securityId = lot.securityId
            n.kind = "Options"
            n.flags = mutableListOf("assumed-expiry")
            out.add(n)
        }
        return out
    }

    // MARK: FX

    fun rateOn(fx: Map<String, Double>, day: String): Double {
        var d = day.take(10)
        if (d.isEmpty()) return FX_FALLBACK
        repeat(12) {
            val r = fx[d]
            if (r != null && r > 0) return r
            d = shiftDate(d, -1)
        }
        return FX_FALLBACK
    }

    fun toCad(fx: Map<String, Double>, amount: Double, currency: String, day: String): Double {
        val ccy = (if (currency.isEmpty()) "CAD" else currency).uppercase()
        if (ccy != "USD") return amount
        return amount * rateOn(fx, day)
    }

    fun applyFx(slices: List<Slice>, fx: Map<String, Double>) {
        for (t in slices) {
            val ccy = (if (t.currency.isEmpty()) "CAD" else t.currency).uppercase()
            if (ccy != "USD") {
                t.pnlCad = t.pnl
                t.feesCad = t.commission
                continue
            }
            val qty = t.quantity
            val mult = optionMultiplier(t.symbol)
            val entryC = t.entryCommission
            val exitC = t.exitCommission
            val entryNotional = t.entryPrice * qty * mult
            val exitNotional = t.exitPrice * qty * mult
            t.pnlCad = if (t.openDirection == "SHORT") {
                toCad(fx, entryNotional - entryC, ccy, t.entryDate) - toCad(fx, exitNotional + exitC, ccy, t.exitDate)
            } else {
                toCad(fx, exitNotional - exitC, ccy, t.exitDate) - toCad(fx, entryNotional + entryC, ccy, t.entryDate)
            }
            t.feesCad = toCad(fx, entryC, ccy, t.entryDate) + toCad(fx, exitC, ccy, t.exitDate)
        }
    }

    // MARK: securities / exchange labels

    private val EXCH_ALIAS = mapOf(
        "TSXV" to "TSX-V", "TSX-V" to "TSX-V", "TSX VENTURE" to "TSX-V", "CDNX" to "TSX-V", "VENTURE" to "TSX-V",
        "TORONTO" to "TSX", "TSX" to "TSX", "CBOE CANADA" to "Cboe Canada", "CBOE CA" to "Cboe Canada", "NEO" to "Cboe Canada",
    )
    private val MIC_MAP = mapOf(
        "XTSV" to "TSX-V", "XTSX" to "TSX", "XNAS" to "NASDAQ", "XNYS" to "NYSE", "XASE" to "NYSE American",
        "ARCX" to "NYSE Arca", "XCNQ" to "CSE", "NEOE" to "Cboe Canada",
    )

    fun exchangeLabel(sec: Security?): String {
        val raw = (sec?.primaryExchange ?: "").trim()
        val up = raw.uppercase()
        EXCH_ALIAS[up]?.let { return it }
        if (raw.isNotEmpty()) return raw
        return MIC_MAP[(sec?.primaryMic ?: "").uppercase()] ?: ""
    }

    fun listingTicker(sym: String): String {
        val s = sym.trim()
        return LISTING_RE.find(s)?.groupValues?.get(1) ?: s
    }

    fun isAlphaVenue(sec: Security?): Boolean {
        val exch = (sec?.primaryExchange ?: "").uppercase()
        val mic = (sec?.primaryMic ?: "").uppercase()
        return exch == "ALPHA EXCHANGE" || exch == "ALPHA" || mic == "XATS"
    }

    class Securities(rows: List<Security>) {
        val byId = LinkedHashMap<String, Security>()

        init {
            for (r in rows) if (r.id.isNotEmpty()) byId[r.id] = r
        }

        /** Security id -> currency for the cash rows Wealthsimple lists as securities (CAD, USD). */
        fun cashCurrencies(): Map<String, String> {
            val out = HashMap<String, String>()
            for ((sid, sec) in byId) {
                val sym = sec.symbol.uppercase()
                if (sym == "CAD" || sym == "USD" || sid.startsWith("sec-c-")) out[sid] = sec.currency.uppercase().ifEmpty { sym }
            }
            return out
        }

        fun preferred(sec: Security?): Security? {
            if (sec == null || !isAlphaVenue(sec)) return sec
            val sym = listingTicker(sec.symbol)
            val ccy = sec.currency
            if (sym.isEmpty()) return sec
            for (other in byId.values) {
                if (other === sec || other.underlyingId.isNotEmpty()) continue
                if (listingTicker(other.symbol) != sym) continue
                if (ccy.isNotEmpty() && other.currency.isNotEmpty() && other.currency != ccy) continue
                if (isAlphaVenue(other) || exchangeLabel(other).isEmpty()) continue
                return other
            }
            return sec
        }

        fun listing(securityId: String): Security? {
            val sec = byId[securityId]
            val under = if (sec != null && sec.underlyingId.isNotEmpty()) byId[sec.underlyingId] else null
            return preferred(under ?: sec)
        }

        fun exchange(securityId: String) = exchangeLabel(listing(securityId))

        fun name(securityId: String, fallback: String = ""): String {
            val n = listing(securityId)?.name ?: ""
            return if (n.isEmpty()) fallback else n
        }
    }

    // MARK: trades (round trips), positions, cashflow

    fun fillRow(a: Act): FillRow {
        val (day, clock) = whenParts(if (a.occurredAt.isEmpty()) a.transactionDate else a.occurredAt)
        val side = tradeSide(a)
        val qty = abs(a.quantity)
        val f = FillRow()
        f.id = a.id
        f.whenAt = if (a.occurredAt.isEmpty()) a.transactionDate else a.occurredAt
        f.date = if (day.isEmpty()) a.transactionDate else day
        f.time = clock
        f.side = side
        f.sub = a.activitySubType
        f.qty = if (side == "SELL") -qty else qty
        f.price = a.unitPrice
        f.amount = a.netCashAmount
        f.fees = a.commission
        f.currency = a.currency
        f.flags = a.flags.toList()
        return f
    }

    fun collapseTrade(gid: String, members: List<Slice>, status: String, actsById: Map<String, Act>, securities: Securities, journal: Map<String, JournalEntry> = emptyMap()): Trade {
        val slices = members.sortedWith(compareBy({ it.exitDate }, { it.entryDate }, { sliceMemberKey(it) }))
        val t0 = slices[0]
        val qty = slices.sumOf { it.quantity }
        val entryNotional = slices.sumOf { it.entryPrice * it.quantity }
        val exitNotional = slices.sumOf { it.exitPrice * it.quantity }
        val pnl = slices.sumOf { it.pnl }
        val pnlCad = slices.sumOf { it.pnlCad }
        val fees = slices.sumOf { it.commission }
        val feesCad = slices.sumOf { it.feesCad }
        val entryDate = slices.minOf { it.entryDate }
        val exitDate = slices.maxOf { it.exitDate }
        val entryWhen = slices.minOf { if (it.entryWhen.isEmpty()) it.entryDate else it.entryWhen }
        val exitWhen = slices.maxOf { if (it.exitWhen.isEmpty()) it.exitDate else it.exitWhen }
        val mult = optionMultiplier(t0.symbol)
        val entry = if (qty != 0.0) entryNotional / qty else 0.0
        val exitPx = if (qty != 0.0) exitNotional / qty else t0.exitPrice
        val basis = abs(entry * qty * mult)
        val secId = slices.firstOrNull { it.securityId.isNotEmpty() }?.securityId ?: ""
        val ids = mutableListOf<String>()
        for (s in slices) {
            for (k in listOf(s.buyActivityId, s.sellActivityId)) if (k.isNotEmpty() && k !in ids) ids.add(k)
        }
        val fills = ids.mapNotNull { actsById[it] }.map { fillRow(it) }.toMutableList()
        // label each fill by what it did in this trade, not by the broker's order
        // type: the open/close order types are option language, shares and crypto
        // fills are simply bought or sold
        val openedIds = slices.map { it.buyActivityId }.toSet()
        val closedIds = slices.map { it.sellActivityId }.toSet()
        for (f in fills) {
            val opened = openedIds.contains(f.id)
            val closed = closedIds.contains(f.id)
            val side = if (f.side == "BUY") "BUY" else "SELL"
            if (t0.kind != "Options") {
                f.sub = side + (if (opened && closed) " (close + open)" else "")
            } else if (closed && !opened) {
                f.sub = "$side TO CLOSE"
            } else if (opened && !closed) {
                f.sub = "$side TO OPEN"
            } else if (opened && closed) {
                f.sub = "$side (close + open)"
            }
        }
        fills.sortByDescending { it.whenAt }
        val t = Trade()
        t.id = gid
        t.status = status
        t.symbol = t0.symbol
        t.underlying = underlyingSymbol(t0.symbol)
        t.name = securities.name(secId, if (t0.name.isEmpty()) t0.symbol else t0.name)
        t.exchange = if (t0.kind != "Crypto") securities.exchange(secId) else "Crypto"
        t.kind = t0.kind
        t.currency = t0.currency
        t.account = t0.account
        t.accountId = t0.accountId
        t.securityId = secId
        t.side = if (t0.openDirection == "LONG") "SELL" else "COVER"
        t.openDirection = t0.openDirection
        t.qty = qty
        t.mult = mult
        t.entry = entry
        t.exit = exitPx
        t.entryDate = entryDate
        t.exitDate = exitDate
        t.entryWhen = entryWhen
        t.exitWhen = exitWhen
        t.holdDays = daysBetween(entryDate, exitDate)
        t.pnl = pnl
        t.pnlCad = pnlCad
        t.fees = fees
        t.feesCad = feesCad
        t.pnlPct = if (basis > 0) pnl / basis else null
        t.legCount = slices.size
        t.fills = fills
        t.flags = slices.flatMap { it.flags }.toSet().sorted()
        val note = journal[gid]
        t.grade = note?.grade ?: ""
        t.thesis = note?.thesis ?: ""
        t.tags = note?.tags ?: emptyList()
        return t
    }

    fun buildTrades(closed: List<Slice>, actsById: Map<String, Act>, securities: Securities, journal: Map<String, JournalEntry> = emptyMap()): List<Trade> {
        val byRt = LinkedHashMap<String, MutableList<Slice>>()
        for (s in closed) {
            val rt = s.rt ?: ("rt:" + sliceMemberKey(s))
            byRt.getOrPut(rt) { mutableListOf() }.add(s)
        }
        return byRt.entries.map { collapseTrade(it.key, it.value, "closed", actsById, securities, journal) }
            .sortedWith(compareByDescending<Trade> { it.exitDate }.thenByDescending { it.id })
    }

    /** symbol -> (price, date) from the newest fill with a price. */
    fun lastFillPrices(activities: List<Act>): Map<String, Pair<Double, String>> {
        val out = HashMap<String, Pair<Double, String>>()
        for (a in activities.sortedWith(compareBy({ it.transactionDate }, { it.occurredAt }))) {
            if (a.category != "trade" && a.category != "option_event") continue
            if (a.unitPrice > 0 && a.symbol.isNotEmpty()) out[a.symbol] = Pair(a.unitPrice, a.transactionDate)
        }
        return out
    }

    fun buildPositions(openLots: List<Lot>, lastPrices: Map<String, Pair<Double, String>>, securities: Securities, today: String, quotes: Map<String, Quote>, journal: Map<String, JournalEntry> = emptyMap(), actsById: Map<String, Act> = emptyMap()): List<Position> {
        val groups = LinkedHashMap<String, MutableList<Lot>>()
        for (lot in openLots) {
            val k = listOf(lot.symbol, lot.accountType, lot.currency, lot.direction).joinToString("\u0001")
            groups.getOrPut(k) { mutableListOf() }.add(lot)
        }
        val rows = mutableListOf<Position>()
        for ((k, group) in groups) {
            val lots = group.sortedWith(compareBy({ it.date }, { it.whenAt }))
            val bits = k.split("\u0001")
            val symbol = bits[0]; val account = bits[1]; val currency = bits[2]; val direction = bits[3]
            val mult = optionMultiplier(symbol)
            val qty = lots.sumOf { it.qty }
            if (qty <= 1e-9) continue
            val cost = lots.sumOf { it.qty * it.price * mult }
            val fees = lots.sumOf { it.commission }
            val secId = lots.firstOrNull { it.securityId.isNotEmpty() }?.securityId ?: ""
            val last = lastPrices[symbol]
            var lastPx = last?.first ?: (if (qty != 0.0) cost / (qty * mult) else 0.0)
            var lastAt = last?.second ?: ""
            var priceSource = "fill"
            val quote = quotes[symbol]
            val qpx = quote?.price
            if (quote != null && qpx != null && qpx != 0.0) {
                lastPx = qpx
                lastAt = quote.fetchedAt
                priceSource = "quote"
            }
            val mv = qty * lastPx * mult
            val unreal = if (direction == "LONG") mv - cost else cost - mv
            val held = lots.sumOf { it.qty * daysBetween(it.date, today) }
            val legacyPid = "pos:" + listOf(account, symbol, currency).joinToString("|")
            val p = Position()
            p.id = lots[0].rt ?: legacyPid
            p.symbol = symbol
            p.underlying = underlyingSymbol(symbol)
            p.name = securities.name(secId, if (lots[0].name.isEmpty()) symbol else lots[0].name)
            p.exchange = if (lots[0].kind != "Crypto") securities.exchange(secId) else "Crypto"
            p.kind = lots[0].kind
            p.account = account
            p.accountId = lots[0].accountId
            p.currency = currency
            p.securityId = secId
            p.short = direction == "SHORT"
            p.qty = qty
            p.mult = mult
            p.avg = if (qty != 0.0) cost / (qty * mult) else 0.0
            p.cost = cost
            p.fees = fees
            p.last = lastPx
            p.lastAt = lastAt
            p.priceSource = priceSource
            p.priceChange = quote?.priceChange
            p.percentChange = quote?.percentChange
            // the day's move on the whole position, in its own currency, from the quote's change
            p.dayChange = quote?.priceChange?.let { qty * it * mult * (if (direction == "SHORT") -1 else 1) }
            p.mv = mv
            p.unreal = unreal
            p.unrealPct = if (cost != 0.0) unreal / cost else null
            p.held = if (qty != 0.0) round(held / qty).toInt() else 0
            p.opened = lots[0].date
            p.rt = lots[0].rt
            p.lots = lots.map { PositionLot(it.date, it.qty, it.price, it.qty * it.price * mult, daysBetween(it.date, today), it.flags.toList(), it.activityId) }
            // A position and the trade it becomes when it closes share one journal
            // entry: both are keyed by the round trip that opened the position.
            val note = journal[p.id] ?: journal[legacyPid]
            p.grade = note?.grade ?: ""
            p.thesis = note?.thesis ?: ""
            p.tags = note?.tags ?: emptyList()
            p.fills = lots.mapNotNull { actsById[it.activityId] }.map { fillRow(it) }.sortedByDescending { it.whenAt }
            rows.add(p)
        }
        val book = rows.sumOf { abs(it.cost) }
        for (r in rows) r.alloc = if (book != 0.0) abs(r.cost) / book else 0.0
        return rows.sortedByDescending { it.alloc }
    }

    fun buildCashflow(activities: List<Act>, securities: Securities, fx: Map<String, Double>): List<CashRow> {
        val rows = mutableListOf<CashRow>()
        for (a in activities) {
            val cat = a.category
            val raw = compact(a.rawType)
            val at = compact(a.activityType)
            val cash = a.netCashAmount
            val kind = when {
                cat == "dividend" -> "Dividend"
                cat == "interest" -> "Interest"
                raw == "WITHHOLDINGTAX" || at == "WITHHOLDINGTAX" -> "Withholding tax"
                raw == "INTERESTCHARGE" || at == "INTERESTCHARGE" -> "Interest charge"
                else -> continue
            }
            if (abs(cash) < EPS) continue
            val (day, clock) = whenParts(if (a.occurredAt.isEmpty()) a.transactionDate else a.occurredAt)
            var symbol = a.symbol.trim()
            if (symbol.isEmpty() && (kind == "Interest" || kind == "Interest charge")) symbol = "Cash"
            val r = CashRow()
            r.id = a.id
            r.date = if (a.transactionDate.isEmpty()) day else a.transactionDate
            r.time = clock
            r.symbol = if (symbol.isEmpty()) "—" else symbol
            r.name = securities.name(a.securityId, if (a.name != symbol) a.name else "")
            r.kind = kind
            r.account = normAccountName(a.accountType).ifEmpty { a.accountId }
            r.accountId = a.accountId
            r.qty = if (a.quantity != 0.0) a.quantity else null
            r.per = if (a.unitPrice != 0.0) a.unitPrice else null
            r.amount = cash
            r.currency = if (a.currency.isEmpty()) "CAD" else a.currency
            r.amountCad = toCad(fx, cash, a.currency, a.transactionDate)
            rows.add(r)
        }
        return rows.sortedWith(compareByDescending<CashRow> { it.date }.thenByDescending { it.id })
    }

    // MARK: build

    fun buildBase(raw: List<Act>, securityRows: List<Security>, market: Market, today: String, navHistory: List<NavPoint> = emptyList(), navByAccount: Map<String, List<NavPoint>> = emptyMap(), journal: Map<String, JournalEntry> = emptyMap(), accounts: List<AccountInfo> = emptyList(), balances: List<BalanceRow> = emptyList(), margin: List<MarginRow> = emptyList()): Base {
        var acts = normalizeActivities(raw)
        val securities = Securities(securityRows)
        val delivered = synthesizeAssignmentShares(acts, securities)
        if (delivered.isNotEmpty()) acts = acts + delivered
        var fifo = matchFifo(acts)
        val synthetic = synthesizeExpiries(fifo.open, today)
        if (synthetic.isNotEmpty()) {
            acts = acts + synthetic
            fifo = matchFifo(acts)
        }
        val actsById = HashMap<String, Act>()
        for (a in acts) actsById[a.id] = a
        applyFx(fifo.closed, market.fx)
        val base = Base()
        base.today = today
        base.fx = market.fx
        base.benchmark = market.benchmark
        val benchmarks = LinkedHashMap(market.benchmarks)
        if (!benchmarks.containsKey("SP500")) benchmarks["SP500"] = market.benchmark
        base.benchmarks = benchmarks
        base.equity = ModelView.equitySeries(navHistory)
        base.equityByAccount = navByAccount.entries.associate { normAccountName(it.key) to ModelView.equitySeries(it.value) }
        base.journal = journal
        base.distributions = market.distributions
        base.quotes = market.quotes
        base.activities = acts
        base.closed = fifo.closed
        base.openLots = fifo.open
        base.unmatched = fifo.unmatched
        base.trades = buildTrades(fifo.closed, actsById, securities, journal)
        base.positions = buildPositions(fifo.open, lastFillPrices(acts), securities, today, market.quotes, journal, actsById)
        base.cashflow = buildCashflow(acts, securities, market.fx)
        base.accounts = accounts.map { AccountInfo(it.id, normAccountName(it.name), it.currency, it.nav, it.type, it.status) }
        base.balances = balances
        base.margin = margin
        base.cashCurrencies = securities.cashCurrencies()
        return base
    }

    fun kpi(trades: List<Trade>): KPI {
        val vals = trades.map { it.pnlCad }
        val wins = vals.filter { it > 0 }
        val losses = vals.filter { it < 0 }
        val be = vals.filter { it == 0.0 }
        val gw = wins.sum()
        val gl = abs(losses.sum())
        val n = vals.size
        val total = vals.sum()
        return KPI(
            realized = total, count = n, wins = wins.size, losses = losses.size, breakeven = be.size,
            winRate = if (n > 0) wins.size.toDouble() / n else null,
            grossWin = gw, grossLoss = gl,
            profitFactor = if (gl > 0) gw / gl else (if (gw > 0) null else 0.0),
            profitFactorInfinite = gl == 0.0 && gw > 0,
            expectancy = if (n > 0) total / n else null,
            avgWin = if (wins.isEmpty()) 0.0 else gw / wins.size,
            avgLoss = if (losses.isEmpty()) 0.0 else -gl / losses.size,
            fees = trades.sumOf { it.feesCad },
            avgHold = if (n > 0) trades.sumOf { it.holdDays }.toDouble() / n else null,
            openCount = trades.count { it.status == "open" },
        )
    }

    fun monthLabel(key: String): String {
        val m = key.substring(5, 7).toInt()
        return MONTHS[m - 1] + " '" + key.substring(2, 4)
    }

    /** Verified payment frequency from actual payment dates (any order).
     * Only the most recent gaps count (the last three), so a fund that changes
     * its schedule is re-read after two payments at the new cadence. */
    fun paymentsPerYear(dates: List<String>): Int? {
        val days = dates.map { it.take(10) }.filter { it.isNotEmpty() }.toSet().sorted()
        if (days.size < 2) return null
        val gaps = days.zipWithNext { a, b -> daysBetween(a, b) }.filter { it > 0 }.takeLast(3).sorted()
        if (gaps.isEmpty()) return null
        val median = gaps[gaps.size / 2]
        val perYear = 365.25 / median
        var best = SCHEDULES[0]
        for (s in SCHEDULES) if (abs(s - perYear) < abs(best - perYear)) best = s
        return best
    }

    private class Rate(val per: Double, val freq: Int, val annual: Double, val verified: Boolean, val source: String)
}
