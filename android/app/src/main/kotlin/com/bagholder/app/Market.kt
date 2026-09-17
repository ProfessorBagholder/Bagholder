// Market data on the phone: what the desktop's crates/market fetches server-side,
// fetched here from the same sources with the same rules, and kept in the
// app's files. Every request records its outcome, and that is what a chart's
// empty state reports. Nothing is hand-mapped per ticker.
package com.bagholder.app

import com.bagholder.model.Distribution
import com.bagholder.model.Model
import com.bagholder.model.Quote
import com.bagholder.model.Trade
import org.json.JSONArray
import org.json.JSONObject
import java.net.HttpURLConnection
import java.net.URL
import java.text.SimpleDateFormat
import java.time.LocalDate
import java.time.ZoneId
import java.time.ZonedDateTime
import java.util.Date
import java.util.Locale
import java.util.TimeZone
import kotlin.math.round

/** A daily bar as the sources answer it. */
data class DailyBar(val date: String, val open: Double?, val high: Double?, val low: Double?, val close: Double, val volume: Double?)

/** What a held instrument needs for a quote or a chart. */
data class Instrument(val symbol: String, val exchange: String, val currency: String, val kind: String)

class HttpStatusException(val code: Int) : Exception("HTTP $code")
class BackoffException : Exception("backing off")

object MarketData {
    private const val TMX_URL = "https://app-money.tmx.com/graphql"
    private const val TMX_QUOTE_QUERY = "query getQuoteBySymbol(\$symbol: String, \$locale: String) { getQuoteBySymbol(symbol: \$symbol, locale: \$locale) { symbol name exchangeName price priceChange percentChange prevClose currency dividendFrequency dividendYield dividendAmount exDividendDate } }"
    private const val TMX_DIVIDENDS_QUERY = "query getDividendsForSymbol(\$symbol: String!, \$page: Int, \$batch: Int) { dividends: getDividendsForSymbol(symbol: \$symbol, page: \$page, batch: \$batch) { dividends { exDate payableDate amount currency } } }"
    private const val TMX_HISTORY_QUERY = "query getTimeSeriesData(\$symbol: String!, \$freq: String, \$interval: Int, \$start: String, \$end: String) { getTimeSeriesData(symbol: \$symbol, freq: \$freq, interval: \$interval, start: \$start, end: \$end) { dateTime open high low close volume } }"
    private val TMX_HEADERS = mapOf("locale" to "en", "Origin" to "https://money.tmx.com", "Referer" to "https://money.tmx.com/")
    private const val TMX_BATCH = 24
    private const val UA = "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/17.0 Safari/605.1.15"
    private val US_EXCHANGES = setOf("NASDAQ", "NYSE", "NYSE AMERICAN", "NYSE ARCA", "BATS", "AMEX", "ARCA", "CBOE", "IEX")
    private val CBOE_CANADA = setOf("CBOE CANADA", "NEO")
    private val CANADIAN = setOf("TSX", "TSX-V", "TSXV", "CSE", "CBOE CANADA", "NEO", "ALPHA EXCHANGE")
    private val TMX_FORMS = mapOf("CAD" to listOf("", ":CNX", ":AQL"), "USD" to listOf(":US"))
    private val TMX_VENUE_OF_FORM = mapOf("" to listOf("TORONTO STOCK EXCHANGE", "TSX VENTURE"), ":CNX" to listOf("CANADIAN SECURITIES EXCHANGE"), ":AQL" to listOf("CBOE", "NEO"), ":US" to listOf("NYSE", "NASDAQ", "NEW YORK"))
    private val TMX_INDICES = mapOf("TSX" to "^TSX", "TSX60" to "^TX60")
    private val YAHOO_SUFFIX = mapOf("TSX" to ".TO", "TSX-V" to ".V", "TSXV" to ".V", "CSE" to ".CN", "CBOE CANADA" to ".NE", "NEO" to ".NE")
    private val YAHOO_FORMS = mapOf("CAD" to listOf(".TO", ".V", ".CN", ".NE"), "USD" to listOf(""))
    private val YAHOO_HEADERS = mapOf("User-Agent" to "Mozilla/5.0", "Accept" to "application/json")
    private val SOURCE_LABELS = mapOf("tmx" to "TMX Money", "yahoo" to "Yahoo Finance", "coinbase" to "Coinbase", "cboe" to "Cboe", "boc" to "Bank of Canada", "fred" to "FRED")
    private const val COVERAGE_SLACK_DAYS = 7
    private const val QUOTE_REFRESH_MS = 60_000L
    private const val RECORD_STALE_MS = 20 * 3600_000L
    private const val HISTORY_STALE_MS = 20 * 3600_000L
    private const val YAHOO_MIN_INTERVAL_MS = 2000L
    private const val YAHOO_BACKOFF_MS = 600_000L

    private val lock = Any()
    private val health = HashMap<String, Pair<Boolean, String>>()
    private val chartNotes = HashMap<String, List<Triple<String, String, String?>>>()

