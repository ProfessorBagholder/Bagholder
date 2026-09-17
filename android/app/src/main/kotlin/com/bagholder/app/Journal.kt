// The app's state: the session, the last pull, the journal, the filter set,
// and the model built from them. Screens read `Book`; the pull and the market
// data run off the main thread and write it back.
package com.bagholder.app

import android.content.Context
import android.webkit.CookieManager
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import com.bagholder.model.AccountInfo
import com.bagholder.model.BalanceRow
import com.bagholder.model.Base
import com.bagholder.model.MarginRow
import com.bagholder.model.Filters
import com.bagholder.model.JournalEntry
import com.bagholder.model.Market
import com.bagholder.model.Model
import com.bagholder.model.ModelView
import com.bagholder.model.NavPoint
import com.bagholder.model.Position
import com.bagholder.model.Quote
import com.bagholder.model.Security
import com.bagholder.model.Trade
import com.bagholder.model.View
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.delay
import kotlinx.coroutines.isActive
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import java.text.SimpleDateFormat
import java.util.Calendar
import java.util.Date
import java.util.Locale
import java.util.TimeZone

object Book {
    sealed class Phase {
        object Idle : Phase()
        object Pulling : Phase()
        object Ready : Phase()
        data class Failed(val message: String) : Phase()
    }

