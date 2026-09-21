"""Broker reporting and explicit inventory events from WS's read-only web APIs.

Broker adjusted returns and journal FIFO results deliberately remain distinct.
No CSV, guessed split ratio, or transfer market valuation supplies cost basis.
"""
from concurrent.futures import ThreadPoolExecutor
from datetime import datetime, timezone
from decimal import Decimal
import json

from sync_details import SECURITIES_QUERY, contract, number

VERSION = "2"
CACHE_KEY = "ws_inventory_details_v1"
VERSION_KEY = "ws_inventory_details_version"
REPORT_KEY = "ws_realized_report_v1"

TRANSFER_QUERY = """query FetchFundingIntentStatusSummary($fundingIntentId: ID!) {
 fundingIntentStatusSummary: funding_intent_status_summary(funding_intent_id:$fundingIntentId) {
  sourceFundingPoint { fundingPointId } destinationFundingPoint { fundingPointId }
  details { ... on FundingIntentStatusSummaryInternalTransferDetails {
   totalAmount { amount currency } transferType: transfer_type
   internalTransferItems { security_id symbol currency quantity }
  } }
 }
}"""
CORPORATE_QUERY = """query FetchCorporateActionChildActivities($activityCanonicalId: String!) {
 corporateActionChildActivities(condition:{activityCanonicalId:$activityCanonicalId}) {
  nodes { canonicalId activityCanonicalId assetName assetSymbol assetType entitlementType quantity currency }
 }
}"""
REPORT_QUERY = """query FetchIdentityRealizedReturns($identityId: ID!, $currency: Currency!, $first: Int, $cursor: String) {
 identity(id:$identityId) { financials(accountScope:OWN) { realizedReturns(currency:$currency) {
  totalValue { amount currency }
  timeRangeBreakdown(resolution:MONTHLY) { month year totalValue { amount currency } }
  securityBreakdown(first:$first,after:$cursor) {
   edges { node { security { id currency securityType stock { symbol name }
    optionDetails { multiplier optionType strikePrice expiryDate underlyingSecurity { id stock { symbol } } }
   } totalValue { amount currency } } }
   pageInfo { hasNextPage endCursor }
  }
 } } }
}"""