    // MARK: files

    private fun meta(key: String): String = Store.readJSON("meta.json").optString(key, "")

    private fun setMeta(key: String, value: String) {
        synchronized(lock) {
            val m = Store.readJSON("meta.json")
            m.put(key, value)
            Store.writeJSON("meta.json", m)
        }
    }

    fun nowStamp(): String {
        val f = SimpleDateFormat("yyyy-MM-dd'T'HH:mm:ss'Z'", Locale.US)
        f.timeZone = TimeZone.getTimeZone("UTC")
        return f.format(Date())
    }

    private fun ageMs(stamp: String): Long? {
        if (stamp.isEmpty()) return null
        return try {
            System.currentTimeMillis() - ZonedDateTime.parse(stamp).toInstant().toEpochMilli()
        } catch (e: Exception) {
            null
        }
    }

    private fun today() = Model.todayLocal()

    // MARK: source outcomes

    private fun sourceOf(url: String): String {
        val host = try { URL(url).host.lowercase() } catch (e: Exception) { "" }
        for ((key, needle) in listOf("tmx" to "tmx.com", "yahoo" to "yahoo.com", "coinbase" to "coinbase.com", "cboe" to "cboe.com", "boc" to "bankofcanada.ca", "fred" to "stlouisfed.org")) {
            if (host.contains(needle)) return key
        }
        return host.ifEmpty { "other" }
    }

    private fun describeFailure(e: Exception): String = when (e) {
        is HttpStatusException -> if (e.code == 429) "refused the request (too many)" else "answered with an error (${e.code})"
        is BackoffException -> "refused the request; asked again in ten minutes"
        else -> "could not be reached"
    }

    private fun note(source: String, ok: Boolean, e: Exception? = null) {
        synchronized(lock) { health[source] = Pair(ok, if (ok) "" else describeFailure(e ?: Exception())) }
    }

    // MARK: HTTP

    private fun request(url: String, method: String = "GET", body: JSONObject? = null, headers: Map<String, String> = emptyMap()): String {
        val source = sourceOf(url)
        try {
            val conn = URL(url).openConnection() as HttpURLConnection
            conn.requestMethod = method
            conn.connectTimeout = 30_000
            conn.readTimeout = 30_000
            conn.setRequestProperty("User-Agent", UA)
            conn.setRequestProperty("Accept", if (body == null) "text/csv,application/json,*/*;q=0.8" else "*/*")
            for ((k, v) in headers) conn.setRequestProperty(k, v)
            if (body != null) {
                conn.setRequestProperty("Content-Type", "application/json")
                conn.doOutput = true
                conn.outputStream.use { it.write(body.toString().toByteArray(Charsets.UTF_8)) }
            }
            val status = conn.responseCode
            if (status >= 400) {
                conn.disconnect()
                val e = HttpStatusException(status)
                if (status != 404) note(source, false, e)   // a symbol a source does not carry is not the source failing
                throw e
            }
            val text = conn.inputStream.bufferedReader().use { it.readText() }
            conn.disconnect()
            note(source, true)
            return text
        } catch (e: HttpStatusException) {
            throw e
        } catch (e: Exception) {
            note(source, false, e)
            throw e
        }
    }

    private fun tmx(operation: String, variables: JSONObject, query: String): JSONObject =
        JSONObject(request(TMX_URL, "POST", JSONObject().put("operationName", operation).put("variables", variables).put("query", query), TMX_HEADERS))

    private fun num(v: Any?): Double? = when (v) {
        null, JSONObject.NULL -> null
        is Number -> v.toDouble().takeIf { !it.isNaN() }
        is String -> v.toDoubleOrNull()
        else -> null
    }

    // MARK: TMX symbol forms

    fun tmxSymbol(symbol: String): String {
        var s = symbol.trim().uppercase()
        for (suffix in listOf(".TO", ".V", ".CN", ".NE")) if (s.endsWith(suffix)) s = s.dropLast(suffix.length)
        return s
    }

    fun tmxForm(exchange: String, currency: String): String? {
        val ex = exchange.trim().uppercase()
        val ccy = currency.trim().uppercase()
        if (US_EXCHANGES.contains(ex) || (ex.isEmpty() && ccy == "USD")) return ":US"
        if (CBOE_CANADA.contains(ex)) return ":AQL"
        if (ex == "CSE") return ":CNX"
        if (ex == "TSX" || ex == "TSX-V" || ex == "TSXV") return ""
        if (ccy == "CAD") return ""
        if (ccy == "USD") return ":US"
        return null
    }

    private fun tmxQuoteSymbol(i: Instrument): String? {
        val s = tmxSymbol(i.symbol)
        if (s.isEmpty() || s.contains(" ")) return null
        return s + (tmxForm(i.exchange, i.currency) ?: return null)
    }

    private fun tmxRecordSymbol(symbol: String, exchange: String): String? {
        val s = tmxSymbol(symbol)
        if (s.isEmpty()) return null
        return s + (tmxForm(exchange, "CAD") ?: return null)
    }

