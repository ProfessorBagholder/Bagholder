// The Wealthsimple pull: the session from the login cookie, token refresh,
// the GraphQL calls the web app makes, and the activity rows mapped the way
// crates/ws/src/mapping.rs maps them. A port of the iOS app's WSPull.swift.
package com.bagholder.app

import com.bagholder.model.Act
import org.json.JSONArray
import org.json.JSONObject
import java.io.ByteArrayOutputStream
import java.net.HttpURLConnection
import java.net.URL
import java.net.URLDecoder
import java.text.SimpleDateFormat
import java.util.Base64
import java.util.Date
import java.util.Locale
import java.util.TimeZone
import kotlin.math.abs

/** One Wealthsimple activity row as the sync stores it. */
data class WSActivity(
    val id: String, val canonicalId: String, val occurredAt: String, val transactionDate: String,
    val accountId: String, var fifoId: String, val accountType: String, val activityType: String, val activitySubType: String,
    val description: String, val direction: String, val symbol: String, val name: String, val currency: String,
    val quantity: Double, val unitPrice: Double, val commission: Double, val netCashAmount: Double,
    val category: String, val rawType: String, val aftType: String, val counterSymbol: String, val securityId: String,
) {
    fun toAct(): Act {
        val a = Act()
        a.id = id; a.occurredAt = occurredAt; a.transactionDate = transactionDate; a.accountId = accountId; a.fifoId = fifoId
        a.accountType = accountType; a.activityType = activityType; a.activitySubType = activitySubType
        a.description = description; a.direction = direction; a.symbol = symbol; a.name = name; a.currency = currency
        a.quantity = quantity; a.unitPrice = unitPrice; a.commission = commission; a.netCashAmount = netCashAmount
        a.category = category; a.rawType = rawType; a.aftType = aftType; a.securityId = securityId
        return a
    }

    fun toJson(): JSONObject = JSONObject()
        .put("id", id).put("canonicalId", canonicalId).put("occurredAt", occurredAt).put("transactionDate", transactionDate)
        .put("accountId", accountId).put("fifoId", fifoId).put("accountType", accountType).put("activityType", activityType)
        .put("activitySubType", activitySubType).put("description", description).put("direction", direction).put("symbol", symbol)
        .put("name", name).put("currency", currency).put("quantity", quantity).put("unitPrice", unitPrice).put("commission", commission)
        .put("netCashAmount", netCashAmount).put("category", category).put("rawType", rawType).put("aftType", aftType)
        .put("counterSymbol", counterSymbol).put("securityId", securityId)

    companion object {
        fun fromJson(d: JSONObject) = WSActivity(
            d.optString("id"), d.optString("canonicalId"), d.optString("occurredAt"), d.optString("transactionDate"),
            d.optString("accountId"), d.optString("fifoId"), d.optString("accountType"), d.optString("activityType"), d.optString("activitySubType"),
            d.optString("description"), d.optString("direction"), d.optString("symbol"), d.optString("name"), d.optString("currency"),
            d.optDouble("quantity", 0.0), d.optDouble("unitPrice", 0.0), d.optDouble("commission", 0.0), d.optDouble("netCashAmount", 0.0),
            d.optString("category"), d.optString("rawType"), d.optString("aftType"), d.optString("counterSymbol"), d.optString("securityId"),
        )
    }
}

data class WSAccountRow(val id: String, val nickname: String, val currency: String, val netLiquidationValue: Double?, val unifiedAccountType: String = "", val status: String = "") {
    fun toJson(): JSONObject = JSONObject().put("id", id).put("nickname", nickname).put("currency", currency).put("netLiquidationValue", netLiquidationValue ?: JSONObject.NULL)
        .put("unifiedAccountType", unifiedAccountType).put("status", status)
    companion object {
        /** A pull stored before the type and status were kept reads without them. */
        fun fromJson(o: JSONObject) = WSAccountRow(o.optString("id"), o.optString("nickname"), o.optString("currency"), if (o.isNull("netLiquidationValue")) null else o.optDouble("netLiquidationValue"),
            o.optString("unifiedAccountType"), o.optString("status"))
    }
}

class WSBalanceRow(val accountId: String, val securityId: String, val quantity: Double) {
    fun toJson(): JSONObject = JSONObject().put("accountId", accountId).put("securityId", securityId).put("quantity", quantity)
    companion object { fun fromJson(o: JSONObject) = WSBalanceRow(o.optString("accountId"), o.optString("securityId"), o.optDouble("quantity", 0.0)) }
}

class WSMarginRow(val accountId: String, val buyingPower: Double?, val currency: String, val unavailable: String) {
    fun toJson(): JSONObject = JSONObject().put("accountId", accountId).put("buyingPower", buyingPower ?: JSONObject.NULL).put("currency", currency).put("unavailable", unavailable)
    companion object { fun fromJson(o: JSONObject) = WSMarginRow(o.optString("accountId"), if (o.isNull("buyingPower")) null else o.optDouble("buyingPower"), o.optString("currency", "CAD"), o.optString("unavailable")) }
}

class WSSecurityListing(
    val id: String, val symbol: String, val name: String, val primaryExchange: String, val primaryMic: String,
    val currency: String, val underlyingId: String,
) {
    fun toJson(): JSONObject = JSONObject().put("id", id).put("symbol", symbol).put("name", name).put("primaryExchange", primaryExchange)
        .put("primaryMic", primaryMic).put("currency", currency).put("underlyingId", underlyingId)

    companion object {
        fun fromJson(d: JSONObject) = WSSecurityListing(
            d.optString("id"), d.optString("symbol"), d.optString("name"), d.optString("primaryExchange"), d.optString("primaryMic"),
            d.optString("currency"), d.optString("underlyingId"),
        )
    }
}

/** One day of Wealthsimple's NAV history. */
data class WSNavPoint(val date: String, val equity: Double, val currency: String, val netDeposits: Double?) {
    fun toJson(): JSONObject = JSONObject().put("date", date).put("equity", equity).put("currency", currency).put("netDeposits", netDeposits ?: JSONObject.NULL)
    companion object {
        fun fromJson(d: JSONObject) = WSNavPoint(d.optString("date"), d.optDouble("equity", 0.0), d.optString("currency", "CAD"), if (d.has("netDeposits") && !d.isNull("netDeposits")) d.optDouble("netDeposits") else null)
    }
}

class WSSession(
    var accessToken: String, var refreshToken: String, var clientId: String, var identityCanonicalId: String,
    var expiresAt: String, var sessionId: String, var wssdi: String,
)

class PullException(message: String) : Exception(message)
class UnauthorizedException : Exception("unauthorized")

object WSPull {
    const val GRAPHQL_URL = "https://my.wealthsimple.com/graphql"
    const val TOKEN_URL = "https://api.production.wealthsimple.com/v1/oauth/v2/token"
    const val TOKEN_INFO_URL = "https://api.production.wealthsimple.com/v1/oauth/v2/token/info"
    const val LOGIN_URL = "https://my.wealthsimple.com/app/login"
    const val OAUTH_COOKIE = "_oauth2_access_v2"
    const val WS_CLIENT = "@wealthsimple/wealthsimple"
    const val GRAPHQL_VERSION = "12"

    private val MONTHS = listOf("JAN", "FEB", "MAR", "APR", "MAY", "JUN", "JUL", "AUG", "SEP", "OCT", "NOV", "DEC")
    private val KEEP_STATUS = setOf(
        "POSTED", "COMPLETED", "SETTLED", "COMPLETE", "FILLED", "EXECUTED",
        "PROCESSED", "CONFIRMED", "BOOKED", "SUCCEEDED", "SUCCESS",
    )
    private val CORP_BLOBS = listOf(
        "STKDIS", "STOCKDISTRIBUTION", "STOCKDIV", "SPINOFF", "SPIN",
        "DIVIDENDINKIND", "INKIND", "CORPORATEACTION", "CODECHANGE",
        "SYMBOLCHANGE", "TICKERCHANGE", "LISTINGSTATUS", "SECURITYSWAP",
        "MANDATORYEXCHANGE", "NAMECHANGE",
    )
    private val SKIP_MARKERS = listOf("SHARE_LENDING", "SHARELENDING", "STOCK_LENDING", "STOCKLENDING")
    private val IDENTITY_KEYS = listOf(
        "identity_canonical_id", "identityCanonicalId", "canonical_id",
        "identity_id", "resource_owner_id", "sub",
    )

