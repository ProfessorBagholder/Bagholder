"""Reconcile complete account/date export windows without deleting sync data."""
import hashlib
import json
from collections import Counter, defaultdict


META_KEY = "activity_export_windows"


def prepare(rows, snapshot, account_map=None):
    """Resolve custodian IDs using balances, then unambiguous trade evidence."""
    mapping = dict(account_map or {})
    accounts = {a["id"]: a for a in snapshot.get("accounts", [])}
    for b in snapshot.get("balances", []):
        if b.get("custodianAccountId"):
            mapping[b["custodianAccountId"]] = b["accountId"]
    existing = snapshot.get("activities", [])
    security_ids = defaultdict(set)
    for sec in snapshot.get("securities", []):
        security_ids[(sec.get("symbol"), sec.get("currency"))].add(sec["id"])
    for a in existing:
        if a.get("securityId") and a.get("symbol"):
            security_ids[(a["symbol"], a.get("currency"))].add(a["securityId"])
    def signature(a):
        return (a.get("transactionDate"), a.get("symbol"), a.get("currency"),
                round(abs(a.get("quantity") or 0), 6), round(a.get("netCashAmount") or 0, 2))
    index = defaultdict(set)
    for a in existing:
        if a.get("category") == "trade" and a.get("symbol"):
            index[signature(a)].add(a["accountId"])
    grouped = defaultdict(list)
    for a in rows:
        grouped[a["accountId"]].append(a)
    windows = []
    output = []
    for custodian, group in grouped.items():
        if custodian not in mapping:
            votes = Counter()
            for a in group:
                candidates = index.get(signature(a), set())
                if a.get("category") == "trade" and len(candidates) == 1:
                    votes.update(candidates)
            ranked = votes.most_common()
            if ranked and ranked[0][1] >= 3 and (len(ranked) == 1 or ranked[0][1] > ranked[1][1] * 2):
                mapping[custodian] = ranked[0][0]
            elif accounts:
                raise ValueError("Cannot unambiguously map export account " + custodian + "; provide accountMap before reconciliation")
            else:
                mapping[custodian] = custodian
        aid = mapping[custodian]
        acc = accounts.get(aid, {})
        name = acc.get("nickname") or acc.get("unifiedAccountType") or group[0]["accountType"]
        seen = Counter()
        for raw in group:
            a = dict(raw, accountId=aid, bookId=aid, fifoId=aid, accountType=name)
            known = security_ids.get((a["symbol"], a["currency"]), set())
            if len(known) == 1:
                a["securityId"] = next(iter(known))
            content = {k: v for k, v in a.items() if k != "id"}
            digest = hashlib.sha256(json.dumps(content, sort_keys=True).encode()).hexdigest()
            seen[digest] += 1
            a["id"] = "export:" + digest + ":" + str(seen[digest])
            output.append(a)
        windows.append(dict(accountId=aid, custodianAccountId=custodian,
            first=min(a["transactionDate"] for a in group), last=max(a["transactionDate"] for a in group)))
    return output, windows


def select(activities, windows):
    """A reconciled window replaces feed rows only for that account and period."""
    spans = defaultdict(list)
    for w in windows:
        spans[w["accountId"]].append((w["first"], w["last"]))
    return [a for a in activities if a.get("source") == "ws-export" or not any(
        first <= a.get("transactionDate", "") <= last
        for first, last in spans.get(a.get("accountId"), []))]