    private fun tmxBare(key: String) = key.substringBefore(":")

    private fun tmxRemembered(key: String): String {
        if (key.isEmpty() || key.startsWith("^")) return key
        val v = meta("tmx_form:" + tmxBare(key))
        return if (v.startsWith("@")) tmxBare(key) + v.drop(1) else key
    }

    private fun tmxResolve(key: String): String {
        if (key.isEmpty() || key.startsWith("^")) return key
        val bare = tmxBare(key)
        val suffix = key.drop(bare.length)
        var forms = if (suffix == ":US") TMX_FORMS["USD"]!! else TMX_FORMS["CAD"]!!
        if (forms.contains(suffix)) forms = listOf(suffix) + forms.filter { it != suffix }
        val metaKey = "tmx_form:$bare"
        val v = meta(metaKey)
        if (v.startsWith("@")) return bare + v.drop(1)
        if (v.startsWith("none@") && v.drop(5) > Model.shiftDate(today(), -1)) return ""
        for (form in forms) {
            val cand = bare + form
            val q = try { tmx("getQuoteBySymbol", JSONObject().put("symbol", cand).put("locale", "en"), TMX_QUOTE_QUERY).optJSONObject("data")?.optJSONObject("getQuoteBySymbol") } catch (e: Exception) { null }
            val venue = q?.optString("exchangeName", "")?.uppercase() ?: ""
            if (venue.isNotEmpty() && TMX_VENUE_OF_FORM[form]!!.any { venue.contains(it) }) {
                setMeta(metaKey, "@$form")
                return cand
            }
        }
        setMeta(metaKey, "none@" + today())
        return ""
    }

    private fun <T> tmxLookup(key: String, fn: (String) -> T?): Pair<T?, String> {
        val first = tmxRemembered(key)
        fn(first)?.let { return Pair(it, first) }
        if (key.isEmpty() || key.startsWith("^")) return Pair(null, first)
        val alt = tmxResolve(key)
        if (alt.isNotEmpty() && alt != first) return Pair(fn(alt), alt)
        return Pair(null, first)
    }

    // MARK: quotes

    private fun tmxQuote(key: String): Quote? {
        val data = try { tmx("getQuoteBySymbol", JSONObject().put("symbol", key).put("locale", "en"), TMX_QUOTE_QUERY) } catch (e: Exception) { return null }
        val q = data.optJSONObject("data")?.optJSONObject("getQuoteBySymbol") ?: return null
        val price = num(q.opt("price")) ?: return null
        return Quote(price, num(q.opt("priceChange")), num(q.opt("percentChange")), nowStamp(), q.optString("exDividendDate", "").take(10))
    }

    private fun cboeCAQuote(sym: String): Quote? {
        val d = try { JSONObject(request("https://www-api.cboe.com/ca/equities/securities-1/$sym/quote/")).optJSONObject("data") } catch (e: Exception) { null } ?: return null
        val last = num(d.opt("last")) ?: 0.0
        val prev = num(d.opt("prev_close"))
        val px = if (last > 0) last else prev ?: 0.0
        if (px <= 0) return null
        return Quote(px, num(d.opt("change")), num(d.opt("change_pct")), nowStamp(), "")
    }

    private fun coinbaseSpot(pair: String): Quote? {
        val d = try { JSONObject(request("https://api.coinbase.com/v2/prices/$pair/spot")).optJSONObject("data") } catch (e: Exception) { null } ?: return null
        val px = num(d.opt("amount")) ?: return null
        if (px <= 0) return null
        return Quote(px, null, null, nowStamp(), "")
    }

    private val OCC_WORDY = Regex("^([A-Z][A-Z0-9.]{0,9}) (\\d{1,2})([A-Z]{3})(\\d{2}) (\\d+(?:\\.\\d+)?) (CALL|PUT|C|P)$")
    private val OCC_COMPACT = Regex("^([A-Z][A-Z0-9.]{0,9}) (\\d{6}[CP]\\d{8})$")
    private val MONTHS = listOf("JAN", "FEB", "MAR", "APR", "MAY", "JUN", "JUL", "AUG", "SEP", "OCT", "NOV", "DEC")

    fun occCode(symbol: String): String {
        val u = symbol.trim().uppercase().replace(Regex("\\s+"), " ")
        OCC_COMPACT.find(u)?.let { return it.groupValues[1] + it.groupValues[2] }
        val m = OCC_WORDY.find(u) ?: return ""
        val mi = MONTHS.indexOf(m.groupValues[3])
        if (mi < 0) return ""
        val strike = m.groupValues[5].toDoubleOrNull() ?: return ""
        return m.groupValues[1] + m.groupValues[4] + String.format(Locale.ROOT, "%02d%02d", mi + 1, m.groupValues[2].toInt()) + m.groupValues[6].take(1) + String.format(Locale.ROOT, "%08d", round(strike * 1000).toInt())
    }

    private fun occRoot(code: String) = Regex("^([A-Z][A-Z0-9.]{0,9})\\d{6}[CP]\\d{8}$").find(code)?.groupValues?.get(1) ?: ""