    // MARK: JSON cookie / session

    private fun str(o: JSONObject?, k: String): String {
        if (o == null || !o.has(k) || o.isNull(k)) return ""
        val v = o.opt(k)
        return if (v is String) v else v.toString()
    }

    private fun num(v: Any?, def: Double = 0.0): Double = when (v) {
        null, JSONObject.NULL -> def
        is Number -> v.toDouble()
        is String -> if (v.isEmpty()) def else v.toDoubleOrNull() ?: def
        else -> def
    }

    private fun dict(v: Any?): JSONObject = v as? JSONObject ?: JSONObject()
    private fun arr(v: Any?): JSONArray = v as? JSONArray ?: JSONArray()

    fun jsonWithAccessToken(raw: String): JSONObject? {
        var cur = raw.trim()
        repeat(3) {
            if (cur.contains("access_token")) {
                try {
                    val obj = JSONObject(cur)
                    if (str(obj, "access_token").isNotEmpty()) return obj
                } catch (e: Exception) {
                }
            }
            val nxt = try { URLDecoder.decode(cur, "UTF-8") } catch (e: Exception) { cur }
            if (nxt == cur) return null
            cur = nxt
        }
        return null
    }

    fun session(fromCookie: String, wssdi: String?): WSSession? {
        val oauth = jsonWithAccessToken(fromCookie) ?: return null
        val access = str(oauth, "access_token")
        if (access.isEmpty()) return null
        var cid = clientIdFromObject(oauth)
        if (cid.isEmpty()) cid = clientIdFromJWT(access)
        return WSSession(
            access, str(oauth, "refresh_token"), cid, identityFrom(oauth), expiresAtString(oauth.opt("expires_at")),
            str(oauth, "session_id"), (wssdi ?: "").trim(),
        )
    }

    private fun identityFrom(obj: JSONObject): String {
        for (k in IDENTITY_KEYS) {
            val v = str(obj, k)
            if (v.isNotEmpty()) return v
        }
        return ""
    }

    private fun clientIdFromObject(obj: JSONObject): String {
        for (k in listOf("client_id", "clientId", "application_uid", "azp")) {
            val v = str(obj, k).trim()
            if (v.isNotEmpty()) return v
        }
        val app = dict(obj.opt("application"))
        for (k in listOf("uid", "client_id", "clientId")) {
            val v = str(app, k).trim()
            if (v.isNotEmpty()) return v
        }
        return ""
    }

    private fun clientIdFromTokenInfo(info: JSONObject): String {
        val fromObj = clientIdFromObject(info)
        if (fromObj.isNotEmpty()) return fromObj
        val uid = str(info, "application_uid")
        if (uid.isNotEmpty()) return uid
        val app = dict(info.opt("application"))
        val a = str(app, "uid")
        if (a.isNotEmpty()) return a
        return str(app, "client_id")
    }

    private fun clientIdFromJWT(token: String): String {
        val parts = token.split(".")
        if (parts.size != 3) return ""
        var payload = parts[1].replace("-", "+").replace("_", "/")
        val rem = payload.length % 4
        if (rem != 0) payload += "=".repeat(4 - rem)
        return try {
            clientIdFromObject(JSONObject(String(Base64.getDecoder().decode(payload), Charsets.UTF_8)))
        } catch (e: Exception) {
            ""
        }
    }

    class TokenBox(var sess: WSSession, var oauth: JSONObject) {
        var didRefresh = false
    }

    private fun persistCookie(box: TokenBox) {
        Store.saveSession(box.oauth.toString(), box.sess.wssdi.ifEmpty { null })
    }

    private fun ensureClientId(box: TokenBox) {
        var cid = clientIdFromObject(box.oauth)
        if (cid.isEmpty()) cid = clientIdFromJWT(box.sess.accessToken)
        if (cid.isEmpty()) {
            try {
                cid = clientIdFromTokenInfo(tokenInfo(box.sess))
            } catch (e: Exception) {
            }
        }
        cid = cid.trim()
        if (cid.isEmpty()) return
        if (box.sess.clientId == cid && str(box.oauth, "client_id") == cid) return
        box.sess.clientId = cid
        box.oauth.put("client_id", cid)
        persistCookie(box)
    }

    private fun expiresAtString(raw: Any?): String {
        if (raw is String && raw.contains("T")) return raw.trim()
        return if (raw == null || raw == JSONObject.NULL) "" else raw.toString()
    }

    private fun utcFormat(pattern: String): SimpleDateFormat {
        val f = SimpleDateFormat(pattern, Locale.US)
        f.timeZone = TimeZone.getTimeZone("UTC")
        return f
    }

    private fun expiresAtAsTimestamp(data: JSONObject): String? {
        val s = data.opt("expires_at")
        if (s is String && s.contains("T")) return s.trim()
        val unix: Double = when {
            s is Number -> s.toDouble()
            data.opt("expires_in") is Number -> System.currentTimeMillis() / 1000.0 + (data.opt("expires_in") as Number).toDouble()
            else -> return null
        }
        return utcFormat("yyyy-MM-dd'T'HH:mm:ss.000'Z'").format(Date((unix * 1000).toLong()))
    }

    private fun parseExpires(raw: String): Date? {
        val s = raw.trim()
        for (pattern in listOf("yyyy-MM-dd'T'HH:mm:ss.SSSX", "yyyy-MM-dd'T'HH:mm:ssX", "yyyy-MM-dd'T'HH:mm:ss.SSS'Z'", "yyyy-MM-dd'T'HH:mm:ss'Z'")) {
            try {
                return utcFormat(pattern).parse(s)
            } catch (e: Exception) {
            }
        }
        return null
    }

    private fun tokenNeedsRefresh(sess: WSSession): Boolean {
        val exp = parseExpires(sess.expiresAt) ?: return false
        return Date(System.currentTimeMillis() + 300_000) >= exp
    }

    private fun oauthErrorCode(data: JSONObject): String {
        val err = str(data, "error").trim()
        if (err.isEmpty()) return ""
        if (Regex("^[a-fA-F0-9]{32,}$").matches(err)) return ""
        if (!Regex("^[A-Za-z0-9_.-]{1,64}$").matches(err)) return ""
        return err
    }

    private fun refreshFailureMessage(data: JSONObject): String {
        val parts = mutableListOf<String>()
        val status = data.optInt("_http_status", 0)
        if (status != 0) parts.add("Wealthsimple token refresh HTTP $status")
        val oauthErr = oauthErrorCode(data)
        if (oauthErr.isNotEmpty()) parts.add(oauthErr)
        if (parts.isEmpty()) return "Wealthsimple token refresh failed"
        return parts.joinToString(" ")
    }

    const val REFUSED_LOGIN_MESSAGE = "Saved login refused. Connect Wealthsimple again."
    private val refreshLock = Any()
    private var refusedRefreshToken = ""   // a token Wealthsimple answered invalid_grant to; never posted again this run

    /** Refreshes run one at a time (bagholder.refresh_session): Wealthsimple rotates the
     *  refresh token on every grant, so two callers posting the same token would leave the
     *  loser with invalid_grant and the login dead. Under the lock the saved login is read
     *  again and, when another caller has rotated it meanwhile, adopted without a post; a
     *  token Wealthsimple has refused is never posted again this run. */
    private fun refreshSession(box: TokenBox) {
        val rt = box.sess.refreshToken.trim()
        if (rt.isEmpty()) throw PullException("missing refresh token")
        synchronized(refreshLock) {
            Store.loadSession()?.let { (cookie, wssdi) ->
                val stored = jsonWithAccessToken(cookie)
                val built = session(cookie, wssdi)
                if (stored != null && built != null && built.accessToken.isNotEmpty() && built.refreshToken.isNotEmpty() && built.refreshToken != rt) {
                    box.sess = built
                    box.oauth = stored
                    return
                }
            }
            if (rt == refusedRefreshToken) throw PullException(REFUSED_LOGIN_MESSAGE)
            refreshSessionLocked(box, rt)
        }
    }