def now():
    return datetime.now(timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ")


def target(item):
    if item.get("type") == "CORPORATE_ACTION":
        return "corporate", item["canonicalId"]
    external = item.get("externalCanonicalId") or ""
    if item.get("type") == "INTERNAL_TRANSFER" and external.startswith("funding_intent-"):
        return "transfer", external
    return None


def movement(parent, identity, account, symbol, quantity, currency, event, typ, name="", sid=""):
    row = dict(parent, canonicalId=identity, accountId=account["id"], bookId=account["id"], fifoId=account["id"],
        accountType=account.get("nickname") or account.get("unifiedAccountType") or account["id"],
        symbol=symbol, name=name or symbol, quantity=float(quantity), currency=currency,
        activityType=typ, activitySubType=event, category="other", direction="",
        unitPrice=0.0, netCashAmount=0.0, commission=0.0, rawType="WS_DETAIL_INVENTORY",
        aftType="WS_DETAIL:" + parent["canonicalId"], counterSymbol="", securityId=sid)
    row.pop("id", None)
    row["description"] = f"WS inventory movement: {quantity} {symbol}"
    return row


def symbol_history(items, accounts, mapper, cache):
    """Keep dated feed spellings across incremental reads, keyed by broker ID.

    Funding details use bare stock symbols while executions can use exchange
    suffixes. Only actual stock trades are evidence; option and swap summaries
    can identify a different asset from the quantity they describe.
    """
    saved = cache.setdefault("symbols", {})
    for item in items:
        cid = item.get("canonicalId")
        if not cid:
            continue
        saved.pop(cid, None)
        if item.get("type") not in ("DIY_BUY", "DIY_SELL") or item.get("contractType"):
            continue
        rows = mapper(item, accounts)
        if len(rows) != 1:
            continue
        row = rows[0]
        if row.get("securityId") and row.get("symbol") and row.get("quantity"):
            saved[cid] = [row["securityId"], row["currency"], row["symbol"], row["transactionDate"]]
    history = {}
    for sid, currency, symbol, date in saved.values():
        history.setdefault((sid, currency), {}).setdefault(date, set()).add(symbol)
    return history


def transfer_symbol(symbol, sid, currency, date, history):
    # Do not turn this into a ticker-rename heuristic. The same broker ID,
    # currency, and an exchange-suffix-only difference are all required.
    def bare(value):
        for suffix in (".TO", ".V", ".CN", ".NE"):
            if value.endswith(suffix):
                return value[:-len(suffix)]
        return value
    dates = (history or {}).get((sid, currency), {})
    before = [day for day in dates if day <= date]
    names = dates[max(before)] if before else set()
    if len(names) == 1:
        candidate = next(iter(names))
        if bare(candidate) == bare(symbol):
            return candidate
    return symbol


def transfer_group(items, accounts, mapper, detail, securities, history=None):
    src = (detail.get("sourceFundingPoint") or {}).get("fundingPointId")
    dst = (detail.get("destinationFundingPoint") or {}).get("fundingPointId")
    d = detail.get("details") or {}
    assets = d.get("internalTransferItems") or []
    parents = [p for i in items for p in mapper(i, accounts)]
    if not parents:
        raise ValueError("Transfer summary unavailable")
    if not assets:
        if d.get("transferType") not in ("partial_in_cash", "full_in_cash", "in_cash"):
            raise ValueError("Transfer inventory detail unavailable")
        return None
    if src == dst or src not in accounts or dst not in accounts:
        raise ValueError("Unknown transfer account")
    # One owner for the entire atomic event; source and destination legs are
    # emitted even when only one account's activity feed still has the summary.
    parent = next((p for p in parents if p["accountId"] == src), parents[0])
    event = items[0]["externalCanonicalId"]
    money = d.get("totalAmount") or {}
    cash = number(money.get("amount"))
    if cash < 0:
        raise ValueError("Invalid transfer cash")
    summaries = []
    for p in parents:
        if p["accountId"] not in (src, dst) or (cash and p["currency"] != money.get("currency")):
            raise ValueError("Transfer cash currency mismatch")
        p.update(netCashAmount=float(-cash if p["accountId"] == src else cash),
                 rawType="WS_DETAIL_TRANSFER", aftType="" if p["canonicalId"] == parent["canonicalId"] else "WS_DETAIL:" + parent["canonicalId"])
        if p["canonicalId"] == parent["canonicalId"]:
            parent = p
        else:
            summaries.append(p)
    legs = []
    for a in assets:
        sid = a.get("security_id")
        sec = securities.get(sid) or {}
        symbol = contract(sec) if sec.get("optionDetails") else a.get("symbol") or (sec.get("stock") or {}).get("symbol")
        qty = number(a.get("quantity"));currency = a.get("currency") or sec.get("currency")
        if not sid or not symbol or qty <= 0 or currency not in ("CAD", "USD"):
            raise ValueError("Incomplete transfer asset")
        if not sec.get("optionDetails"):
            symbol = transfer_symbol(symbol, sid, currency, parent["transactionDate"], history)
        for aid, sign in ((src, -1), (dst, 1)):
            legs.append(movement(parent, event + ":" + aid + ":" + sid, accounts[aid], symbol,
                qty * sign, currency, event, "InternalSecurityTransfer", sid=sid))
    if len({r["canonicalId"] for r in legs}) != len(legs):
        raise ValueError("Duplicate transfer asset")
    return parent, summaries + legs


def corporate_group(item, accounts, mapper, nodes):
    mapped = mapper(item, accounts)
    if len(mapped) != 1:
        raise ValueError("Ambiguous corporate-action summary")
    parent = mapped[0]
    before = [n for n in nodes if n.get("entitlementType") in ("SUBMIT", "HOLD")]
    after = [n for n in nodes if n.get("entitlementType") == "RECEIVE"]
    if len(before) != 1 or len(after) != 1 or len(nodes) != 2:
        raise ValueError("Corporate action requires cost allocation")
    src, dst = before[0], after[0]
    if any(n.get("activityCanonicalId") != item["canonicalId"] or n.get("assetType") != "EQUITY" for n in nodes):
        raise ValueError("Corporate action identity mismatch")
    out_qty, in_qty = number(src.get("quantity")), number(dst.get("quantity"))
    if src["entitlementType"] == "HOLD":
        if src.get("assetSymbol") != dst.get("assetSymbol"):
            raise ValueError("Spin-off requires broker cost allocation")
        in_qty += out_qty
    if min(out_qty, in_qty) <= 0 or not src.get("assetSymbol") or not dst.get("assetSymbol"):
        raise ValueError("Invalid corporate action quantities")
    currency = src.get("currency") or dst.get("currency")
    dest_currency = dst.get("currency") or currency
    if currency not in ("CAD", "USD") or dest_currency not in ("CAD", "USD"):
        raise ValueError("Unknown corporate action currency")
    account = accounts.get(parent["accountId"]) or dict(id=parent["accountId"], nickname=parent["accountType"])
    event = item["canonicalId"]
    legs = [movement(parent, event + ":inventory:out", account, src["assetSymbol"], -out_qty,
                     currency, event, "CorporateAction", src.get("assetName")),
            movement(parent, event + ":inventory:in", account, dst["assetSymbol"], in_qty,
                     dest_currency, event, "CorporateAction", dst.get("assetName"))]
    parent.update(rawType="WS_DETAIL_PARENT", category="other", activityType="Other")
    return parent, legs


def enrich(items, accounts, mapper, query, cache, progress=None):
    """Replay cached explicit events on incremental sync; retry unavailable details.

    Cached raw summaries are cleared by clear-data, so a clean run must fetch
    every event again. All fetched data comes from read-only broker queries.
    """
    history = symbol_history(items, accounts, mapper, cache)
    saved = cache.setdefault("items", {})
    for i in items:
        if target(i) and mapper(i, accounts):
            saved[i["canonicalId"]] = i
    batches = {}
    for i in saved.values():
        batches.setdefault(target(i), []).append(i)
    details = cache.setdefault("details", {})
    securities = cache.setdefault("securities", {})
    jobs = []
    for (kind, key), batch in sorted(batches.items()):
        # Completed funding events and posted actions are immutable inventory
        # instructions. A changed summary invalidates its cache entry.
        signature = json.dumps(sorted(batch, key=lambda i:i["canonicalId"]), sort_keys=True)
        prior = details.get(key) or {}
        if prior.get("signature") != signature or not prior.get("data"):
            jobs.append((kind, key, signature))
    def fetch(job):
        kind, key, signature = job
        try:
            if kind == "transfer":
                data = query("FetchFundingIntentStatusSummary", {"fundingIntentId":key}, TRANSFER_QUERY).get("fundingIntentStatusSummary")
            else:
                data = query("FetchCorporateActionChildActivities", {"activityCanonicalId":key}, CORPORATE_QUERY).get("corporateActionChildActivities")
            return key, dict(signature=signature, data=data)
        except PermissionError:
            raise
        except Exception:
            return key, {}
    if jobs and progress:
        progress("Reading WS transfers and corporate actions…")
    with ThreadPoolExecutor(max_workers=3) as pool:
        for key, result in pool.map(fetch, jobs):
            details[key] = result
    ids = sorted({a["security_id"] for (kind,key) in batches if kind == "transfer"
        for a in (((details.get(key) or {}).get("data") or {}).get("details") or {}).get("internalTransferItems") or []
        if a.get("security_id") and a["security_id"] not in securities})
    for offset in range(0, len(ids), 50):
        result = query("DetailSecurities", {"ids":ids[offset:offset+50]}, SECURITIES_QUERY)
        securities.update({s["id"]:s for s in result.get("securities") or [] if s and s.get("id")})
    groups, warnings = [], []
    for (kind,key), batch in sorted(batches.items()):
        data = (details.get(key) or {}).get("data")
        try:
            if not data:
                raise ValueError("Broker detail unavailable")
            group = transfer_group(batch, accounts, mapper, data, securities, history) if kind == "transfer" else corporate_group(batch[0], accounts, mapper, data.get("nodes") or [])
            if group:
                groups.append(group)
        except (ValueError, KeyError, TypeError) as exc:
            affected = sorted({n.get("assetSymbol") for n in (data or {}).get("nodes") or [] if n.get("assetSymbol")})
            warnings.append(dict(event=key, kind=kind, symbol=batch[0].get("assetSymbol") or "",
                symbols=affected, date=(batch[0].get("occurredAt") or "")[:10], reason=str(exc)))
            # A failed/incomplete response must be retried even when the feed
            # summary has not changed. Never retain previously derived legs.
            details.pop(key, None)
            if kind == "corporate":
                for parent in mapper(batch[0], accounts):
                    parent.update(category="other", activityType="Other", rawType="WS_DETAIL_MISSING_INVENTORY")
                    # A spin-off's feed summary can name only the new asset.
                    # Flag the retained security too: its basis allocation is
                    # also unknown, even though its quantity did not change.
                    account = accounts.get(parent["accountId"]) or {"id":parent["accountId"], "nickname":parent["accountType"]}
                    markers = []
                    for symbol in affected:
                        marker = movement(parent, key + ":unresolved:" + symbol, account, symbol, 0,
                            parent["currency"], key, "Other")
                        marker["rawType"] = "WS_DETAIL_MISSING_INVENTORY"
                        markers.append(marker)
                    groups.append((parent, markers))
            else:
                for item in batch:
                    for parent in mapper(item, accounts):
                        parent.update(rawType="WS_DETAIL_MISSING_TRANSFER")
                        groups.append((parent, []))
    return groups, warnings


def fetch_report(identity, query):
    # The reporting service can return an inconsistent page or a transient
    # server error. Restart the whole read once; never mix pages from attempts.
    for attempt in range(2):
        try:
            return _fetch_report_once(identity, query)
        except PermissionError:
            raise
        except (ValueError, KeyError, TypeError, RuntimeError):
            if attempt:
                raise


def _fetch_report_once(identity, query):
    """Require a complete, non-duplicated breakdown matching the broker total."""
    rows, seen, cursors = [], set(), set()
    cursor = None
    total = None
    months = []
    for _ in range(100):
        data = query("FetchIdentityRealizedReturns", dict(identityId=identity, currency="CAD", first=500, cursor=cursor), REPORT_QUERY)
        report = data["identity"]["financials"]["realizedReturns"]
        money = report["totalValue"]
        if money.get("currency") != "CAD":
            raise ValueError("Unexpected broker return currency")
        current = number(money.get("amount"))
        if total is not None and total != current:
            raise ValueError("Broker returns changed during pagination; retry sync")
        total = current
        if cursor is None:
            months = [{"month":f"{int(m['year']):04d}-{int(m['month']):02d}",
                       "pnl":float(number(m['totalValue']['amount']))} for m in report.get("timeRangeBreakdown") or []]
        breakdown = report["securityBreakdown"]
        for e in breakdown["edges"]:
            n = e["node"];sec = n["security"];sid = sec["id"]
            if sid in seen or n["totalValue"].get("currency") != "CAD":
                raise ValueError("Duplicate security or inconsistent report currency")
            seen.add(sid)
            stock = sec.get("stock") or {}
            option = sec.get("optionDetails") or {}
            symbol = contract(sec) if option else stock.get("symbol")
            root = ((option.get("underlyingSecurity") or {}).get("stock") or {}).get("symbol") or symbol
            if not symbol:
                raise ValueError("Missing report security identity")
            rows.append(dict(securityId=sid, symbol=symbol, underlying=root, name=stock.get("name") or symbol,
                             pnl=float(number(n["totalValue"]["amount"]))))
        page = breakdown["pageInfo"]
        if not page.get("hasNextPage"):
            if abs(sum((Decimal(str(r["pnl"])) for r in rows), Decimal(0)) - total) > Decimal(".02"):
                raise ValueError("Incomplete broker return breakdown")
            return dict(status="ok", fetchedAt=now(), currency="CAD", scope="All investing accounts · all time",
                        total=float(total), securities=rows, monthly=months)
        cursor = page.get("endCursor")
        if not cursor or cursor in cursors:
            raise ValueError("Invalid broker pagination")
        cursors.add(cursor)
    raise ValueError("Broker return pagination exceeded limit")