    private fun optionMark(row: JSONObject): Quote? {
        val bid = num(row.opt("bid")) ?: 0.0
        val ask = num(row.opt("ask")) ?: 0.0
        val prev = num(row.opt("prev_day_close"))
        val px = if (bid > 0 && ask > 0) (bid + ask) / 2 else num(row.opt("last_trade_price")) ?: prev
        if (px == null || px <= 0) return null
        return Quote(px, prev?.let { px - it }, prev?.let { (px / it - 1) * 100 }, nowStamp(), "")
    }

    fun quoteSource(i: Instrument): Pair<String, String>? {
        val sym = tmxSymbol(i.symbol)
        val ccy = i.currency.ifEmpty { "CAD" }.uppercase()
        if (sym.isEmpty()) return null
        if (i.kind == "Crypto") return Pair("coinbase", "$sym-$ccy")
        if (i.kind == "Options") {
            val code = occCode(i.symbol)
            return if (code.isNotEmpty() && ccy == "USD") Pair("cboe_options", code) else null
        }
        if (i.kind != "Shares") return null
        if (CBOE_CANADA.contains(i.exchange.uppercase())) return Pair("cboe_ca", sym)
        return tmxQuoteSymbol(i)?.let { Pair("tmx", it) }
    }

    fun loadQuotes(): Map<String, Quote> {
        val o = Store.readJSON("quotes.json")
        val out = HashMap<String, Quote>()
        for (sym in o.keys()) {
            val d = o.optJSONObject(sym) ?: continue
            out[sym] = Quote(num(d.opt("price")), num(d.opt("priceChange")), num(d.opt("percentChange")), d.optString("fetchedAt", ""), d.optString("exDividendDate", ""))
        }
        return out
    }

    private fun saveQuotes(quotes: Map<String, Quote>) {
        val o = JSONObject()
        for ((sym, q) in quotes) {
            o.put(sym, JSONObject().put("price", q.price ?: JSONObject.NULL).put("priceChange", q.priceChange ?: JSONObject.NULL)
                .put("percentChange", q.percentChange ?: JSONObject.NULL).put("fetchedAt", q.fetchedAt).put("exDividendDate", q.exDividendDate))
        }
        Store.writeJSON("quotes.json", o)
    }

    /** Live-ish prices for held positions, at most every minute each. */
    fun refreshQuotes(instruments: List<Instrument>): Map<String, Quote> {
        val quotes = loadQuotes().toMutableMap()
        val chains = HashMap<String, Map<String, JSONObject>>()
        val seen = HashSet<String>()
        for (i in instruments) {
            val sym = tmxSymbol(i.symbol)
            val (source, key) = quoteSource(i) ?: continue
            if (sym.isEmpty() || !seen.add(sym)) continue
            val old = quotes[sym]
            if (old != null) { val a = ageMs(old.fetchedAt); if (a != null && a < QUOTE_REFRESH_MS) continue }
            val q: Quote? = when (source) {
                "tmx" -> tmxLookup(key) { tmxQuote(it) }.first
                "cboe_ca" -> cboeCAQuote(key)
                "coinbase" -> coinbaseSpot(key)
                "cboe_options" -> {
                    val root = occRoot(key)
                    val chain = chains.getOrPut(root) {
                        val out = HashMap<String, JSONObject>()
                        try {
                            val opts = JSONObject(request("https://cdn.cboe.com/api/global/delayed_quotes/options/$root.json")).optJSONObject("data")?.optJSONArray("options") ?: JSONArray()
                            for (k in 0 until opts.length()) { val o = opts.optJSONObject(k) ?: continue; out[o.optString("option", "")] = o }
                        } catch (e: Exception) {
                        }
                        out
                    }
                    chain[key]?.let { optionMark(it) }
                }
                else -> null
            }
            if (q?.price != null) quotes[sym] = if (q.exDividendDate.isEmpty()) q.copy(exDividendDate = old?.exDividendDate ?: "") else q
        }
        saveQuotes(quotes)
        return quotes
    }

    // MARK: declared distribution records

    fun distributions(): Map<String, List<Distribution>> {
        val o = Store.readJSON("distributions.json")
        val out = HashMap<String, List<Distribution>>()
        for (sym in o.keys()) {
            val rows = o.optJSONArray(sym) ?: continue
            out[sym] = (0 until rows.length()).map { k ->
                val r = rows.getJSONObject(k)
                Distribution(r.optString("exDate"), r.optString("payDate"), r.optDouble("amount", 0.0), r.optString("currency"))
            }
        }
        return out
    }

    private fun isCanadianListing(exchange: String, currency: String): Boolean {
        val ex = exchange.trim().uppercase()
        if (ex.isNotEmpty()) return CANADIAN.contains(ex)
        return currency.uppercase() == "CAD"
    }

