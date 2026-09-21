"""Dated, explicit custody/corporate-action movements, never global aliases.

Pair only unambiguous event legs. Two unrelated renames on the same day must
not consume each other's lots. DLR is the sole instrument-specific fallback.
"""
from collections import defaultdict


TYPES = {"InternalSecurityTransfer", "SecurityTransfer", "ListingSwap",
         "CorporateAction", "LegacyCorporateAction", "Correction"}


def movements(activities):
    rows = list(activities)
    journals = []
    for a in rows:
        if a.get("category") == "transfer" and a.get("kind") == "Crypto":
            outbound = "OUT" in str(a.get("activitySubType", "")).upper() or a.get("netCashAmount", 0) < 0
            journals.append(dict(a, category="inventory", quantity=(-1 if outbound else 1) * abs(a.get("quantity", 0)), _destination=None))
    for a in rows:
        if a.get("activityType") != "JOURNAL_SHARES" or a.get("symbol") not in ("DLR", "DLR.U") or not a.get("quantity"):
            continue
        source = a["symbol"]
        other = "DLR.U" if source == "DLR" else "DLR"
        src = dict(a, quantity=-abs(a["quantity"]), currency="USD" if source == "DLR.U" else "CAD", category="inventory")
        dst = dict(a, id=a["id"] + ":counter", symbol=other, quantity=abs(a["quantity"]),
                   currency="USD" if other == "DLR.U" else "CAD", securityId="")
        journals.append(dict(src, _destination=dst))
    events = [a for a in rows if (a.get("source") == "ws-export" or a.get("rawType") == "WS_DETAIL_INVENTORY")
              and a.get("activityType") in TYPES and a.get("symbol")
              and a.get("quantity")]
    groups = defaultdict(list)
    for a in events:
        t = a["activityType"]
        # Account transfers may arrive in exports with slightly different times.
        if t == "InternalSecurityTransfer":
            key = (t, a["transactionDate"], a["symbol"], a["currency"],
                   a.get("activitySubType") if a.get("rawType") == "WS_DETAIL_INVENTORY" else "")
        else:
            key = (t, a["accountId"], a["transactionDate"],
                   a.get("occurredAt"), a.get("activitySubType") if t in ("CorporateAction", "LegacyCorporateAction", "Correction") else "")
        groups[key].append(a)
    consumed = set()
    result = []
    for group in groups.values():
        negatives = [a for a in group if a["quantity"] < 0]
        positives = [a for a in group if a["quantity"] > 0]
        def candidates(a, pool):
            return [b for b in pool if b["id"] not in consumed
                    and (a["activityType"] != "InternalSecurityTransfer" or a["accountId"] != b["accountId"])
                    and (abs(abs(a["quantity"])-abs(b["quantity"])) < 1e-8
                         or (len(negatives) == len(positives) == 1))]
        for a in negatives:
            choices = candidates(a, positives)
            if len(choices) != 1 or len(candidates(choices[0], negatives)) != 1:
                continue
            b = choices[0]
            # An internal transfer cannot silently change quantity.
            if a["activityType"] == "InternalSecurityTransfer" and abs(a["quantity"] + b["quantity"]) > 1e-8:
                continue
            consumed.update((a["id"], b["id"]))
            event = dict(a, category="inventory", _destination=b)
            result.append(event)
        for a in group:
            if a["id"] in consumed:
                continue
            if a["activityType"] == "ListingSwap" and a["symbol"] in ("DLR", "DLR.U") and len(group) == 1:
                other = "DLR.U" if a["symbol"] == "DLR" else "DLR"
                b = dict(a, id=a["id"] + ":counter", symbol=other,
                         currency="USD" if other == "DLR.U" else "CAD",
                         quantity=-a["quantity"], securityId="")
                src, dst = (a, b) if a["quantity"] < 0 else (b, a)
                result.append(dict(src, category="inventory", _destination=dst))
            else:
                result.append(dict(a, category="inventory", _destination=None))
    ids = {a["id"] for a in events}
    ids.update(a["id"] for a in journals)
    return [a for a in rows if a["id"] not in ids] + result + journals