    private fun refreshSessionLocked(box: TokenBox, rt: String) {
        val cid = box.sess.clientId.trim()
        if (cid.isEmpty()) throw PullException("session has no client id")
        val headers = sessionHeaders(box.sess, mapOf("x-wealthsimple-client" to WS_CLIENT, "x-ws-profile" to "invest"))
        val body = JSONObject().put("grant_type", "refresh_token").put("refresh_token", rt).put("client_id", cid)
        val data = httpJSON("POST", TOKEN_URL, headers, body, 60_000, throwOnAuth = false)
        val access = str(data, "access_token")
        if (access.isEmpty()) {
            if (oauthErrorCode(data) == "invalid_grant") refusedRefreshToken = rt
            throw PullException(refreshFailureMessage(data))
        }
        box.oauth.put("access_token", access)
        box.sess.accessToken = access
        val newRt = str(data, "refresh_token")
        if (newRt.isNotEmpty()) {
            box.oauth.put("refresh_token", newRt)
            box.sess.refreshToken = newRt
        }
        expiresAtAsTimestamp(data)?.let {
            box.oauth.put("expires_at", it)
            box.sess.expiresAt = it
        }
        box.oauth.put("client_id", cid)
        box.sess.clientId = cid
        box.didRefresh = true
        persistCookie(box)
    }

    // MARK: HTTP

    private fun httpJSON(method: String, url: String, headers: Map<String, String>, body: JSONObject?, timeoutMs: Int, throwOnAuth: Boolean = true): JSONObject {
        val conn = URL(url).openConnection() as HttpURLConnection
        conn.requestMethod = method
        conn.connectTimeout = timeoutMs
        conn.readTimeout = timeoutMs
        conn.setRequestProperty("Accept", "application/json")
        for ((k, v) in headers) conn.setRequestProperty(k, v)
        if (body != null) {
            conn.setRequestProperty("Content-Type", "application/json")
            conn.doOutput = true
            conn.outputStream.use { it.write(body.toString().toByteArray(Charsets.UTF_8)) }
        }
        val status = conn.responseCode
        val stream = if (status >= 400) conn.errorStream else conn.inputStream
        val bytes = ByteArrayOutputStream()
        stream?.use { it.copyTo(bytes) }
        conn.disconnect()
        if (throwOnAuth && (status == 401 || status == 403)) throw UnauthorizedException()
        val text = bytes.toString("UTF-8")
        if (text.isEmpty()) return JSONObject().put("_http_status", status)
        val obj = try { JSONObject(text) } catch (e: Exception) { return JSONObject().put("error", "invalid_json").put("_http_status", status) }
        obj.put("_http_status", status)
        return obj
    }

    private fun sessionHeaders(sess: WSSession, extra: Map<String, String>): Map<String, String> {
        val h = extra.toMutableMap()
        if (sess.wssdi.isNotEmpty()) h["x-ws-device-id"] = sess.wssdi
        if (sess.sessionId.isNotEmpty()) h["x-ws-session-id"] = sess.sessionId
        return h
    }

    private fun tokenInfo(sess: WSSession): JSONObject {
        val headers = sessionHeaders(sess, mapOf("Authorization" to "Bearer " + sess.accessToken, "x-wealthsimple-client" to WS_CLIENT))
        return httpJSON("GET", TOKEN_INFO_URL, headers, null, 60_000)
    }

    private fun graphql(box: TokenBox, operation: String, variables: JSONObject, query: String): JSONObject {
        try {
            return graphqlOnce(box.sess, operation, variables, query)
        } catch (e: UnauthorizedException) {
            if (box.didRefresh) throw PullException("Wealthsimple token refresh HTTP 401")
            refreshSession(box)
            try {
                return graphqlOnce(box.sess, operation, variables, query)
            } catch (e2: UnauthorizedException) {
                throw PullException("Wealthsimple token refresh HTTP 401")
            }
        }
    }

    private fun graphqlOnce(sess: WSSession, operation: String, variables: JSONObject, query: String): JSONObject {
        val headers = sessionHeaders(sess, mapOf(
            "Authorization" to "Bearer " + sess.accessToken,
            "x-wealthsimple-client" to WS_CLIENT,
            "x-ws-profile" to "trade",
            "x-ws-api-version" to GRAPHQL_VERSION,
            "x-ws-locale" to "en-CA",
            "x-platform-os" to "web",
            "Origin" to "https://my.wealthsimple.com",
            "Referer" to "https://my.wealthsimple.com/app/trade",
        ))
        val body = JSONObject().put("operationName", operation).put("query", query).put("variables", variables)
        val data = httpJSON("POST", GRAPHQL_URL, headers, body, 90_000)
        val status = data.optInt("_http_status", 0)
        if (status == 401 || status == 403) throw UnauthorizedException()
        if (data.has("errors") && !data.isNull("errors")) {
            val errs = data.opt("errors")
            val first: Any? = if (errs is JSONArray && errs.length() > 0) errs.opt(0) else errs
            var emsg = first.toString()
            if (first is JSONObject) {
                emsg = str(first, "message")
                if (emsg.isEmpty()) emsg = str(first, "error")
                if (emsg.isEmpty()) emsg = first.toString()
            }
            throw PullException("$operation: $emsg")
        }
        return data.optJSONObject("data") ?: throw PullException("graphql failed: $operation")
    }

    // MARK: the pull

    class PullResult(val activities: List<WSActivity>, val listings: List<WSSecurityListing>, val newRows: Boolean,
                     val nav: List<WSNavPoint>, val navByAccount: Map<String, List<WSNavPoint>>,
                     val accounts: List<WSAccountRow> = emptyList(), val balances: List<WSBalanceRow> = emptyList(), val margin: List<WSMarginRow> = emptyList())

    /** What Wealthsimple states per account and per cash row, read on every sync and every few minutes between. */
    class PortfolioSnapshot(val accounts: List<WSAccountRow>, val balances: List<WSBalanceRow>, val margin: List<WSMarginRow>)

    private fun slimAccounts(accounts: List<JSONObject>): List<WSAccountRow> = accounts.mapNotNull { a ->
        val id = str(a, "id")
        if (id.isEmpty()) null
        else {
            val fin = dict(dict(a.opt("financials")).opt("currentCombined"))
            val (amt, _) = moneyAmount(fin, listOf("netLiquidationValue", "netLiquidationValueV2"))
            WSAccountRow(id, str(a, "nickname"), str(a, "currency"), amt, str(a, "unifiedAccountType"), str(a, "status"))
        }
    }

    /** bagholder.margin_account_ids: the open margin accounts, the only ones whose buying power is
     *  margin available. Every self-directed account answers the query with the cash it could buy
     *  with, and cash, card and crypto accounts with an error; neither is margin. */
    fun marginAccountIds(rows: List<WSAccountRow>): List<String> =
        rows.filter { it.id.isNotEmpty() && it.unifiedAccountType.uppercase().contains("MARGIN") && it.status.lowercase() != "closed" }.map { it.id }