    /** The declared records of the Canadian dividend payers whose copy is older than 20 hours. */
    fun refreshDistributions(payers: List<Instrument>, force: Boolean = false) {
        val stamps = Store.readJSON("distributions_fetched.json")
        val records = Store.readJSON("distributions.json")
        val quotes = loadQuotes().toMutableMap()
        for (i in payers) {
            val sym = tmxSymbol(i.symbol)
            if (sym.isEmpty() || !isCanadianListing(i.exchange, i.currency)) continue
            if (!force) { val a = ageMs(stamps.optString(sym, "")); if (a != null && a < RECORD_STALE_MS) continue }
            val recSym = tmxRecordSymbol(sym, i.exchange) ?: continue
            val (quote, form) = tmxLookup(recSym) { tmxQuote(it) }
            val rows = JSONArray()
            try {
                val list = tmx("getDividendsForSymbol", JSONObject().put("symbol", form).put("page", 1).put("batch", TMX_BATCH), TMX_DIVIDENDS_QUERY)
                    .optJSONObject("data")?.optJSONObject("dividends")?.optJSONArray("dividends") ?: JSONArray()
                for (k in 0 until list.length()) {
                    val r = list.optJSONObject(k) ?: continue
                    val ex = r.optString("exDate", "").take(10)
                    val amt = num(r.opt("amount")) ?: continue
                    if (ex.length == 10 && amt > 0) rows.put(JSONObject().put("exDate", ex).put("payDate", r.optString("payableDate", "").take(10)).put("amount", amt).put("currency", r.optString("currency", "")))
                }
            } catch (e: Exception) {
            }
            // A Cboe Canada listing's price comes from Cboe's own feed; TMX's delayed
            // quote for it must not replace that, only its record is kept.
            if (quote != null && !CBOE_CANADA.contains(i.exchange.uppercase())) quotes[sym] = quote
            if (rows.length() > 0) records.put(sym, rows)
            if (quote != null || rows.length() > 0) stamps.put(sym, nowStamp())
        }
        Store.writeJSON("distributions.json", records)
        Store.writeJSON("distributions_fetched.json", stamps)
        saveQuotes(quotes)
    }

    // MARK: the indexes

    /** The S&P/TSX Composite and the TSX 60 from TMX Money, appended from a week before the newest stored day. */
    fun refreshIndexes() {
        val a = ageMs(meta("indexes_fetched"))
        if (a != null && a < 6 * 3600_000L) return
        val all = Store.readJSON("indexes.json")
        for ((key, sym) in TMX_INDICES) {
            val have = all.optJSONObject(key) ?: JSONObject()
            val last = have.keys().asSequence().maxOrNull()
            val start = if (last != null) Model.shiftDate(last, -7) else "2016-01-01"
            val data = try { tmx("getTimeSeriesData", JSONObject().put("symbol", sym).put("freq", "day").put("interval", 1).put("start", start).put("end", today()), TMX_HISTORY_QUERY) } catch (e: Exception) { continue }
            for (b in parseTMXHistory(data)) have.put(b.date, b.close)
            all.put(key, have)
        }
        Store.writeJSON("indexes.json", all)
        setMeta("indexes_fetched", nowStamp())
    }

    // MARK: bars

    private fun parseTMXHistory(data: JSONObject): List<DailyBar> {
        val rows = data.optJSONObject("data")?.optJSONArray("getTimeSeriesData") ?: JSONArray()
        val out = mutableListOf<DailyBar>()
        for (k in 0 until rows.length()) {
            val r = rows.optJSONObject(k) ?: continue
            val d = r.optString("dateTime", "").take(10)
            val close = num(r.opt("close")) ?: continue
            if (d.length == 10) out.add(DailyBar(d, num(r.opt("open")), num(r.opt("high")), num(r.opt("low")), close, num(r.opt("volume"))))
        }
        return out.sortedBy { it.date }
    }

    private fun wholeBars(bars: List<DailyBar>) = bars.filter { it.open != null && it.high != null && it.low != null }

    fun chartInstrument(i: Instrument): Instrument {
        if (i.kind == "Options") {
            val under = Model.underlyingSymbol(i.symbol)
            if (under.isNotEmpty() && under != "—") return Instrument(under, i.exchange, i.currency.ifEmpty { "USD" }, "Shares")
        }
        return i
    }

    private fun yahooForms(i: Instrument): List<String> {
        val root = tmxSymbol(i.symbol).replace(".", "-")
        val ccy = i.currency.ifEmpty { "CAD" }.uppercase()
        var forms = YAHOO_FORMS[ccy] ?: return emptyList()
        if (root.isEmpty() || root.contains(" ")) return emptyList()
        val first = YAHOO_SUFFIX[i.exchange.uppercase()]
        if (first != null && forms.contains(first)) forms = listOf(first) + forms.filter { it != first }
        return forms.map { root + it }
    }

