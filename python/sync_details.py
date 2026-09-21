"""Read-only WS order-detail enrichment. CSVs are never inputs to web sync."""
from datetime import datetime
from decimal import Decimal, InvalidOperation
import hashlib
import json

VERSION = "1"
CACHE_KEY = "ws_order_details_v1"
VERSION_KEY = "ws_order_details_version"

MULTILEG_QUERY = """query FetchSoOrdersMultilegOrder($branchId: String!, $orderBatchId: String!) {
 soOrdersMultilegOrder(branchId: $branchId, orderBatchId: $orderBatchId) {
  orderBatchId status totalFee orderCurrency securityCurrency filledExchangeRateWithSpread
  legs { orderId securityId symbol side openClose status filledQuantity
   averageFillPrice { amount currency } filledNetValue orderCurrency firstFilledAtUtc lastFilledAtUtc }
 }
}"""
CRYPTO_QUERY = """query FetchCryptoOrder($id: ID!) {
 cryptoOrder(id: $id) { id status filledAt currency executedQuantity executedValue swapFee fee totalCost }
}"""
SECURITIES_QUERY = """query DetailSecurities($ids: [ID!]!) {
 securities(ids: $ids) { id currency stock { symbol name }
  optionDetails { multiplier optionType strikePrice expiryDate underlyingSecurity { id stock { symbol } } }
 }
}"""


def number(value):
    if value is None or value == "":
        raise ValueError("Missing execution amount")
    try:
        result = Decimal(str(value))
    except InvalidOperation as exc:
        raise ValueError("Invalid execution amount") from exc
    if not result.is_finite():
        raise ValueError("Non-finite execution amount")
    return result


def detail_kind(item):
    typ = str(item.get("type") or "").upper()
    if "MULTILEG" in typ:
        return "option"
    if typ in ("CRYPTO_BUY", "CRYPTO_SELL") and "SWAP" in str(item.get("subType") or "").upper():
        return "crypto"
    return None


def contract(security):
    option = security.get("optionDetails") or {}
    underlying = ((option.get("underlyingSecurity") or {}).get("stock") or {}).get("symbol")
    symbol = underlying or (security.get("stock") or {}).get("symbol")
    right = str(option.get("optionType") or "").upper()
    if not symbol or right not in ("PUT", "CALL") or number(option.get("multiplier")) != 100:
        raise ValueError("Missing or unsupported option identity")
    expiry = datetime.strptime(option["expiryDate"][:10], "%Y-%m-%d").strftime("%d%b%y").upper()
    return f"{symbol} {expiry} {number(option['strikePrice']):.2f} {right}"


def child(parent, ident, symbol, qty, price, cash, fee, currency, when, kind, sub, security_id=""):
    row = dict(parent)
    row.update(canonicalId=ident, symbol=symbol, name=symbol, quantity=float(qty),
        unitPrice=float(price), netCashAmount=float(cash), commission=float(fee),
        currency=currency.upper(), occurredAt=when, transactionDate=when[:10], settlementDate=when[:10],
        category="trade", activityType="Trade", activitySubType=sub,
        direction="DEBIT" if cash < 0 else "CREDIT", rawType="WS_DETAIL_" + kind.upper(),
        aftType="WS_DETAIL:" + parent["canonicalId"], counterSymbol="", securityId=security_id)
    row.pop("id", None)
    row["description"] = f"WS execution detail: {sub} {abs(qty)} {symbol}"
    return row


def option_legs(item, parent, detail, securities):
    if detail.get("orderBatchId") != item.get("externalCanonicalId"):
        raise ValueError("Option order identity mismatch")
    if str(detail.get("status") or "").lower() not in ("posted", "filled"):
        raise ValueError("Option order is not settled")
    output = []
    actual_fees = Decimal(0)
    for leg in detail.get("legs") or []:
        if str(leg.get("status") or "").lower() not in ("posted", "filled"):
            raise ValueError("Option leg is not settled")
        side, intent = leg.get("side"), leg.get("openClose")
        if side not in ("BUY", "SELL") or intent not in ("OPEN", "CLOSE"):
            raise ValueError("Missing option execution direction")
        qty = number(leg.get("filledQuantity"));px = leg.get("averageFillPrice") or {}
        price = number(px.get("amount"));net = abs(number(leg.get("filledNetValue")))
        currency = str(px.get("currency") or "").upper()
        if qty <= 0 or price < 0 or currency not in ("CAD", "USD") or currency != str(leg.get("orderCurrency") or "").upper():
            raise ValueError("Unsupported option execution units or currency")
        gross = qty * price * 100
        fee = net - gross if side == "BUY" else gross - net
        if fee < Decimal("-.02"):
            raise ValueError("Option cash does not reconcile to its fill")
        fee = max(Decimal(0), fee);actual_fees += fee
        sid = leg.get("securityId");symbol = contract(securities.get(sid) or {})
        ident = leg.get("orderId");when = leg.get("lastFilledAtUtc") or leg.get("firstFilledAtUtc")
        if not ident or not when:
            raise ValueError("Missing option execution identity or time")
        output.append(child(parent, ident, symbol, qty if side == "BUY" else -qty, price,
            -net if side == "BUY" else net, fee, currency, when, "option", side + "TO" + intent, sid))
    if not output or len({r['canonicalId'] for r in output}) != len(output):
        raise ValueError("Missing or duplicate option legs")
    total_fee = number(detail.get("totalFee"))
    if abs(actual_fees - total_fee) > Decimal(".02"):
        raise ValueError("Option fees require additional detail")
    return output


