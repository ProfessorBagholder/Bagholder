"""The disclosures pipeline: one instrument's regulatory filings gathered from every
source that covers it, normalized to a single shape, merged newest first.

A "source" is a regulator's filing system (SEDAR+ for Canada, SEC EDGAR for the US,
more as markets are added); a "category" is the kind of disclosure, the same across
sources. Each provider module exposes:

    SOURCE                       the source label, e.g. "SEDAR+"
    available()                  whether the source can be reached at all
    covers(symbol, exchange, currency)   whether it applies to this instrument
    fetch(symbol, name, exchange, currency, limit=…)   -> [item, …]
    document(row)                -> (bytes, content_type)

An item is a plain dict:
    {id, source, category, date, dateText, type, title, size, url}
`id` is prefixed with the source ("sedar:…", "sec:…") so a stored row routes back
to the provider that can download it. This module owns the shared vocabulary and
the merge; the providers own the per-site fetching.
"""
from __future__ import annotations

import re

# Categories — the labels the UI shows, one vocabulary across sources.
FINANCIALS = "Financials"
EVENTS = "Material events"
GOVERNANCE = "Governance"
OFFERINGS = "Offerings"
INSIDER = "Insider & ownership"
NEWS = "News releases"
OTHER = "Other"
CATEGORIES = [FINANCIALS, EVENTS, GOVERNANCE, OFFERINGS, INSIDER, NEWS, OTHER]


class SourceUnavailable(Exception):
    """A source could not be reached, or its optional dependency is not installed."""


_WS = re.compile(r"\s+")
_TAGS = re.compile(r"<[^>]+>")


def clean(text):
    return _WS.sub(" ", _TAGS.sub(" ", text or "")).strip()


# Corporate suffixes and filler dropped before comparing two issuer names.
_NAME_NOISE = re.compile(
    r"\b(inc|corp|corporation|ltd|limited|co|company|plc|the|sa|nv|ag|llc|lp|trust|fund|holdings?)\b",
    re.I,
)


def _name_tokens(name):
    n = _NAME_NOISE.sub(" ", (name or "").lower())
    return {t for t in re.split(r"[^a-z0-9]+", n) if len(t) > 1}


def names_match(a, b):
    """Whether two issuer names plausibly denote the same company, used to reject a
    ticker that collides with an unrelated filer in another market."""
    ta, tb = _name_tokens(a), _name_tokens(b)
    if not ta or not tb:
        return False
    overlap = ta & tb
    return bool(overlap) and len(overlap) >= min(len(ta), len(tb)) * 0.5


def _sort_key(item):
    return (item.get("date") or item.get("dateText") or "", item.get("source") or "")


# Providers are imported at the end, after the vocabulary above is defined, so the
# provider modules can `import disclosures` without a partial-module problem.
import edgar          # noqa: E402
import sedar          # noqa: E402

PROVIDERS = [sedar, edgar]


def available():
    """True when at least one source can be reached."""
    return any(p.available() for p in PROVIDERS)


def providers_for(symbol, exchange="", currency=""):
    out = []
    for p in PROVIDERS:
        try:
            if p.available() and p.covers(symbol, exchange, currency):
                out.append(p)
        except Exception:
            continue
    return out


def fetch(symbol, name="", exchange="", currency="", limit=200):
    """Every covering source's filings for one instrument, merged newest first.

    Returns {"items": [...], "sources": {SOURCE: {available, matched, count, error}}}.
    A source that fails is recorded and skipped; the others still return."""
    items, sources = [], {}
    for p in PROVIDERS:
        covered = False
        try:
            covered = p.available() and p.covers(symbol, exchange, currency)
        except Exception:
            covered = False
        if not covered:
            sources[p.SOURCE] = {"available": _safe_available(p), "matched": False, "filer": False, "count": 0, "error": ""}
            continue
        try:
            got = p.fetch(symbol, name=name, exchange=exchange, currency=currency) or []
            items.extend(got)
            filer = bool(got)
            if not filer and hasattr(p, "has_filer"):
                try:
                    filer = bool(p.has_filer(symbol, name=name, exchange=exchange, currency=currency))
                except Exception:
                    filer = False
            sources[p.SOURCE] = {"available": True, "matched": bool(got), "filer": filer, "count": len(got), "error": ""}
        except SourceUnavailable as e:
            sources[p.SOURCE] = {"available": False, "matched": False, "filer": False, "count": 0, "error": str(e)}
        except Exception as e:
            sources[p.SOURCE] = {"available": True, "matched": False, "filer": False, "count": 0, "error": "%s: %s" % (type(e).__name__, e)}
    items.sort(key=_sort_key, reverse=True)
    return {"items": items[: max(1, int(limit))], "sources": sources}


def _safe_available(p):
    try:
        return bool(p.available())
    except Exception:
        return False


def document(row):
    """Download the document a stored row points at, routed to its source's provider.
    Returns (bytes, content_type). Raises SourceUnavailable on failure."""
    src = (row or {}).get("source") or ""
    for p in PROVIDERS:
        if p.SOURCE == src:
            return p.document(row)
    raise SourceUnavailable("no provider for source %r" % src)