    fun historyCandidates(i: Instrument): List<Pair<String, String>> {
        val sym = tmxSymbol(i.symbol)
        val ccy = i.currency.ifEmpty { "CAD" }.uppercase()
        if (sym.isEmpty()) return emptyList()
        if (i.kind == "Crypto") {
            val out = mutableListOf(Pair("coinbase", "$sym-$ccy"), Pair("yahoo", "$sym-$ccy"))
            if (ccy != "USD") out += listOf(Pair("coinbase", "$sym-USD"), Pair("yahoo", "$sym-USD"))
            return out
        }
        if (i.kind != "Shares") return emptyList()
        val out = mutableListOf<Pair<String, String>>()
        tmxQuoteSymbol(i)?.let { out.add(Pair("tmx", it)) }
        out += yahooForms(i).map { Pair("yahoo", it) }
        return out
    }

    private fun orderedCandidates(i: Instrument): List<Pair<String, String>> {
        val cands = historyCandidates(i)
        val v = if (cands.isEmpty()) "" else meta("bars_source:" + tmxSymbol(i.symbol))
        if (v.contains("|")) {
            val win = Pair(v.substringBefore("|"), v.substringAfter("|"))
            if (cands.contains(win)) return listOf(win) + cands.filter { it != win }
        }
        return cands
    }

    private fun barCurrency(key: String, i: Instrument): String {
        if (i.kind == "Crypto" && key.contains("-")) return key.substringAfter("-")
        return i.currency.ifEmpty { "CAD" }.uppercase()
    }

    private fun inPositionCurrency(bars: List<DailyBar>, barCcy: String, currency: String): List<DailyBar> {
        val quote = barCcy.uppercase()
        val ccy = currency.ifEmpty { "CAD" }.uppercase()
        if (quote == ccy) return bars
        if (!(quote == "USD" && ccy == "CAD")) return emptyList()
        val fx = Store.loadFx()
        val out = mutableListOf<DailyBar>()
        for (b in bars) {
            var rate: Double? = null
            for (k in 0 until 7) { val r = fx[Model.shiftDate(b.date, -k)]; if (r != null && r > 0) { rate = r; break } }
            val r = rate ?: continue
            out.add(DailyBar(b.date, b.open?.times(r), b.high?.times(r), b.low?.times(r), b.close * r, b.volume))
        }
        return out
    }

    private var yahooNextAt = 0L
    private var yahooBackoffUntil = 0L

    private fun yahooGet(url: String): String {
        synchronized(lock) {
            val now = System.currentTimeMillis()
            if (now < yahooBackoffUntil) { note("yahoo", false, BackoffException()); throw BackoffException() }
            val wait = yahooNextAt - now
            if (wait > 0) Thread.sleep(wait)
            yahooNextAt = System.currentTimeMillis() + YAHOO_MIN_INTERVAL_MS
        }
        try {
            return request(url, headers = YAHOO_HEADERS)
        } catch (e: HttpStatusException) {
            if (e.code == 429) yahooBackoffUntil = System.currentTimeMillis() + YAHOO_BACKOFF_MS
            throw e
        }
    }

    private fun parseYahooDaily(text: String): List<DailyBar> {
        val r = JSONObject(text).optJSONObject("chart")?.optJSONArray("result")?.optJSONObject(0) ?: return emptyList()
        val ts = r.optJSONArray("timestamp") ?: JSONArray()
        val q = r.optJSONObject("indicators")?.optJSONArray("quote")?.optJSONObject(0) ?: JSONObject()
        val meta = r.optJSONObject("meta") ?: JSONObject()
        val zone = try { ZoneId.of(meta.optString("exchangeTimezoneName", "")) } catch (e: Exception) { ZoneId.ofOffset("UTC", java.time.ZoneOffset.ofTotalSeconds(meta.optInt("gmtoffset", 0))) }
        fun col(k: String, i: Int): Double? { val a = q.optJSONArray(k) ?: return null; return if (i < a.length()) num(a.opt(i)) else null }
        val out = mutableListOf<DailyBar>()
        for (i in 0 until ts.length()) {
            val sec = num(ts.opt(i)) ?: continue
            val close = col("close", i) ?: continue
            if (close <= 0) continue
            val day = java.time.Instant.ofEpochSecond(sec.toLong()).atZone(zone).toLocalDate().toString()
            out.add(DailyBar(day, col("open", i), col("high", i), col("low", i), close, col("volume", i)))
        }
        return out.sortedBy { it.date }
    }

    private fun fetchYahooDaily(symbol: String, start: String, end: String): List<DailyBar> {
        val missKey = "yahoo_miss:$symbol"
        if (meta(missKey) == today()) return emptyList()
        val s = LocalDate.parse(start).toEpochDay() * 86400
        val e = (LocalDate.parse(end).toEpochDay() + 1) * 86400
        return try {
            parseYahooDaily(yahooGet("https://query1.finance.yahoo.com/v8/finance/chart/$symbol?period1=$s&period2=$e&interval=1d"))
        } catch (ex: HttpStatusException) {
            if (ex.code == 404) { setMeta(missKey, today()); emptyList() } else throw ex
        }
    }