    private fun fetchBalances(box: TokenBox, accountIds: List<String>): List<WSBalanceRow> {
        val out = mutableListOf<WSBalanceRow>()
        val ids = accountIds.filter { it.isNotEmpty() }
        for (chunk in ids.chunked(20)) {
            val variables = JSONObject().put("ids", JSONArray(chunk)).put("type", "TRADING")
            val data = graphql(box, "FetchAccountsWithBalance", variables, Queries.FETCH_ACCOUNTS_WITH_BALANCE)
            val accounts = arr(data.opt("accounts"))
            for (i in 0 until accounts.length()) {
                val acc = dict(accounts.opt(i))
                val aid = str(acc, "id")
                val cas = arr(acc.opt("custodianAccounts"))
                for (j in 0 until cas.length()) {
                    val fin = dict(dict(cas.opt(j)).opt("financials"))
                    val raw = fin.opt("balance")
                    val bals = if (raw is JSONArray) raw else JSONArray().apply { if (raw is JSONObject) put(raw) }
                    for (k in 0 until bals.length()) {
                        val b = dict(bals.opt(k))
                        out.add(WSBalanceRow(aid, str(b, "securityId"), b.optDouble("quantity", 0.0)))
                    }
                }
            }
        }
        return out
    }

    /** bagholder.parse_margin: a Money when available, the reason when not, null when the account has no margin figures. */
    private fun parseMargin(data: JSONObject): WSMarginRow? {
        val trading = dict(dict(dict(dict(dict(data.opt("account")).opt("financials")).opt("current")).opt("marginV3")).opt("trading"))
        val bp = trading.opt("buyingPower") as? JSONObject ?: return null
        if (bp.optString("__typename") == "BuyingPowerMetricAvailable") {
            val total = dict(bp.opt("total"))
            val amount = total.optString("amount").toDoubleOrNull() ?: return null
            return WSMarginRow("", amount, total.optString("currency").ifEmpty { "CAD" }, "")
        }
        val reason = dict(bp.opt("reason"))
        var why = reason.optString("__typename").ifEmpty { bp.optString("__typename").ifEmpty { "unavailable" } }
        val n = arr(reason.opt("securities")).length()
        if (n > 0) why += " ($n securities)"
        return WSMarginRow("", null, "CAD", why)
    }

    private fun fetchMargin(box: TokenBox, accountIds: List<String>): List<WSMarginRow> {
        val out = mutableListOf<WSMarginRow>()
        for (aid in accountIds.filter { it.isNotEmpty() }) {
            val data = try {
                graphql(box, "FetchAccountCurrentMarginBuyingPowerV2", JSONObject().put("accountId", aid).put("currency", "CAD"), Queries.FETCH_ACCOUNT_MARGIN_BUYING_POWER)
            } catch (e: Exception) { continue }
            val row = parseMargin(data) ?: continue
            out.add(WSMarginRow(aid, row.buyingPower, row.currency, row.unavailable))
        }
        return out
    }

    private fun portfolioSnapshot(box: TokenBox, accounts: List<JSONObject>): PortfolioSnapshot {
        val rows = slimAccounts(accounts)
        val ids = rows.map { it.id }
        val balances = try { fetchBalances(box, ids) } catch (e: Exception) { emptyList() }
        return PortfolioSnapshot(rows, balances, fetchMargin(box, marginAccountIds(rows)))
    }

    /** The Portfolio figures between syncs: net liquidation values, cash balances and buying power. */
    fun refreshPortfolio(oauthCookie: String, wssdi: String?): PortfolioSnapshot {
        val oauthObj = jsonWithAccessToken(oauthCookie) ?: throw PullException("No Wealthsimple session")
        val built = session(oauthCookie, wssdi) ?: throw PullException("No Wealthsimple session")
        val box = TokenBox(built, oauthObj)
        ensureClientId(box)
        if (tokenNeedsRefresh(box.sess)) refreshSession(box)
        if (box.sess.identityCanonicalId.isEmpty()) {
            val info = try { tokenInfo(box.sess) } catch (e: UnauthorizedException) { refreshSession(box); tokenInfo(box.sess) }
            box.sess.identityCanonicalId = identityFrom(info)
        }
        if (box.sess.identityCanonicalId.isEmpty()) throw PullException("Wealthsimple session has no identity")
        val accounts = fetchAllAccounts(box, box.sess.identityCanonicalId)
        return portfolioSnapshot(box, accounts)
    }

    fun run(
        oauthCookie: String, wssdi: String?, storedActivities: List<WSActivity>, storedListings: List<WSSecurityListing>,
        storedNav: List<WSNavPoint> = emptyList(), storedNavByAccount: Map<String, List<WSNavPoint>> = emptyMap(),
        onProgress: (String) -> Unit,
    ): PullResult {
        val oauthObj = jsonWithAccessToken(oauthCookie) ?: throw PullException("No Wealthsimple session")
        val built = session(oauthCookie, wssdi) ?: throw PullException("No Wealthsimple session")
        val box = TokenBox(built, oauthObj)
        ensureClientId(box)
        if (tokenNeedsRefresh(box.sess)) refreshSession(box)
        if (box.sess.identityCanonicalId.isEmpty()) {
            val info = try {
                tokenInfo(box.sess)
            } catch (e: UnauthorizedException) {
                refreshSession(box)
                tokenInfo(box.sess)
            }
            box.sess.identityCanonicalId = identityFrom(info)
            if (box.sess.clientId.isEmpty()) {
                val cid = clientIdFromTokenInfo(info)
                if (cid.isNotEmpty()) {
                    box.sess.clientId = cid
                    box.oauth.put("client_id", cid)
                    persistCookie(box)
                }
            }
        }
        if (box.sess.identityCanonicalId.isEmpty()) throw PullException("Wealthsimple session has no identity")

        onProgress("Fetching accounts…")
        val accounts = fetchAllAccounts(box, box.sess.identityCanonicalId)
        onProgress("Fetching balances…")
        val portfolio = portfolioSnapshot(box, accounts)
        onProgress("Checking for new rows…")
        val accById = HashMap<String, JSONObject>()
        for (a in accounts) {
            val aid = str(a, "id")
            if (aid.isNotEmpty()) accById[aid] = a
        }
        val pools = fifoPoolIds(accounts)
        val (startDate, fullHistory) = activitySyncBounds(storedActivities)
        val known: Set<String> = if (fullHistory) emptySet() else knownCanonicalIds(storedActivities)
        val storedKeys = storedActivities.map { activityMergeKey(it) }.filter { it.isNotEmpty() }.toSet()
        val mapped = mutableListOf<WSActivity>()
        class Work(val account: JSONObject, val items: MutableList<JSONObject>, var nextCursor: String?)
        val work = mutableListOf<Work>()
        for (acc in accounts) {
            val aid = str(acc, "id")
            if (aid.isEmpty()) continue
            val page = fetchActivityPage(box, aid, startDate, known, null)
            var hasNew = page.newCount > 0
            if (!hasNew) {
                loop@ for (node in page.nodes) {
                    for (row in mapActivityRows(node, accById)) {
                        val k = activityMergeKey(row)
                        if (k.isNotEmpty() && !storedKeys.contains(k)) { hasNew = true; break@loop }
                    }
                }
            }
            if (hasNew) work.add(Work(acc, page.nodes.toMutableList(), page.nextCursor))
        }
        if (work.isEmpty()) onProgress("No new transactions")
        for ((i, w) in work.withIndex()) {
            val aid = str(w.account, "id")
            val nick = accountType(aid, accById)
            onProgress(if (nick.isEmpty()) "Syncing transactions (${i + 1}/${work.size})" else "Syncing transactions (${i + 1}/${work.size}) $nick")
            while (w.nextCursor != null) {
                val page = fetchActivityPage(box, aid, startDate, known, w.nextCursor)
                w.items.addAll(page.nodes)
                w.nextCursor = page.nextCursor
            }
            for (it in w.items) mapped.addAll(mapActivityRows(it, accById))
        }
        for (a in mapped) a.fifoId = pools[a.accountId] ?: a.accountId
        val merged = mergeActivities(storedActivities, mapped)
        for (a in merged) a.fifoId = pools[a.accountId] ?: a.accountId
        onProgress("Fetching equity history…")
        val sinceNav = if (navMissingDeposits(storedNav)) null else storedNav.map { it.date }.filter { it.isNotEmpty() }.maxOrNull()
        val nav = mergeNav(storedNav, fetchNavHistory(box, box.sess.identityCanonicalId, sinceNav))
        val navByAccount = LinkedHashMap(storedNavByAccount)
        for ((nick, ids) in navAccountGroups(accounts)) {
            onProgress("Fetching equity history for $nick…")
            val storedSeries = navByAccount[nick] ?: emptyList()
            val sinceNick = if (navMissingDeposits(storedSeries)) null else storedSeries.map { it.date }.filter { it.isNotEmpty() }.maxOrNull()
            try {
                val series = ids.map { fetchAccountNavHistory(box, it, sinceNick) }
                navByAccount[nick] = mergeNav(storedSeries, mergeNavPoints(series))
            } catch (e: Exception) {
                if (!navByAccount.containsKey(nick) && storedSeries.isNotEmpty()) navByAccount[nick] = storedSeries
            }
        }
        onProgress("Fetching listings…")
        val listings = storedListings + fetchListings(box, merged, storedListings)
        return PullResult(merged, listings, work.isNotEmpty(), nav, navByAccount, portfolio.accounts, portfolio.balances, portfolio.margin)
    }