    var appVersion = ""
        private set
    var phase by mutableStateOf<Phase>(Phase.Idle)
        private set
    var syncStep by mutableStateOf("")
        private set
    var connected by mutableStateOf(false)
        private set
    var lastSync by mutableStateOf<Long?>(null)
        private set
    var view by mutableStateOf<View?>(null)
        private set
    var filters by mutableStateOf(Filters())
        private set
    var quotes by mutableStateOf<Map<String, Quote>>(emptyMap())
        private set
    var journal: Map<String, JournalEntry> = emptyMap()
        private set
    var base: Base? = null
        private set
    var pull: StoredPull? = null
        private set

    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.IO)
    private var started = false
    private var pulling = false
    private var generation = 0
    private var marketJob: Job? = null
    private var noNewShownAt: Long? = null

    // MARK: status

    /** The header's status: the sync step while pulling, an error once, else when the last sync was. */
    val headerStatus: String
        get() {
            if (syncStep.isNotEmpty()) {
                val shown = noNewShownAt
                if (!(syncStep == "No new transactions" && shown != null && System.currentTimeMillis() - shown >= 45_000)) return syncStep
            }
            (phase as? Phase.Failed)?.let { return it.message }
            lastSync?.let { return Fmt.syncedLabel(it) }
            if (phase == Phase.Pulling) return "Fetching accounts…"
            return ""
        }

    val statusIsError: Boolean get() = phase is Phase.Failed

    // MARK: lifecycle

    fun start(context: Context) {
        if (started) return
        started = true
        Store.init(context.applicationContext)
        appVersion = try { context.packageManager.getPackageInfo(context.packageName, 0).versionName ?: "" } catch (e: Exception) { "" }
        connected = Store.loadSession() != null
        lastSync = Store.lastSync ?: java.io.File(Store.dir, "last-pull.json").takeIf { it.exists() }?.lastModified()
        filters = Store.loadFilters()
        journal = Store.loadJournal()
        quotes = MarketData.loadQuotes()
        scope.launch {
            val stored = Store.loadPull()
            if (stored != null) {
                pull = stored
                val v = rebuild()
                withContext(Dispatchers.Main) {
                    view = v
                    phase = Phase.Ready
                }
            }
        }
    }

    /** On appear: a saved pull is shown at once; the session pulls when a sync is due. */
    fun handleAppear() {
        startMarketLoop()
        startPortfolioLoop()
        if (!connected || pulling) return
        if (pull == null || activityPullDue(lastSync)) pull()
    }

    fun handleBackground() {
        marketJob?.cancel()
        marketJob = null
    }

    /** crates/store/src/admin.rs activity_pull_due: America/Edmonton, Mon-Fri, at or after 14:00. */
    fun activityPullDue(lastSync: Long?, now: Long = System.currentTimeMillis()): Boolean {
        val cal = Calendar.getInstance(TimeZone.getTimeZone("America/Edmonton"))
        cal.timeInMillis = now
        val weekday = cal.get(Calendar.DAY_OF_WEEK)
        if (weekday == Calendar.SATURDAY || weekday == Calendar.SUNDAY) return false
        cal.set(Calendar.HOUR_OF_DAY, 14); cal.set(Calendar.MINUTE, 0); cal.set(Calendar.SECOND, 0); cal.set(Calendar.MILLISECOND, 0)
        val close = cal.timeInMillis
        if (now < close) return false
        val last = lastSync ?: return true
        return last < close
    }

    fun connect(cookie: String, wssdi: String?) {
        Store.saveSession(cookie, wssdi)
        connected = true
        pull()
    }

    fun disconnect() {
        generation += 1
        Store.clearSession()
        Store.clearPull()
        CookieManager.getInstance().removeAllCookies(null)
        CookieManager.getInstance().flush()
        connected = false
        pull = null
        base = null
        view = null
        phase = Phase.Idle
        syncStep = ""
        lastSync = null
        Store.lastSync = null
    }

    fun syncNow() {
        if (connected) pull()
    }

    // MARK: the pull

    private fun pull() {
        val session = Store.loadSession() ?: return
        if (pulling) return
        pulling = true
        generation += 1
        val gen = generation
        noNewShownAt = null
        phase = Phase.Pulling
        syncStep = "Fetching accounts…"
        scope.launch {
            try {
                val stored = pull
                val res = WSPull.run(session.first, session.second, stored?.activities ?: emptyList(), stored?.listings ?: emptyList(),
                    stored?.nav ?: emptyList(), stored?.navByAccount ?: emptyMap()) { step ->
                    scope.launch(Dispatchers.Main) { if (gen == generation) syncStep = step }
                }
                if (gen != generation) return@launch
                val newPull = StoredPull(res.activities, res.listings, nowIso(), res.nav, res.navByAccount, res.accounts, res.balances, res.margin)
                Store.savePull(newPull)
                pull = newPull
                withContext(Dispatchers.Main) {
                    if (res.newRows) {
                        lastSync = System.currentTimeMillis()
                        Store.lastSync = lastSync
                        syncStep = ""
                    } else {
                        syncStep = "No new transactions"
                        noNewShownAt = System.currentTimeMillis()
                    }
                    phase = Phase.Ready
                    view = rebuild()
                }
                withContext(Dispatchers.Main) { if (syncStep.isEmpty()) syncStep = "Fetching exchange rates…" }
                Store.saveFx(WSPull.ensureFxRates(res.activities, Store.loadFx()))
                val sp = WSPull.fetchSp500()
                if (sp.isNotEmpty()) Store.saveSp500(sp)
                if (gen != generation) return@launch
                val v = rebuild()
                withContext(Dispatchers.Main) {
                    if (syncStep == "Fetching exchange rates…") syncStep = ""
                    view = v
                }
            } catch (e: Exception) {
                withContext(Dispatchers.Main) { showError(gen, e.message ?: e.toString()) }
            } finally {
                pulling = false
            }
        }
    }

    private fun showError(gen: Int, msg: String) {
        if (gen != generation) return
        val shown = msg.trim().ifEmpty { "Wealthsimple token refresh failed" }
        if (pull != null) {
            phase = Phase.Ready
            syncStep = shown
        } else {
            phase = Phase.Failed(shown)
            syncStep = ""
        }
    }

    private fun nowIso(): String {
        val f = SimpleDateFormat("yyyy-MM-dd'T'HH:mm:ss'Z'", Locale.US)
        f.timeZone = TimeZone.getTimeZone("UTC")
        return f.format(Date())
    }

    // MARK: the model

    private fun rebuild(): View? {
        val p = pull ?: return null
        val securities = p.listings.map { Security(id = it.id, symbol = it.symbol, name = it.name, underlyingId = it.underlyingId, primaryExchange = it.primaryExchange, primaryMic = it.primaryMic, currency = it.currency) }
        val sp = Store.loadSp500()
        val benchmarks = LinkedHashMap<String, Map<String, Double>>()
        benchmarks["SP500"] = sp
        benchmarks.putAll(Store.loadIndexes())
        val market = Market(fx = Store.loadFx(), distributions = MarketData.distributions(), quotes = quotes, benchmark = sp, benchmarks = benchmarks)
        val nav = p.nav.map { NavPoint(it.date, it.equity, it.netDeposits) }
        val navBy = p.navByAccount.mapValues { e -> e.value.map { NavPoint(it.date, it.equity, it.netDeposits) } }
        val accounts = p.accounts.map { AccountInfo(it.id, it.nickname, it.currency, it.netLiquidationValue, it.unifiedAccountType, it.status) }
        val balances = p.balances.map { BalanceRow(it.accountId, it.securityId, it.quantity) }
        val margin = p.margin.map { MarginRow(it.accountId, it.buyingPower, it.currency, it.unavailable) }
        val b = Model.buildBase(p.activities.map { it.toAct() }, securities, market, Model.todayLocal(), nav, navBy, journal, accounts, balances, margin)
        base = b
        return ModelView.buildView(b, filters)
    }

    fun applyFilters(f: Filters) {
        filters = f
        Store.saveFilters(f)
        base?.let { view = ModelView.buildView(it, f) }
    }

    fun clearFilters() {
        val f = Filters()
        f.benchmark = filters.benchmark
        applyFilters(f)
    }

    fun setBenchmark(key: String) {
        val f = copyFilters(filters)
        f.benchmark = key
        applyFilters(f)
    }

    fun copyFilters(f: Filters): Filters {
        val g = Filters()
        for ((k, v) in f.lists) g.lists[k] = v.toList()
        for ((k, r) in f.ranges) g.ranges[k] = com.bagholder.model.Range(r.op, r.v)
        g.preset = f.preset; g.years = f.years.toList(); g.from = f.from; g.to = f.to; g.search = f.search; g.benchmark = f.benchmark
        return g
    }

    /** A journal entry changes only the trade's or position's own fields: no rematching. */
    fun saveJournal(id: String, entry: JournalEntry) {
        journal = journal + (id to entry)
        Store.saveJournal(journal)
        val b = base ?: return
        b.trades.firstOrNull { it.id == id }?.let { it.grade = entry.grade; it.thesis = entry.thesis; it.tags = entry.tags }
        for (p in b.positions) if (p.id == id) { p.grade = entry.grade; p.thesis = entry.thesis; p.tags = entry.tags }
        view = ModelView.buildView(b, filters)
    }

    fun trade(id: String): Trade? = base?.trades?.firstOrNull { it.id == id }
    fun position(id: String): Position? = base?.positions?.firstOrNull { it.id == id }

    // MARK: market data, every minute while the app is up

    // MARK: the Portfolio figures Wealthsimple states, every five minutes while the app is up

    private val PORTFOLIO_REFRESH_MS = 5L * 60_000
    private var portfolioJob: Job? = null

    private fun startPortfolioLoop() {
        if (portfolioJob != null) return
        portfolioJob = scope.launch {
            // the first read as soon as the app is ready, then every five minutes; a
            // failed read is logged, not swallowed
            while (isActive) {
                val session = Store.loadSession()
                val stored = pull
                if (session == null || stored == null || phase != Phase.Ready) { delay(5_000); continue }
                try {
                    val snap = withContext(Dispatchers.IO) { WSPull.refreshPortfolio(session.first, session.second) }
                    val newPull = StoredPull(stored.activities, stored.listings, stored.syncedAt, stored.nav, stored.navByAccount, snap.accounts, snap.balances, snap.margin)
                    withContext(Dispatchers.IO) { Store.savePull(newPull) }
                    pull = newPull
                    val v = rebuild()
                    withContext(Dispatchers.Main) { view = v }
                } catch (e: Exception) {
                    android.util.Log.w("bagholder", "portfolio refresh failed", e)
                }
                delay(PORTFOLIO_REFRESH_MS)
            }
        }
    }

    private fun startMarketLoop() {
        if (marketJob != null) return
        marketJob = scope.launch {
            while (isActive) {
                val b = base
                if (b != null) {
                    val held = b.positions.map { Instrument(it.symbol, it.exchange, it.currency, it.kind) }
                    val paying = b.cashflow.filter { it.kind == "Dividend" }.map { it.symbol }.toSet()
                    val payers = held.filter { paying.contains(it.symbol) && it.kind == "Shares" }
                    MarketData.refreshIndexes()
                    MarketData.refreshDistributions(payers)
                    val q = MarketData.refreshQuotes(held)
                    if (!isActive) return@launch
                    // the book is rebuilt only when a price actually moved
                    val changed = q.size != quotes.size || q.any { (sym, quote) -> quotes[sym]?.price != quote.price || quotes[sym]?.exDividendDate != quote.exDividendDate }
                    quotes = q
                    if (changed) {
                        val v = rebuild()
                        withContext(Dispatchers.Main) { view = v }
                    }
                }
                delay(60_000)
            }
        }
    }
}