    private fun coinbaseMarket(pair: String): String {
        val p = pair.uppercase()
        if (!p.contains("-")) return ""
        val key = "coinbase_product:$p"
        val v = meta(key)
        if (v.startsWith("@")) return v.drop(1)
        if (v.startsWith("none@") && v.drop(5) > Model.shiftDate(today(), -1)) return ""
        val id = try { JSONObject(request("https://api.exchange.coinbase.com/products/$p")).optString("id", "").uppercase() } catch (e: Exception) { "" }
        if (id == p) { setMeta(key, "@$p"); return p }
        setMeta(key, "none@" + today())
        return ""
    }

    private fun fetchCoinbaseDaily(product: String, start: String, end: String): List<DailyBar> {
        val s = LocalDate.parse(start).toEpochDay() * 86400
        val e = (LocalDate.parse(end).toEpochDay() + 1) * 86400
        val span = 300L * 86400
        val out = HashMap<Long, DailyBar>()
        var cur = s
        val iso = SimpleDateFormat("yyyy-MM-dd'T'HH:mm:ss'Z'", Locale.US).apply { timeZone = TimeZone.getTimeZone("UTC") }
        while (cur < e) {
            val to = minOf(cur + span, e)
            val rows = JSONArray(request("https://api.exchange.coinbase.com/products/$product/candles?granularity=86400&start=${iso.format(Date(cur * 1000))}&end=${iso.format(Date(to * 1000))}"))
            for (k in 0 until rows.length()) {
                val r = rows.optJSONArray(k) ?: continue
                if (r.length() < 6) continue
                val t = num(r.opt(0)) ?: continue
                val lo = num(r.opt(1)); val hi = num(r.opt(2)); val op = num(r.opt(3)); val cl = num(r.opt(4)) ?: continue
                if (cl > 0) out[t.toLong()] = DailyBar(LocalDate.ofEpochDay(t.toLong() / 86400).toString(), op, hi, lo, cl, num(r.opt(5)))
            }
            cur += span
        }
        return out.keys.sorted().map { out[it]!! }
    }

    private fun fetchDailyFrom(source: String, key: String, i: Instrument, start: String, end: String): List<DailyBar> = when (source) {
        "tmx" -> {
            var failure: Exception? = null
            val (bars, _) = tmxLookup(key) { form ->
                try {
                    val b = parseTMXHistory(tmx("getTimeSeriesData", JSONObject().put("symbol", form).put("freq", "day").put("interval", 1).put("start", start).put("end", end), TMX_HISTORY_QUERY))
                    if (b.isEmpty()) null else b
                } catch (e: Exception) {
                    failure = e
                    null
                }
            }
            if (bars != null) wholeBars(bars) else if (failure != null) throw failure!! else emptyList()
        }
        "coinbase" -> {
            val product = coinbaseMarket(key)
            if (product.isEmpty()) emptyList() else inPositionCurrency(fetchCoinbaseDaily(product, start, end), barCurrency(key, i), i.currency)
        }
        "yahoo" -> inPositionCurrency(wholeBars(fetchYahooDaily(key, start, end)), barCurrency(key, i), i.currency)
        else -> emptyList()
    }

    private fun fetchHistory(i: Instrument, start: String, end: String): Pair<List<DailyBar>, String> {
        val spanStart = LocalDate.parse(start).toEpochDay()
        val answers = mutableListOf<Triple<String, String, List<DailyBar>>>()
        val notes = mutableListOf<Triple<String, String, String?>>()
        for ((source, key) in orderedCandidates(i)) {
            var bars: List<DailyBar> = emptyList()
            try {
                bars = fetchDailyFrom(source, key, i, start, end)
                notes.add(Triple(source, key, null))
            } catch (e: Exception) {
                notes.add(Triple(source, key, describeFailure(e)))
            }
            answers.add(Triple(source, key, bars))
            val first = bars.firstOrNull()
            if (first != null && LocalDate.parse(first.date).toEpochDay() <= spanStart + COVERAGE_SLACK_DAYS) break
        }
        var best: Triple<Long, Pair<String, String>, List<DailyBar>>? = null
        for ((source, key, bars) in answers) {
            val first = bars.firstOrNull() ?: continue
            val f = LocalDate.parse(first.date).toEpochDay()
            if (f <= spanStart + COVERAGE_SLACK_DAYS) {
                setMeta("bars_source:" + tmxSymbol(i.symbol), "$source|$key")
                return Pair(bars, source)
            }
            if (best == null || f < best.first) best = Triple(f, Pair(source, key), bars)
        }
        if (best != null) {
            setMeta("bars_source:" + tmxSymbol(i.symbol), best.second.first + "|" + best.second.second)
            return Pair(best.third, best.second.first)
        }
        synchronized(lock) { chartNotes[tmxSymbol(i.symbol)] = notes }
        return Pair(emptyList(), "")
    }