    // MARK: NAV history

    private fun navMissingDeposits(stored: List<WSNavPoint>): Boolean {
        var seen = false
        for (p in stored) {
            val y = p.date.take(4)
            if (y != "2024" && y != "2025" && y != "2026") continue
            seen = true
            if (p.netDeposits == null) return true
        }
        return !seen
    }

    private fun mergeNav(stored: List<WSNavPoint>, incoming: List<WSNavPoint>): List<WSNavPoint> {
        val byDate = HashMap<String, WSNavPoint>()
        for (r in stored) byDate[r.date] = r
        for (r in incoming) {
            val old = byDate[r.date]
            byDate[r.date] = if (r.netDeposits != null) r else if (old?.netDeposits != null) r.copy(netDeposits = old.netDeposits) else r
        }
        return byDate.keys.sorted().map { byDate[it]!! }
    }

    private fun mergeNavPoints(seriesList: List<List<WSNavPoint>>): List<WSNavPoint> {
        val byDate = HashMap<String, WSNavPoint>()
        for (series in seriesList) for (rec in series) {
            val d = rec.date.take(10)
            if (d.isEmpty()) continue
            val cur = byDate[d]
            byDate[d] = if (cur == null) WSNavPoint(d, rec.equity, rec.currency.ifEmpty { "CAD" }, rec.netDeposits)
            else cur.copy(equity = cur.equity + rec.equity, currency = rec.currency.ifEmpty { cur.currency },
                netDeposits = if (rec.netDeposits != null) (cur.netDeposits ?: 0.0) + rec.netDeposits else cur.netDeposits)
        }
        return byDate.keys.sorted().map { byDate[it]!! }
    }

    private fun navAccountGroups(accounts: List<JSONObject>): List<Pair<String, List<String>>> {
        val groups = HashMap<String, MutableList<String>>()
        for (acc in accounts) {
            val aid = str(acc, "id").trim()
            if (aid.isEmpty()) continue
            val nick = str(acc, "nickname").trim().ifEmpty { str(acc, "unifiedAccountType").trim().ifEmpty { str(acc, "type").trim() } }
            if (nick.isEmpty()) continue
            val ids = groups.getOrPut(nick) { mutableListOf() }
            if (!ids.contains(aid)) ids.add(aid)
        }
        return groups.keys.sorted().map { Pair(it, groups[it]!!) }
    }

    private fun moneyAmount(node: JSONObject, keys: List<String>): Pair<Double?, String> {
        for (key in keys) {
            val money = dict(node.opt(key))
            if (!money.has("amount") || money.isNull("amount")) continue
            val amt = num(money.opt("amount"), Double.NaN)
            if (amt.isNaN()) continue
            return Pair(amt, str(money, "currency"))
        }
        return Pair(null, "")
    }

    private fun navPointsFromPayload(data: JSONObject): Pair<List<WSNavPoint>, JSONObject> {
        val ident = dict(data.opt("identity"))
        val acc = dict(data.opt("account"))
        val fin = if (ident.has("financials")) dict(ident.opt("financials")) else dict(acc.opt("financials"))
        val hist = dict(fin.opt("historicalDaily"))
        val points = mutableListOf<WSNavPoint>()
        val edges = arr(hist.opt("edges"))
        for (i in 0 until edges.length()) {
            val node = dict(dict(edges.opt(i)).opt("node"))
            val (amt, cur) = moneyAmount(node, listOf("netLiquidationValue", "netLiquidationValueV2"))
            val d = str(node, "date").take(10)
            if (d.isEmpty() || amt == null) continue
            val (nd, _) = moneyAmount(node, listOf("netDeposits", "netDepositsV2"))
            points.add(WSNavPoint(d, amt, cur.ifEmpty { "CAD" }, nd))
        }
        return Pair(points, dict(hist.opt("pageInfo")))
    }

    private fun paginateNavHistory(box: TokenBox, operation: String, query: String, extra: JSONObject, sinceDate: String?): List<WSNavPoint> {
        val today = utcFormat("yyyy-MM-dd").format(Date())
        val sinceDay = (sinceDate ?: "").trim().take(10)
        if (sinceDay.isNotEmpty() && sinceDay > today) return emptyList()
        val year0 = if (sinceDay.length >= 4) sinceDay.take(4).toIntOrNull() ?: 2020 else 2020
        val year1 = today.take(4).toIntOrNull() ?: year0
        if (year1 < year0) return emptyList()
        val points = mutableListOf<WSNavPoint>()
        for (year in year0..year1) {
            var start = "$year-01-01"
            if (sinceDay.isNotEmpty() && start < sinceDay) start = sinceDay
            val end = if (year == year1) today else "$year-12-31"
            if (start > end) continue
            var cursor: String? = null
            for (page in 0 until 8) {
                val variables = JSONObject(extra.toString()).put("startDate", start).put("endDate", end)
                if (cursor != null) variables.put("cursor", cursor)
                val data = graphql(box, operation, variables, query)
                val (chunk, info) = navPointsFromPayload(data)
                points.addAll(chunk)
                if (!info.optBoolean("hasNextPage", false)) break
                val next = str(info, "endCursor")
                if (next.isEmpty()) break
                cursor = next
            }
        }
        val byDate = HashMap<String, WSNavPoint>()
        for (r in points) byDate[r.date] = r
        return byDate.keys.sorted().map { byDate[it]!! }
    }

    private fun fetchNavHistory(box: TokenBox, identityId: String, sinceDate: String?): List<WSNavPoint> =
        paginateNavHistory(box, "IdentityHistoricalFinancialsQuery", Queries.IDENTITY_HISTORICAL_FINANCIALS,
            JSONObject().put("identityId", identityId).put("currency", "CAD").put("limit", 400).put("includeNetDeposits", true), sinceDate)

    private fun fetchAccountNavHistory(box: TokenBox, accountId: String, sinceDate: String?): List<WSNavPoint> {
        val aid = accountId.trim()
        if (aid.isEmpty()) return emptyList()
        return paginateNavHistory(box, "FetchAccountHistoricalFinancials", Queries.FETCH_ACCOUNT_HISTORICAL_FINANCIALS,
            JSONObject().put("id", aid).put("currency", "CAD").put("resolution", "DAILY").put("first", 400), sinceDate)
    }

    // MARK: the S&P 500 from FRED

    /** FRED's daily closes as YYYY-MM-DD -> close; empty on any failure. */
    fun fetchSp500(): Map<String, Double> {
        val out = HashMap<String, Double>()
        try {
            val conn = URL("https://fred.stlouisfed.org/graph/fredgraph.csv?id=SP500").openConnection() as HttpURLConnection
            conn.connectTimeout = 45_000; conn.readTimeout = 45_000
            conn.setRequestProperty("Accept", "text/csv,*/*")
            val text = conn.inputStream.bufferedReader().use { it.readText() }
            conn.disconnect()
            for (line in text.replace("\r\n", "\n").replace("\r", "\n").split("\n")) {
                val parts = line.split(",")
                if (parts.size < 2) continue
                val d = parts[0].trim().trim('"').take(10)
                val raw = parts[1].trim().trim('"')
                if (!Regex("^\\d{4}-\\d{2}-\\d{2}$").matches(d) || raw.isEmpty() || raw == ".") continue
                val px = raw.toDoubleOrNull() ?: continue
                if (px > 0) out[d] = px
            }
        } catch (e: Exception) {
        }
        return out
    }