def crypto_legs(item, parent, detail):
    if detail.get("id") != item.get("externalCanonicalId"):
        raise ValueError("Crypto order identity mismatch")
    if str(detail.get("status") or "").lower() not in ("posted", "filled"):
        raise ValueError("Crypto swap is not settled")
    outgoing = str(item.get("assetSymbol") or "").removeprefix("EXCHANGE:").upper()
    incoming = str(item.get("counterAssetSymbol") or "").removeprefix("EXCHANGE:").upper()
    if not outgoing or not incoming or outgoing == incoming:
        raise ValueError("Missing crypto swap assets")
    units = number(detail.get("executedQuantity"));value = number(detail.get("executedValue"))
    coin_fee = number(detail.get("swapFee"));fee = number(detail.get("fee"))
    total = number(detail.get("totalCost"))
    # BUY describes buying the counter-coin; the feed still labels the coin
    # being spent. WS's swap fee is always in the outgoing coin, not cash.
    buy = item["type"] == "CRYPTO_BUY"
    out_qty = (value if buy else units) + coin_fee
    in_qty = units if buy else value
    net = total - fee if buy else total
    currency = str(detail.get("currency") or "").upper()
    when = detail.get("filledAt")
    if min(out_qty, in_qty, net) <= 0 or min(coin_fee, fee) < 0 or currency not in ("CAD", "USD") or not when:
        raise ValueError("Missing crypto swap execution amounts")
    cid = parent["canonicalId"]
    return [child(parent, cid + ":swap:out", outgoing, -out_qty, (net + fee) / out_qty,
                  net, fee, currency, when, "crypto", "SELL", parent.get("securityId", "")),
            child(parent, cid + ":swap:in", incoming, in_qty, net / in_qty,
                  -net, Decimal(0), currency, when, "crypto", "BUY")]


def enrich(items, accounts, mapper, query, cache, progress=None):
    """Return ordinary rows and atomic parent/leg groups; cache successful details only."""
    ordinary, groups, pending, resolved = [], [], [], []
    securities = cache.setdefault("securities", {})
    orders = cache.setdefault("orders", {})
    for item in items:
        rows = mapper(item, accounts)
        if not rows:
            continue
        kind = detail_kind(item)
        if not kind:
            ordinary.extend(rows)
            continue
        parent = rows[0]
        external = item.get("externalCanonicalId")
        signature = hashlib.sha256(json.dumps(item, sort_keys=True).encode()).hexdigest()
        prior = orders.get(parent["canonicalId"], {})
        detail = None
        try:
            if not external:
                raise ValueError("Missing external order identifier")
            if prior.get("signature") == signature:
                detail = prior.get("detail")
            else:
                if progress:
                    progress("Reading WS execution details…")
                if kind == "option":
                    detail = query("FetchSoOrdersMultilegOrder", {"branchId": "TR", "orderBatchId": external}, MULTILEG_QUERY).get("soOrdersMultilegOrder")
                else:
                    detail = query("FetchCryptoOrder", {"id": external}, CRYPTO_QUERY).get("cryptoOrder")
            if not isinstance(detail, dict):
                raise ValueError("WS did not return execution details")
            resolved.append((item, parent, kind, signature, detail))
        except PermissionError:
            raise
        except Exception:
            pending.append((parent, kind))
    needed = sorted({leg.get("securityId") for _, _, kind, _, detail in resolved if kind == "option"
                     for leg in detail.get("legs", []) if leg.get("securityId") and leg["securityId"] not in securities})
    for start in range(0, len(needed), 50):
        try:
            data = query("DetailSecurities", {"ids": needed[start:start+50]}, SECURITIES_QUERY)
            for security in data.get("securities") or []:
                if security and security.get("id"):
                    securities[security["id"]] = security
        except PermissionError:
            raise
        except Exception:
            pass
    for item, parent, kind, signature, detail in resolved:
        try:
            legs = option_legs(item, parent, detail, securities) if kind == "option" else crypto_legs(item, parent, detail)
            orders[parent["canonicalId"]] = dict(signature=signature, activity=item, detail=detail)
            original_type = parent["rawType"]
            parent.update(category="other", activityType="Other", rawType="WS_DETAIL_PARENT", aftType=original_type)
            groups.append((parent, legs))
        except (ValueError, KeyError, TypeError):
            pending.append((parent, kind))
    for parent, kind in pending:
        parent.update(category="other", activityType="Other", rawType="WS_DETAIL_MISSING_" + kind.upper())
        # A revised order with unavailable details must also withdraw any old
        # children atomically. Never retain half of a revised execution.
        groups.append((parent, []))
    return ordinary, groups, len(pending)
