"""Explicit, persistent statement evidence for manual in-kind transfers.

Rules supply quantities, never costs. The derived view moves existing lots;
the stored broker feed stays intact. No rule is bundled with the application.
"""
from datetime import date
from decimal import Decimal, InvalidOperation
import math

from ws_reconcile import movement

META_KEY = "statement_transfer_corrections_v1"
SOURCE = "statement-confirmed"


def validate(rules):
    """Return a canonical, JSON-safe ruleset or reject it in its entirety."""
    if not isinstance(rules, list):
        raise ValueError("Transfer corrections must be a list")
    result, seen = [], set()
    for rule in rules:
        if not isinstance(rule, dict):
            raise ValueError("Invalid transfer correction")
        keys = {"event", "date", "sourceAccount", "destinationAccount", "currency", "evidence", "assets"}
        if set(rule) != keys:
            raise ValueError("Correction needs event, date, accounts, currency, evidence and assets only")
        for key in keys - {"assets"}:
            if not isinstance(rule[key], str) or not rule[key].strip():
                raise ValueError("Correction fields must be nonempty text")
        clean = {k: rule[k].strip() for k in keys - {"assets"}}
        event = clean["event"]
        if not event.startswith("asset_movement_request_group-") or event in seen:
            raise ValueError("Correction must identify a unique manual transfer")
        seen.add(event)
        if date.fromisoformat(clean["date"]).isoformat() != clean["date"]:
            raise ValueError("Correction date must be YYYY-MM-DD")
        if clean["sourceAccount"] == clean["destinationAccount"] or clean["currency"] not in ("CAD", "USD"):
            raise ValueError("Correction needs different accounts and CAD or USD currency")
        if not isinstance(rule["assets"], list) or not rule["assets"]:
            raise ValueError("Correction needs a complete list of transferred securities")
        assets, sids, symbols = [], set(), set()
        for asset in rule["assets"]:
            if not isinstance(asset, dict) or set(asset) != {"symbol", "securityId", "quantity"}:
                raise ValueError("Assets accept symbol, securityId and quantity only; no assumed cost")
            if any(not isinstance(asset[k], str) or not asset[k].strip() for k in ("symbol", "securityId")):
                raise ValueError("Assets need a symbol and broker security ID")
            symbol, sid = asset["symbol"].strip(), asset["securityId"].strip()
            try:
                quantity = Decimal(str(asset["quantity"]))
                valid = quantity.is_finite() and quantity > 0 and math.isfinite(float(quantity)) and float(quantity) > 0
            except (InvalidOperation, ValueError, OverflowError):
                valid = False
            if not valid or sid in sids or symbol in symbols:
                raise ValueError("Transfer quantities must be positive, finite and unique by security")
            sids.add(sid)
            symbols.add(symbol)
            assets.append(dict(symbol=symbol, securityId=sid, quantity=str(quantity.normalize())))
        clean["assets"] = sorted(assets, key=lambda a: a["securityId"])
        result.append(clean)
    return sorted(result, key=lambda r: (r["date"], r["event"]))


def apply(snapshot, rules):
    """Overlay exact broker events, without mutating the snapshot or its rows.

    Reapplying to an effective view is safe, including when removing a rule.
    Missing/changed broker parents and existing broker legs never get patched.
    """
    rows = []
    for row in snapshot.get("activities", []):
        if row.get("source") == SOURCE:
            continue
        row = dict(row)
        original = row.pop("statementOriginal", None)
        if original is not None:
            row.update(original)
            row.pop("statementCorrection", None)
        rows.append(row)
    audit = []
    for rule in validate(rules):
        event = rule["event"]
        entry = {k: rule[k] for k in ("event", "date", "evidence", "assets", "sourceAccount", "destinationAccount")}
        entry.update(status="pending", reason="Waiting for both broker transfer records after sync")
        audit.append(entry)
        ids = [event + "-" + rule[k] for k in ("sourceAccount", "destinationAccount")]
        parents = [[r for r in rows if r.get("canonicalId") == cid] for cid in ids]
        if not any(parents):
            continue
        if any(len(p) != 1 for p in parents):
            entry.update(status="blocked", reason="Expected exactly one source and one destination broker record")
            continue
        src, dst = [p[0] for p in parents]
        children = [r for r in rows if r.get("source") == "wealthsimple" and (
            r.get("aftType") in ["WS_DETAIL:" + cid for cid in ids]
            or (r.get("rawType") == "WS_DETAIL_INVENTORY" and r.get("activitySubType") == event))]
        if children:
            entry.update(status="broker-details", reason="Broker movement details are present; saved correction not applied")
            continue
        valid = all(
            p.get("source") == "wealthsimple" and p.get("rawType") == "ASSET_MOVEMENT"
            and p.get("accountId") == aid and p.get("activitySubType") == side
            and p.get("transactionDate") == rule["date"] and p.get("currency") == rule["currency"]
            and not p.get("quantity") and not p.get("symbol") and not p.get("securityId")
            for p, aid, side in ((src, rule["sourceAccount"], "SOURCE"), (dst, rule["destinationAccount"], "DESTINATION")))
        if not valid:
            entry.update(status="blocked", reason="Broker event no longer matches the statement correction; review required")
            continue
        # Both legs use the source's timestamp so receipt cannot precede delivery.
        for asset in rule["assets"]:
            for parent, sign in ((src, -1), (dst, 1)):
                identity = "statement:" + event + ":" + parent["accountId"] + ":" + asset["securityId"]
                row = movement(src, identity, {"id": parent["accountId"], "nickname": parent.get("accountType")},
                    asset["symbol"], sign * Decimal(asset["quantity"]), rule["currency"], event,
                    "InternalSecurityTransfer", sid=asset["securityId"])
                row.update(id=identity, source=SOURCE, fifoId=parent.get("fifoId") or parent["accountId"],
                    statementCorrection=event, evidence=rule["evidence"],
                    description="Statement-confirmed in-kind transfer: " + asset["symbol"])
                rows.append(row)
        for parent in (src, dst):
            parent["statementOriginal"] = {k: parent.get(k) for k in ("rawType", "activityType", "category", "netCashAmount")}
            parent.update(rawType="WS_DETAIL_TRANSFER", activityType="Transfer", category="transfer",
                          netCashAmount=0.0, statementCorrection=event)
        entry.update(status="applied", reason="Share quantities confirmed by statement; original lots and costs carried between accounts")
    return dict(snapshot, activities=rows, statementCorrections=audit)