    private fun fetchAllAccounts(box: TokenBox, identityId: String): List<JSONObject> {
        val accounts = mutableListOf<JSONObject>()
        var cursor: String? = null
        while (true) {
            val variables = JSONObject().put("identityId", identityId).put("pageSize", 25).put("startDate", "2015-01-01")
            if (cursor != null) variables.put("cursor", cursor)
            val data = graphql(box, "FetchAllAccountFinancials", variables, Queries.FETCH_ALL_ACCOUNT_FINANCIALS)
            val conn = dict(dict(data.opt("identity")).opt("accounts"))
            val edges = arr(conn.opt("edges"))
            for (i in 0 until edges.length()) {
                val node = dict(dict(edges.opt(i)).opt("node"))
                if (node.length() > 0) accounts.add(node)
            }
            val page = dict(conn.opt("pageInfo"))
            if (!page.optBoolean("hasNextPage", false)) break
            val next = str(page, "endCursor")
            if (next.isEmpty()) break
            cursor = next
        }
        return accounts
    }

    private fun activityFetchCondition(accountId: String, startDate: String?): JSONObject {
        val end = Date(System.currentTimeMillis() + 86_400_000L)
        val endDate = utcFormat("yyyy-MM-dd'T'HH:mm:ss").format(end) + ".999Z"
        val cond = JSONObject().put("endDate", endDate).put("accountIds", JSONArray().put(accountId))
        if (startDate != null) {
            var raw = startDate.trim()
            if (raw.isNotEmpty()) {
                if (!raw.contains("T")) raw = raw.take(10) + "T00:00:00.000Z"
                cond.put("startDate", raw)
            }
        }
        return cond
    }

    private fun activitySyncBounds(stored: List<WSActivity>): Pair<String?, Boolean> {
        if (stored.isEmpty()) return Pair(null, true)
        var newest = ""
        for (a in stored) {
            val occurred = a.occurredAt.trim()
            val pick = if (occurred.isEmpty()) a.transactionDate.trim() else occurred
            if (pick > newest) newest = pick
        }
        if (newest.isEmpty()) return Pair(null, false)
        val start = newest.substringBefore("T").take(10)
        if (start.length < 10) return Pair(null, false)
        return Pair(start, false)
    }

    private fun knownCanonicalIds(stored: List<WSActivity>) = stored.map { it.canonicalId.trim() }.filter { it.isNotEmpty() }.toSet()

    private fun activityMergeKey(a: WSActivity): String {
        val cid = a.canonicalId.trim()
        return if (cid.isNotEmpty()) cid else a.id.trim()
    }

    private fun mergeActivities(stored: List<WSActivity>, incoming: List<WSActivity>): List<WSActivity> {
        val byKey = LinkedHashMap<String, WSActivity>()
        val noKey = mutableListOf<WSActivity>()
        fun put(a: WSActivity) {
            val k = activityMergeKey(a)
            if (k.isEmpty()) { noKey.add(a); return }
            if (!byKey.containsKey(k)) byKey[k] = a
        }
        for (a in stored) put(a)
        for (a in incoming) put(a)
        return byKey.values.toList() + noKey
    }

    private class Page(val nodes: List<JSONObject>, val newCount: Int, val nextCursor: String?)

    private fun fetchActivityPage(box: TokenBox, accountId: String, startDate: String?, known: Set<String>, cursor: String?): Page {
        val variables = JSONObject().put("first", 100).put("orderBy", "OCCURRED_AT_DESC")
            .put("condition", activityFetchCondition(accountId, startDate))
        if (cursor != null) variables.put("cursor", cursor)
        val data = graphql(box, "FetchActivityFeedItems", variables, Queries.FETCH_ACTIVITY_FEED_ITEMS)
        val feed = dict(data.opt("activityFeedItems"))
        val nodes = mutableListOf<JSONObject>()
        var newOnPage = 0
        val edges = arr(feed.opt("edges"))
        for (i in 0 until edges.length()) {
            val node = dict(dict(edges.opt(i)).opt("node"))
            if (node.length() == 0) continue
            nodes.add(node)
            val cid = str(node, "canonicalId").trim()
            if (cid.isNotEmpty() && !known.contains(cid)) newOnPage++
        }
        val bounded = !startDate.isNullOrBlank()
        if (bounded && known.isNotEmpty() && newOnPage == 0) return Page(nodes, newOnPage, null)
        val page = dict(feed.opt("pageInfo"))
        if (!page.optBoolean("hasNextPage", false)) return Page(nodes, newOnPage, null)
        val next = str(page, "endCursor")
        if (next.isEmpty()) return Page(nodes, newOnPage, null)
        return Page(nodes, newOnPage, next)
    }

    private fun fetchSecurity(box: TokenBox, securityId: String): WSSecurityListing? {
        val sid = securityId.trim()
        if (sid.isEmpty()) return null
        return try {
            val data = graphql(box, "FetchSecurity", JSONObject().put("securityId", sid), Queries.FETCH_SECURITY)
            val sec = dict(data.opt("security"))
            if (sec.length() == 0) return null
            val stock = dict(sec.opt("stock"))
            val under = dict(dict(sec.opt("optionDetails")).opt("underlyingSecurity"))
            val id = str(sec, "id")
            WSSecurityListing(
                id.ifEmpty { sid }, str(stock, "symbol"), str(stock, "name"), str(stock, "primaryExchange"),
                str(stock, "primaryMic"), str(sec, "currency"), str(under, "id"),
            )
        } catch (e: Exception) {
            null
        }
    }

    private fun fetchListings(box: TokenBox, activities: List<WSActivity>, have: List<WSSecurityListing>): List<WSSecurityListing> {
        val named = have.filter { it.id.trim().isNotEmpty() && it.name.isNotEmpty() }.map { it.id.trim() }.toSet()
        val seen = HashSet<String>()
        val ids = mutableListOf<String>()
        for (a in activities) {
            val sid = a.securityId.trim()
            if (sid.isEmpty() || seen.contains(sid) || named.contains(sid)) continue
            seen.add(sid)
            ids.add(sid)
        }
        if (ids.isEmpty()) return emptyList()
        val byId = LinkedHashMap<String, WSSecurityListing>()
        val underIds = mutableListOf<String>()
        for (sid in ids) {
            val rec = fetchSecurity(box, sid) ?: continue
            byId[rec.id] = rec
            val uid = rec.underlyingId.trim()
            if (uid.isNotEmpty() && !seen.contains(uid) && !named.contains(uid)) {
                seen.add(uid)
                underIds.add(uid)
            }
        }
        for (uid in underIds) {
            val rec = fetchSecurity(box, uid) ?: continue
            byId[rec.id] = rec
        }
        return byId.values.toList()
    }

    // MARK: mapping (crates/ws/src/mapping.rs)

    private fun upper(v: Any?): String = (if (v == null || v == JSONObject.NULL) "" else v.toString()).trim().uppercase()
    private fun compact(v: Any?): String = upper(v).replace(Regex("[\\s_\\-]+"), "")
    private fun s(v: Any?): String = if (v == null || v == JSONObject.NULL) "" else v.toString()

    private fun dateOnly(occurred: String): String {
        val t = occurred.trim()
        if (t.isEmpty()) return ""
        return t.substringBefore("T").take(10)
    }

    private fun assetSymbol(item: JSONObject, key: String = "assetSymbol"): String {
        var raw = s(item.opt(key)).trim()
        if (raw.uppercase().startsWith("EXCHANGE:")) raw = raw.substringAfter(":")
        return raw.uppercase().trim()
    }

