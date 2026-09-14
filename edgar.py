"""SEC EDGAR — US regulatory filings, one issuer at a time.

EDGAR is the US Securities and Exchange Commission's filing system. Unlike SEDAR+
it publishes a documented JSON interface with no bot gate and no key; it asks only
for a descriptive User-Agent. This module resolves a ticker to its SEC CIK, reads
the issuer's recent filings from the submissions API, and normalizes them into the
shared disclosure shape used across sources. Standard library only, so US filings
are available even where SEDAR+'s optional dependency is not installed.

One item is:
    {id, source, category, date, dateText, type, title, size, url}
with `id` prefixed "sec:" so the pipeline can route a download back here. EDGAR
documents are static URLs, so downloading needs no session — the stored url is
fetched directly.
"""
from __future__ import annotations

import gzip
import re
import json
import os
import ssl
import threading
import time
from urllib.error import HTTPError, URLError
from urllib.request import Request, urlopen

import disclosures as D

SOURCE = "SEC"
TICKERS_URL = "https://www.sec.gov/files/company_tickers.json"
SUBMISSIONS_URL = "https://data.sec.gov/submissions/CIK%s.json"
ARCHIVE_URL = "https://www.sec.gov/Archives/edgar/data/%d/%s/%s"
# SEC's fair-access policy asks for a User-Agent that identifies the caller with a
# contact address; requests without one are refused (403). Set BAGHOLDER_SEC_UA to
# your own contact. A UA carrying a bare URL is itself refused, so the default is
# name + version + contact only.
UA = os.environ.get("BAGHOLDER_SEC_UA", "Bagholder/1.0 (filings admin@bagholder.app)")
TIMEOUT = 30
PACE_SECONDS = 0.3            # SEC allows up to 10 req/s; stay well under
US_EXCHANGES = {"NASDAQ", "NYSE", "NYSEARCA", "NYSEAMERICAN", "AMEX", "ARCA", "BATS", "US", "OTC", "OTCMKTS", "CBOE"}


def available():
    """EDGAR needs nothing beyond the standard library."""
    return True


# --------------------------------------------------------------------------- #
# HTTP (stdlib, its own CA lookup so it does not depend on curl_cffi)
# --------------------------------------------------------------------------- #
_ctx = None
_lock = threading.Lock()
_last = 0.0


def _context():
    global _ctx
    if _ctx is not None:
        return _ctx
    cafiles = []
    try:
        import certifi
        cafiles.append(certifi.where())
    except Exception:
        pass
    cafiles += ["/etc/ssl/cert.pem", "/etc/ssl/certs/ca-certificates.crt",
                "/opt/homebrew/etc/openssl@3/cert.pem", "/usr/local/etc/openssl@3/cert.pem"]
    for ca in cafiles:
        try:
            _ctx = ssl.create_default_context(cafile=ca)
            return _ctx
        except Exception:
            continue
    _ctx = ssl.create_default_context()
    return _ctx


def _pace():
    global _last
    wait = _last + PACE_SECONDS - time.time()
    if wait > 0:
        time.sleep(wait)
    _last = time.time()


def _get(url):
    _pace()
    req = Request(url, headers={"User-Agent": UA, "Accept-Encoding": "gzip, deflate", "Accept": "application/json"})
    try:
        r = urlopen(req, timeout=TIMEOUT, context=_context())
        raw = r.read()
        if (r.headers.get("Content-Encoding") or "").lower() == "gzip":
            raw = gzip.decompress(raw)
        return raw
    except (HTTPError, URLError, OSError) as e:
        raise D.SourceUnavailable("EDGAR request failed: %s" % e)


def _get_json(url):
    try:
        return json.loads(_get(url).decode("utf-8", "replace"))
    except ValueError as e:
        raise D.SourceUnavailable("EDGAR returned unreadable data: %s" % e)


# --------------------------------------------------------------------------- #
# Ticker → CIK
# --------------------------------------------------------------------------- #
_tickers = None