    fun chartReason(i: Instrument): String {
        val notes = synchronized(lock) { chartNotes[tmxSymbol(i.symbol)] ?: emptyList() }
        val failed = mutableListOf<String>()
        for ((source, _, failure) in notes) {
            if (failure != null) {
                val line = (SOURCE_LABELS[source] ?: source) + " " + failure
                if (!failed.contains(line)) failed.add(line)
            }
        }
        if (failed.isNotEmpty()) return failed.joinToString("; ") + "."
        val names = mutableListOf<String>()
        for ((source, _) in historyCandidates(i)) { val n = SOURCE_LABELS[source] ?: source; if (!names.contains(n)) names.add(n) }
        if (names.isEmpty()) return "No price source covers this instrument."
        val list = if (names.size <= 2) names.joinToString(" or ") else names.dropLast(1).joinToString(", ") + " or " + names.last()
        return "No bars for this span from $list."
    }

    private fun ensureHistory(i: Instrument, start: String, end: String): List<DailyBar> {
        val sym = tmxSymbol(i.symbol)
        if (sym.isEmpty() || start.length != 10 || end.length != 10) return emptyList()
        val name = "bars-" + sym.replace("/", "_") + ".json"
        var file = Store.readJSON(name)
        val coveredFrom = file.optString("start", "")
        val covered = coveredFrom.isNotEmpty() && coveredFrom <= start
        val fresh = ageMs(file.optString("fetchedAt", ""))?.let { it < HISTORY_STALE_MS } ?: false
        val needsRecent = end >= Model.shiftDate(today(), -3)
        if (!covered || (needsRecent && !fresh)) {
            val fetchFrom = if (covered) minOf(start, coveredFrom) else start
            val (bars, source) = fetchHistory(i, fetchFrom, today())
            if (bars.isNotEmpty()) {
                val stored = file.optJSONObject("bars") ?: JSONObject()
                for (b in bars) stored.put(b.date, JSONObject().put("o", b.open ?: JSONObject.NULL).put("h", b.high ?: JSONObject.NULL).put("l", b.low ?: JSONObject.NULL).put("c", b.close).put("v", b.volume ?: JSONObject.NULL))
                val gotFrom = bars[0].date
                val from = if (LocalDate.parse(gotFrom).toEpochDay() <= LocalDate.parse(fetchFrom).toEpochDay() + COVERAGE_SLACK_DAYS) fetchFrom else gotFrom
                file = JSONObject().put("start", from).put("fetchedAt", nowStamp()).put("source", source).put("bars", stored)
                Store.writeJSON(name, file)
            }
        }
        val stored = file.optJSONObject("bars") ?: JSONObject()
        val out = mutableListOf<DailyBar>()
        for (d in stored.keys()) {
            if (d < start || d > end) continue
            val b = stored.optJSONObject(d) ?: continue
            val c = num(b.opt("c")) ?: continue
            out.add(DailyBar(d, num(b.opt("o")), num(b.opt("h")), num(b.opt("l")), c, num(b.opt("v"))))
        }
        return out.sortedBy { it.date }
    }

    /** Weekly (Monday start) or monthly bars from daily ones. */
    private fun aggregateDaily(bars: List<DailyBar>, tf: String): List<DailyBar> {
        val out = mutableListOf<DailyBar>()
        for (b in bars) {
            val d = LocalDate.parse(b.date)
            val key = if (tf == "1w") d.minusDays((d.dayOfWeek.value - 1).toLong()).toString() else d.withDayOfMonth(1).toString()
            val cur = out.lastOrNull()
            if (cur != null && cur.date == key) {
                out[out.size - 1] = cur.copy(
                    close = b.close,
                    high = if (b.high != null) (cur.high?.let { maxOf(it, b.high) } ?: b.high) else cur.high,
                    low = if (b.low != null) (cur.low?.let { minOf(it, b.low) } ?: b.low) else cur.low,
                    volume = if (b.volume != null) (cur.volume ?: 0.0) + b.volume else cur.volume,
                )
            } else {
                out.add(DailyBar(key, b.open, b.high, b.low, b.close, b.volume))
            }
        }
        return out
    }

    /** The trade chart: real bars for the trade's span, ten days either side, at the
     * timeframe the trade's length calls for. Intraday timeframes are not on the phone yet. */
    fun bars(trade: Trade, timeframe: String? = null): Triple<List<Bar>, String, String> {
        val inst = chartInstrument(Instrument(trade.symbol, trade.exchange, trade.currency, trade.kind))
        val start = Model.shiftDate(trade.entryDate, -10)
        val end = minOf(Model.shiftDate(trade.exitDate, 10), today())
        val tf = timeframe ?: if (trade.holdDays <= 180) "1d" else if (trade.holdDays <= 1100) "1w" else "1M"
        val daily = ensureHistory(inst, start, end)
        if (daily.isEmpty()) return Triple(emptyList(), chartReason(inst), tf)
        val shown = if (tf == "1d") daily else aggregateDaily(daily, tf)
        return Triple(shown.mapNotNull { b -> if (b.open == null || b.high == null || b.low == null) null else Bar(b.date, b.open, b.high, b.low, b.close) }, "", tf)
    }
}