    private fun typeBlob(item: JSONObject): Triple<String, String, String> {
        val typ = upper(item.opt("type")).replace("-", "_")
        val sub = upper(item.opt("subType")).replace("-", "_")
        val parts = listOf(typ, sub, s(item.opt("aftTransactionType")), s(item.opt("aftTransactionCategory"))).filter { it.isNotEmpty() }.map { compact(it) }
        return Triple(typ, sub, parts.joinToString("_"))
    }

    private fun isCorpShareMove(item: JSONObject): Boolean {
        val blob = typeBlob(item).third
        if (CORP_BLOBS.any { blob.contains(it) }) return true
        val qty = abs(num(item.opt("assetQuantity")))
        val cash = abs(num(item.opt("amount")))
        if (qty > 0 && assetSymbol(item).isNotEmpty() && cash == 0.0 && (compact(item.opt("type")).contains("DIVIDEND") || blob.contains("DISTRIBUT"))) return true
        return false
    }

    private fun isCodeChange(item: JSONObject): Boolean {
        val blob = typeBlob(item).third
        return listOf("CODECHANGE", "SYMBOLCHANGE", "TICKERCHANGE", "LISTINGSTATUS", "SECURITYSWAP", "MANDATORYEXCHANGE", "NAMECHANGE").any { blob.contains(it) }
    }

    private fun skipActivity(item: JSONObject): Boolean {
        if (item.length() == 0) return true
        if (s(item.opt("occurredAt")).trim().isEmpty()) return true
        val status = compact(item.opt("status"))
        val (typ, sub, blob) = typeBlob(item)
        if (isCorpShareMove(item)) {
            if (status.contains("REJECT") || status.contains("CANCEL") || status.contains("FAIL") || status.contains("VOID")) return true
        } else if ((typ == "DIVIDEND" || typ == "INTEREST_CHARGE") && status.isEmpty()) {
            // crates/ws/src/mapping.rs skip_activity: cash dividends and margin interest charges often
            // arrive with no status at all; both have already hit the cash balance
        } else if (status.isEmpty() || !KEEP_STATUS.contains(status)) {
            return true
        }
        if (typ == "LOAN" || typ == "RECALL" || sub == "LOAN" || sub == "RECALL") return true
        if (typ.endsWith("_LOAN") || sub.endsWith("_LOAN")) return true
        if (typ.endsWith("_RECALL") || sub.endsWith("_RECALL")) return true
        if (SKIP_MARKERS.any { blob.contains(it) }) return true
        return false
    }

    private fun optionSymbol(item: JSONObject): String {
        val under = assetSymbol(item)
        val contract = item.opt("contractType")
        val strike = item.opt("strikePrice")
        val expiry = item.opt("expiryDate")
        val hasContract = s(contract).isNotEmpty()
        if (!hasContract || strike == null || strike == JSONObject.NULL || expiry == null || expiry == JSONObject.NULL || under.isEmpty()) return under
        val ds = s(expiry).trim().substringBefore("T")
        val parts = ds.replace("/", "-").take(10).split("-")
        if (parts.size != 3) return under
        val year = parts[0].toIntOrNull() ?: return under
        val month = parts[1].toIntOrNull() ?: return under
        val day = parts[2].toIntOrNull() ?: return under
        if (month < 1 || month > 12) return under
        val strikeF = num(strike, Double.NaN)
        if (strikeF.isNaN()) return under
        var cp = upper(contract)
        if (cp == "C" || cp == "CALL") cp = "CALL" else if (cp == "P" || cp == "PUT") cp = "PUT"
        return String.format(Locale.ROOT, "%s %02d%s%02d %.2f %s", under, day, MONTHS[month - 1], year % 100, strikeF, cp)
    }

    private fun signedCash(item: JSONObject): Double {
        val amount = abs(num(item.opt("amount")))
        val typ = upper(item.opt("type")).replace("-", "_")
        val sub = upper(item.opt("subType")).replace("-", "_")
        if (typ == "DIY_BUY" || typ == "OPTIONS_BUY" || typ == "WITHDRAWAL" || (typ == "INTERNAL_TRANSFER" && sub.contains("SOURCE"))) return -amount
        if (typ in listOf("DIY_SELL", "OPTIONS_SELL", "DEPOSIT", "CONTRIBUTION", "DIVIDEND", "INTEREST") || (typ == "INTERNAL_TRANSFER" && sub.contains("DESTINATION"))) return amount
        val sign = s(item.opt("amountSign")).trim().lowercase()
        if (sign in listOf("negative", "debit", "-", "neg")) return -amount
        if (sign in listOf("positive", "credit", "+", "pos")) return amount
        if (!item.has("amount") || s(item.opt("amount")).isEmpty()) return 0.0
        return num(item.opt("amount"))
    }

    private fun accountType(accountId: String, accounts: Map<String, JSONObject>): String {
        val rec = accounts[accountId] ?: return ""
        val nick = str(rec, "nickname")
        if (nick.isNotEmpty()) return nick
        val u = str(rec, "unifiedAccountType")
        if (u.isNotEmpty()) return u
        return str(rec, "type")
    }

    private fun fifoPoolIds(recs: List<JSONObject>): Map<String, String> {
        val parent = HashMap<String, String>()
        fun find(x0: String): String {
            var x = x0
            if (parent[x] == null) parent[x] = x
            while (parent[x] != x) {
                parent[x] = parent[parent[x] ?: x] ?: x
                x = parent[x] ?: x
            }
            return x
        }
        fun union(a: String, b: String) {
            if (a.isEmpty() || b.isEmpty()) return
            val ra = find(a); val rb = find(b)
            if (ra != rb) parent[maxOf(ra, rb)] = minOf(ra, rb)
        }
        val byNick = HashMap<String, MutableList<String>>()
        for (a in recs) {
            val aid = str(a, "id")
            if (aid.isEmpty()) continue
            find(aid)
            val lid = str(dict(a.opt("linkedAccount")), "id")
            if (lid.isNotEmpty()) union(aid, lid)
            val nick = str(a, "nickname").trim()
            if (nick.isNotEmpty()) byNick.getOrPut(nick) { mutableListOf() }.add(aid)
        }
        for (ids in byNick.values) {
            val root = ids.firstOrNull() ?: continue
            for (other in ids.drop(1)) union(root, other)
        }
        val out = HashMap<String, String>()
        for (aid in parent.keys.toList()) out[aid] = find(aid)
        return out
    }

    private fun isOption(item: JSONObject) = s(item.opt("contractType")).isNotEmpty()

    private fun isToClose(sub: String): Boolean {
        val c = compact(sub)
        return c.contains("TOCLOSE") || c == "BTC" || c == "STC"
    }

    private fun mapActivityRows(item: JSONObject, accounts: Map<String, JSONObject>): List<WSActivity> {
        if (item.length() == 0) return emptyList()
        val src = assetSymbol(item)
        val dst = assetSymbol(item, "counterAssetSymbol")
        val qty = abs(num(item.opt("assetQuantity")))
        if (src.isNotEmpty() && dst.isNotEmpty() && src != dst && qty > 0 && isCorpShareMove(item)) {
            val cid = s(item.opt("canonicalId")).trim()
            val base = cid.ifEmpty { "swap" }
            val outgoing = JSONObject(item.toString())
            outgoing.put("assetSymbol", src).put("counterAssetSymbol", "").put("type", "STKDIS").put("subType", "STKDIS")
                .put("assetQuantity", -qty).put("amount", 0).put("amountSign", "negative").put("canonicalId", "$base:out")
            val incoming = JSONObject(item.toString())
            incoming.put("assetSymbol", dst).put("counterAssetSymbol", "").put("type", "STKDIS").put("subType", "STKDIS")
                .put("assetQuantity", qty).put("amount", 0).put("amountSign", "positive").put("canonicalId", "$base:in")
            return listOfNotNull(mapActivity(outgoing, accounts), mapActivity(incoming, accounts))
        }
        return listOfNotNull(mapActivity(item, accounts))
    }