def _ticker_map():
    """{TICKER: (cik_int, title)} from SEC's published list, loaded once."""
    global _tickers
    if _tickers is None:
        with _lock:
            if _tickers is None:
                data = _get_json(TICKERS_URL)
                _tickers = {}
                for row in (data.values() if isinstance(data, dict) else data):
                    t = str(row.get("ticker", "")).upper()
                    if t:
                        _tickers[t] = (int(row["cik_str"]), row.get("title", ""))
    return _tickers


def _bare(symbol):
    """A ticker as SEC writes it: no venue suffix, dots to dashes (BRK.B -> BRK-B)."""
    s = (symbol or "").strip().upper()
    for suf in (".TO", ".V", ".CN", ".NE", ".U"):
        if s.endswith(suf):
            s = s[: -len(suf)]
    return s.replace(".", "-")


def covers(symbol, exchange="", currency=""):
    """Whether EDGAR should be consulted for this instrument. True for a US listing,
    or any ticker SEC knows (a cross-listed issuer like Shopify); the name guard in
    fetch() rejects a ticker that collides with an unrelated US filer."""
    if (exchange or "").upper() in US_EXCHANGES or (currency or "").upper() == "USD":
        return True
    return _safe_in_map(_bare(symbol))


def _safe_in_map(ticker):
    try:
        return ticker in _ticker_map()
    except D.SourceUnavailable:
        return False


# --------------------------------------------------------------------------- #
# Categories and titles from the SEC form
# --------------------------------------------------------------------------- #
_TITLES = {
    "10-K": "Annual report", "10-Q": "Quarterly report", "8-K": "Current report",
    "20-F": "Annual report (foreign issuer)", "40-F": "Annual report (Canadian issuer)",
    "6-K": "Report of foreign private issuer", "DEF 14A": "Proxy statement", "DEFA14A": "Proxy soliciting material",
    "S-1": "Registration statement", "F-1": "Registration statement", "424B4": "Prospectus",
    "3": "Initial insider ownership", "4": "Insider transaction", "5": "Annual insider statement",
    "144": "Notice of proposed sale", "SC 13D": "Beneficial ownership (activist)",
    "SC 13G": "Beneficial ownership (passive)", "13F-HR": "Institutional holdings",
    "25": "Delisting notice", "425": "Business combination",
}


def _category(form):
    f = (form or "").upper()
    if f.startswith(("10-K", "10-Q", "20-F", "40-F", "6-K", "ARS", "N-CSR")):
        return D.FINANCIALS
    if f.startswith("8-K"):
        return D.EVENTS
    if "14A" in f or "14C" in f or f.startswith("DEF") or f.startswith("PRE"):
        return D.GOVERNANCE
    if f.startswith(("S-", "F-", "424", "POS", "DRS", "EFFECT", "425", "25")):
        return D.OFFERINGS
    if f in {"3", "4", "5", "3/A", "4/A", "5/A", "144"} or "13D" in f or "13G" in f or f.startswith(("SC 13", "SCHEDULE 13", "13F")):
        return D.INSIDER
    return D.OTHER


def _title(form, description):
    """The plain-English title beside the form code. EDGAR often repeats the form
    as the description ("FORM 4" for a 4); in that case use our own label so the
    cell reads "4 · Insider transaction", not "4 · FORM 4"."""
    d = D.clean(description)
    f = (form or "").upper()
    if d and d.upper() not in (f, "FORM " + f):
        return d
    return _TITLES.get(f, "")