    private fun mapActivity(item: JSONObject, accounts: Map<String, JSONObject>): WSActivity? {
        if (skipActivity(item)) return null
        val occurred = s(item.opt("occurredAt")).trim()
        val transactionDate = dateOnly(occurred)
        if (transactionDate.isEmpty()) return null
        val accountId = s(item.opt("accountId"))
        val typ = upper(item.opt("type")).replace("-", "_")
        val sub = upper(item.opt("subType")).replace("-", "_")
        val qtyRaw = num(item.opt("assetQuantity"))
        val qtyAbs = abs(qtyRaw)
        val cash = signedCash(item)
        val amountAbs = abs(num(item.opt("amount")))
        val fees = abs(num(item.opt("fees")))
        val opt = isOption(item)
        var symbol = if (opt) optionSymbol(item) else assetSymbol(item)
        var cur = upper(item.opt("currency"))
        if (cur != "CAD" && cur != "USD") cur = if (opt) "USD" else "CAD"
        var unitPrice = 0.0
        if (qtyAbs > 0) {
            unitPrice = amountAbs / qtyAbs
            if (opt && unitPrice > 20) unitPrice /= 100.0
        }
        var activityType = "Other"
        var activitySub = sub.ifEmpty { typ }
        var category = "other"
        var quantity = qtyAbs

        if (typ == "DIY_BUY") {
            category = "trade"; activityType = "Trade"
            activitySub = if (opt) (if (isToClose(sub)) "BUYTOCLOSE" else "BUYTOOPEN") else "BUY"
            quantity = qtyAbs
        } else if (typ == "DIY_SELL") {
            category = "trade"; activityType = "Trade"
            activitySub = if (opt) (if (isToClose(sub)) "SELLTOCLOSE" else "SELLTOOPEN") else "SELL"
            quantity = -qtyAbs
        } else if (typ == "OPTIONS_BUY") {
            category = "trade"; activityType = "Trade"
            activitySub = if (isToClose(sub)) "BUYTOCLOSE" else "BUYTOOPEN"
            quantity = qtyAbs
        } else if (typ == "OPTIONS_SELL") {
            category = "trade"; activityType = "Trade"
            activitySub = if (isToClose(sub)) "SELLTOCLOSE" else "SELLTOOPEN"
            quantity = -qtyAbs
        } else if (typ in listOf("EXPIR", "EXPIRY", "EXPIRE", "ASSIGN", "ASSIGNMENT", "EXERCISE")) {
            category = "option_event"
            activityType = if (typ.contains("ASSIGN")) "ASSIGN" else if (typ.contains("EXERCISE")) "EXERCISE" else "EXPIR"
            val covering = typ.contains("ASSIGN") || compact(sub).contains("COVER") || isToClose(sub)
            activitySub = if (covering) "BUY" else "SELL"
            quantity = if (activitySub == "SELL") -qtyAbs else qtyAbs
            if (!opt && symbol.isEmpty()) symbol = assetSymbol(item)
        } else if (typ == "DEPOSIT" || typ == "CONTRIBUTION") {
            activityType = "Deposit"; activitySub = "deposit"; category = "deposit"
        } else if (typ == "WITHDRAWAL") {
            activityType = "Withdrawal"; activitySub = "withdrawal"; category = "withdrawal"
        } else if (typ == "INTERNAL_TRANSFER" || compact(typ) in listOf("TRFIN", "TRFOUT", "TRANSFERIN", "TRANSFEROUT", "INTERNALTRANSFER")) {
            activityType = "Transfer"; activitySub = "transfer"; category = "transfer"
        } else if (typ == "DIVIDEND" && !isCorpShareMove(item)) {
            activityType = "Dividend"; activitySub = "dividend"; category = "dividend"
        } else if (typ == "INTEREST" || sub.contains("FPL_INTEREST") || compact(typ) == "FPLINTEREST") {
            activityType = "Interest"; activitySub = "interest"; category = "interest"
        } else if (typ == "FUNDS_CONVERSION") {
            activityType = "FxExchange"; activitySub = "fx"; category = "fx"
        } else if (typ == "FEE" || typ == "REFUND") {
            activityType = if (typ == "REFUND") "Refund" else "Fee"; activitySub = "fee"; category = "fee"
        } else if (isCorpShareMove(item) || typ in listOf("STOCK_DISTRIBUTION", "STKDIS", "SPIN", "SPINOFF", "STK_DIS") ||
            compact(item.opt("type")).contains("STKDIS") || compact(item.opt("type")).contains("STOCKDISTRIBUTION") ||
            compact(item.opt("subType")).contains("STOCKDISTRIBUTION")) {
            activityType = "STKDIS"; category = "trade"
            unitPrice = 0.0
            val sign = s(item.opt("amountSign")).trim().lowercase()
            var outgoing = qtyRaw < 0 || sign in listOf("negative", "debit", "-", "neg")
            if (!outgoing && isCodeChange(item) && assetSymbol(item, "counterAssetSymbol").isEmpty() && !compact(item.opt("type")).contains("STKDIS")) outgoing = true
            if (outgoing) { activitySub = "SELL"; quantity = -qtyAbs } else { activitySub = "BUY"; quantity = qtyAbs }
        } else {
            activityType = s(item.opt("type")).ifEmpty { "Other" }
            activitySub = s(item.opt("subType")).ifEmpty { "other" }
            category = "other"
        }

        if (activitySub in listOf("SELL", "SELLTOOPEN", "SELLTOCLOSE")) {
            quantity = if (qtyAbs > 0) -qtyAbs else quantity
        }

        val sign = s(item.opt("amountSign")).trim().lowercase()
        var direction = ""
        if (sign in listOf("negative", "debit", "-", "neg") || cash < 0) direction = "DEBIT"
        else if (sign in listOf("positive", "credit", "+", "pos") || cash > 0) direction = "CREDIT"

        val cid = s(item.opt("canonicalId")).trim()
        val id = cid.ifEmpty { "$occurred|$accountId|$symbol|$quantity" }
        val name = s(item.opt("aftOriginatorName")).ifEmpty { s(item.opt("institutionName")).ifEmpty { symbol } }
        return WSActivity(
            id, cid, occurred, transactionDate, accountId, accountId, accountType(accountId, accounts), activityType, activitySub,
            "", direction, symbol, name, cur, quantity, unitPrice, fees, cash, category, s(item.opt("type")),
            s(item.opt("aftTransactionType")), assetSymbol(item, "counterAssetSymbol"), s(item.opt("securityId")),
        )
    }

    // MARK: Bank of Canada FXUSDCAD

    fun ensureFxRates(activities: List<WSActivity>, cached: Map<String, Double>): Map<String, Double> {
        val dates = activities.map { it.transactionDate }.filter { it.isNotEmpty() }.sorted()
        val start = dates.firstOrNull() ?: "2020-01-01"
        val end = utcFormat("yyyy-MM-dd").format(Date())
        val keys = cached.keys.sorted()
        if (keys.isNotEmpty() && keys.first() <= start && keys.last() >= com.bagholder.model.Model.shiftDate(end, -5)) return cached
        val map = cached.toMutableMap()
        try {
            val url = "https://www.bankofcanada.ca/valet/observations/FXUSDCAD/json?start_date=$start&end_date=$end"
            val data = httpJSON("GET", url, emptyMap(), null, 45_000, throwOnAuth = false)
            val obs = arr(data.opt("observations"))
            for (i in 0 until obs.length()) {
                val d = dict(obs.opt(i))
                val day = str(d, "d")
                val v = num(dict(d.opt("FXUSDCAD")).opt("v"), Double.NaN)
                if (day.length == 10 && v > 0) map[day] = v
            }
        } catch (e: Exception) {
            // live failed; keep the cache (rateOn falls back to 1.35)
        }
        return map
    }
}