# --------------------------------------------------------------------------- #
# Fetch
# --------------------------------------------------------------------------- #
def fetch(symbol, name="", exchange="", currency="", limit=200):
    """The issuer's recent EDGAR filings as normalized items, newest first, or []
    when SEC does not know the ticker (or a name-guard rejects a collision)."""
    ticker = _bare(symbol)
    cik_title = _ticker_map().get(ticker)
    if not cik_title:
        return []
    cik, sec_title = cik_title
    us_listed = (exchange or "").upper() in US_EXCHANGES or (currency or "").upper() == "USD"
    if not us_listed and name and not D.names_match(name, sec_title):
        return []                       # a Canadian ticker colliding with a US filer
    sub = _get_json(SUBMISSIONS_URL % str(cik).zfill(10))
    recent = ((sub.get("filings") or {}).get("recent")) or {}
    forms = recent.get("form") or []
    dates = recent.get("filingDate") or []
    docs = recent.get("primaryDocument") or []
    accns = recent.get("accessionNumber") or []
    descs = recent.get("primaryDocDescription") or [""] * len(forms)
    items = []
    for i in range(min(len(forms), len(dates), len(accns))):
        acc = accns[i]
        doc = docs[i] if i < len(docs) else ""
        url = ARCHIVE_URL % (cik, acc.replace("-", ""), doc) if doc else \
            "https://www.sec.gov/cgi-bin/browse-edgar?action=getcompany&CIK=%d" % cik
        items.append({
            "id": "sec:" + acc,
            "source": SOURCE,
            "category": _category(forms[i]),
            "date": dates[i],
            "dateText": dates[i],
            "type": forms[i],
            "title": _title(forms[i], descs[i] if i < len(descs) else ""),
            "size": "",
            "url": url,
        })
    return items[: max(1, int(limit))]


def has_filer(symbol, name="", exchange="", currency=""):
    """Whether SEC knows a filer for this instrument, cheaply (ticker map only),
    so the pipeline can tell 'nothing filed in range' from 'no filer at all' even
    when fetch returns no rows."""
    ticker = _bare(symbol)
    cik_title = None
    try:
        cik_title = _ticker_map().get(ticker)
    except D.SourceUnavailable:
        return False
    if not cik_title:
        return False
    us_listed = (exchange or "").upper() in US_EXCHANGES or (currency or "").upper() == "USD"
    if not us_listed and name and not D.names_match(name, cik_title[1]):
        return False
    return True


_SKIP_DOC = re.compile(r"(?:-index|-index-headers)\.(?:htm|html)$|^\d{10}-\d\d-\d{6}\.txt$|R\d+\.htm$", re.I)


def content(row):
    """(bytes, content_type) of the filing's *substance* — the largest real content
    document in the accession (the MD&A, press release, or data file), not the cover
    form, the index, or the full-submission dump. Many forms (a 6-K, an 8-K) carry
    only boilerplate on the primary document and the actual filing in exhibits; this
    reads what a person would. Falls back to the primary document when the accession
    cannot be listed or holds nothing better."""
    url = (row or {}).get("url") or ""
    if not url.startswith("https://www.sec.gov/"):
        return document(row)
    base, primary = url.rsplit("/", 1)[0], url.rsplit("/", 1)[-1]
    try:
        items = (_get_json(base + "/index.json").get("directory", {}) or {}).get("item", []) or []
    except Exception:
        return document(row)
    cands = []
    for it in items:
        n = str(it.get("name", ""))
        low = n.lower()
        if not low.endswith((".htm", ".html", ".txt", ".xml")):
            continue
        if "index" in low or _SKIP_DOC.search(low):
            continue
        cands.append((n, int(it.get("size") or 0)))
    if not cands:
        return document(row)
    # the largest substantive file is the content; keep the primary as the tiebreak
    cands.sort(key=lambda c: (-c[1], c[0] != primary))
    best = cands[0][0]
    if best == primary:
        return document(row)
    try:
        return document({"url": base + "/" + best})
    except Exception:
        return document(row)


def document(row):
    """Download one EDGAR document. The stored URL is static, so a direct fetch
    works with no session. Returns (bytes, content_type)."""
    url = (row or {}).get("url") or ""
    if not url.startswith("https://www.sec.gov/"):
        raise D.SourceUnavailable("not an SEC document url")
    _pace()
    req = Request(url, headers={"User-Agent": UA, "Accept-Encoding": "gzip, deflate"})
    try:
        r = urlopen(req, timeout=TIMEOUT, context=_context())
        raw = r.read()
        if (r.headers.get("Content-Encoding") or "").lower() == "gzip":
            raw = gzip.decompress(raw)
    except (HTTPError, URLError, OSError) as e:
        raise D.SourceUnavailable("EDGAR document fetch failed: %s" % e)
    ct = "application/octet-stream"
    try:
        ct = r.headers.get_content_type()
    except Exception:
        pass
    return raw, ct
