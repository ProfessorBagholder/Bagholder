"""SEDAR+ document filings, fetched over HTTP one issuer at a time.

SEDAR+ (sedarplus.ca) is the Canadian securities filing system; every reporting
issuer's prospectuses, financial statements, MD&A, material change reports and
news releases are filed there. It publishes no API and sits behind Radware Bot
Manager, which turns away ordinary HTTP clients and every headless browser at the
TLS handshake (each is redirected to validate.perfdrive.com). A browser's own TLS
fingerprint is what the gate actually checks, so this module speaks to the site
through `curl_cffi` with `impersonate="chrome"`: it holds one paced session,
resolves an issuer to its nine-digit SEDAR+ profile number, lists that profile's
filings, and downloads a filing as its PDF. Everything is on demand and paced a
couple of seconds apart; nothing sweeps or monitors, which is what the site's
terms draw the line at.

`curl_cffi` is the app's one optional dependency. Where it is not installed
`available()` is False and the callers show nothing rather than failing; the rest
of Bagholder is untouched.

The site is a server-rendered form application (Catalyst): a search page posts
its whole form back to viewInstance/update.html with a callback node, name and the
view key, and answers with HTML fragments. The functions here drive three of its
services — searchReportingIssuers (issuer name/number lookup), searchDocuments
(the filing list and, in each row, a direct document link) — and fetch the
document resource the row links to.
"""
from __future__ import annotations

import html as _html
import re
import sys
import threading
import time
from urllib.parse import urlencode

import disclosures as D

try:
    from curl_cffi import requests as _cffi
except Exception:  # pragma: no cover - the optional dependency is simply absent
    _cffi = None

BASE = "https://www.sedarplus.ca"
IMPERSONATE = "chrome"
PACE_SECONDS = 2.0            # between actions, so a lookup is a person's pace, not a sweep
TIMEOUT = 90
DOC_TIMEOUT = 180
SEARCH_LIMIT = 100           # rows asked for in one document search
UA_NOTE = "Bagholder, on-demand, for the account holder's own research"


class SedarUnavailable(D.SourceUnavailable):
    """The site could not be reached or curl_cffi is not installed."""


class ProfileNotFound(Exception):
    """No SEDAR+ reporting-issuer profile matched the query."""


def available():
    """True when the optional dependency that clears the bot gate is installed."""
    return _cffi is not None


# --------------------------------------------------------------------------- #
# Session
# --------------------------------------------------------------------------- #
_lock = threading.Lock()
_session = None
_last_action = 0.0


def _pace():
    global _last_action
    wait = _last_action + PACE_SECONDS - time.time()
    if wait > 0:
        time.sleep(wait)
    _last_action = time.time()


def _new_session():
    if _cffi is None:
        raise SedarUnavailable("curl_cffi is not installed; run: pip install curl_cffi")
    return _cffi.Session(impersonate=IMPERSONATE)


def _get_session():
    global _session
    if _session is None:
        _session = _new_session()
    return _session


def reset():
    """Drop the session so the next call opens a fresh one (used after a gate bounce)."""
    global _session
    with _lock:
        _session = None


# --------------------------------------------------------------------------- #
# The form protocol
# --------------------------------------------------------------------------- #
_FIELD_RE = re.compile(r"<(input|select|textarea)\b([^>]*)>", re.I)
_NAME_RE = re.compile(r'name="([^"]*)"')
_TYPE_RE = re.compile(r'type="([^"]*)"')
_VALUE_RE = re.compile(r'value="([^"]*)"')
_SELECTED_RE = re.compile(r'<option[^>]*selected[^>]*value="([^"]*)"|value="([^"]*)"[^>]*selected')


def _form_fields(html):
    """Serialize the form the way the browser would before a callback: every named
    input, the selected option of every select, checked boxes only. Callback (_CB…)
    fields are dropped so the caller sets them cleanly."""
    out = []
    for m in _FIELD_RE.finditer(html):
        tag, attrs = m.group(1).lower(), m.group(2)
        name = _NAME_RE.search(attrs)
        if not name or name.group(1).startswith("_CB"):
            continue
        nm = name.group(1)
        if tag == "input":
            typ = (_TYPE_RE.search(attrs).group(1) if _TYPE_RE.search(attrs) else "text").lower()
            if typ in ("submit", "button", "file"):
                continue
            if typ in ("checkbox", "radio") and "checked" not in attrs:
                continue
            val = _VALUE_RE.search(attrs)
            out.append((nm, _html.unescape(val.group(1)) if val else ""))
        elif tag == "select":
            body = html[m.end():html.find("</select>", m.end())]
            sel = _SELECTED_RE.search(body)
            out.append((nm, _html.unescape(sel.group(1) or sel.group(2)) if sel else ""))
    return out


_VI_PARAM_RE = re.compile(r'<input\b([^>]*class="[^"]*viewInstanceFormParameter[^"]*"[^>]*)>', re.I)


def _vi_params(html):
    """The form's hidden viewInstanceFormParameter inputs, which every callback carries."""
    out = []
    for m in _VI_PARAM_RE.finditer(html):
        name = _NAME_RE.search(m.group(1))
        val = _VALUE_RE.search(m.group(1))
        if name:
            out.append((name.group(1), _html.unescape(val.group(1)) if val else ""))
    return out


# The primary search trigger on a Catalyst list page: either an <… appSearchButton …>
# (the document search) or an <… id="node…-searchButton" …> (the reporting-issuer
# list). Its onclick names the callback node, the callback name (buttonPush or
# fireOnChange) and the async container node. Discovered per page, since the ids
# differ from one service to the next.
_SEARCH_ACTION_RE = re.compile(
    r'(?:appSearchButton|-searchButton)[^>]*?onclick="[^"]*?cat\w*Callback\(\'(W\d+)\',\'(\w+)\'[^"]*?containerNodeId:\'(W\d+)\'',
    re.S,
)


def _search_action(page):
    """(node, name, container) for a page's Search control, or None."""
    m = _SEARCH_ACTION_RE.search(page)
    return (m.group(1), m.group(2), m.group(3)) if m else None


_MENU_ANCHOR = re.compile(r"<a[^>]*?catCallback\('(W\d+)','invokeMenuCb'[^>]*>(.*?)</a>", re.S)
_DOCS_MENU_TEXT = "search and download documents for this profile"


def _issuer_menu_node(html, name=None):
    """On a reporting-issuer result, the menu node that opens the issuer itself —
    the one whose link text is the issuer name, not a generic header action."""
    fallback = None
    want = _text(name or "")[:20].lower()
    for m in _MENU_ANCHOR.finditer(html):
        t = _text(m.group(2))
        if not t or "search for profiles" in t.lower():
            continue
        if want and want in t.lower():
            return m.group(1)
        fallback = fallback or m.group(1)
    return fallback


def _docs_menu_node(html):
    """On an issuer profile, the 'Search and download documents for this profile'
    menu node."""
    idx = html.lower().find(_DOCS_MENU_TEXT)
    if idx < 0:
        return None
    start = html.rfind("<a ", 0, idx)
    m = re.search(r"catCallback\('(W\d+)'", html[start:idx]) if start >= 0 else None
    return m.group(1) if m else None


class _View:
    """One opened service instance: its page, ids and session headers."""

    def __init__(self, service):
        self.service = service
        _pace()
        try:
            r = _get_session().get(
                "%s/csa-party/service/create.html?targetAppCode=csa-party&service=%s" % (BASE, service),
                timeout=TIMEOUT,
            )
        except Exception as e:
            raise SedarUnavailable("could not open %s: %s" % (service, e))
        page = r.text
        if "validate.perfdrive.com" in (r.url or "") or "validate.perfdrive.com" in page[:2000]:
            raise SedarUnavailable("the SEDAR+ bot gate turned the request away")
        m_inst = re.search(r"viewInstance/view\.html\?id=([0-9a-f]+)", r.url or "") or re.search(r"update\.html\?id=([0-9a-f]+)", page)
        m_key = re.search(r"viewInstanceKey:'([^']+)'", page)
        m_sid = re.search(r"sessionId:'([^']+)'", page)
        m_app = re.search(r"/(csa-\w+)/viewInstance", r.url or "")
        if not (m_inst and m_key and m_sid):
            raise SedarUnavailable("SEDAR+ did not return the %s form" % service)
        self.app = m_app.group(1) if m_app else "csa-party"
        self.inst = m_inst.group(1)
        self.key = m_key.group(1)
        self.sid = m_sid.group(1)
        self.page = page
        self.ref = "%s/%s/viewInstance/view.html?id=%s" % (BASE, self.app, self.inst)

    def _headers(self, async_):
        h = {
            "x-catalyst-session-global": self.sid,
            "x-security-token": "null",
            "Referer": self.ref,
            "Origin": BASE,
            "Content-Type": "application/x-www-form-urlencoded; charset=UTF-8",
        }
        if async_:
            h["x-catalyst-async"] = "true"
            h["x-catalyst-secured"] = "true"
            h["X-Requested-With"] = "XMLHttpRequest"
        return h

    def callback(self, node, name, value=None, extra=None, container=None, json_frag=False, html=None):
        """Post the form back with one callback. `container` set means an async
        HTML-fragment update (a search or a page change); `json_frag` a JSON
        autocomplete reply; otherwise a full-page callback (a selection). Fields in
        `extra` override the form's own values of the same name. Returns the text."""
        extra = dict(extra or {})
        if json_frag:
            # A JSON fragment callback carries only the view's hidden parameters,
            # not the whole form (that is how the site's own autocomplete posts).
            data = _vi_params(html if html is not None else self.page)
        else:
            data = [(k, v) for k, v in _form_fields(html if html is not None else self.page) if k not in extra]
        data += [("_CBNODE_", node), ("_CBNAME_", name), ("_VIKEY_", self.key)]
        if value is not None:
            data.append(("_CBVALUE_", value))
        if container:
            data += [
                ("_CBHTMLFRAG_", "true"),
                ("_CBHTMLFRAGID_", str(int(time.time() * 1000))),
                ("_CBHTMLFRAGNODEID_", container),
                ("_CBASYNCUPDATE_", "true"),
            ]
        if json_frag:
            data.append(("_CBJSONFRAG_", "true"))
        for k, v in extra.items():
            data.append((k, v))
        _pace()
        try:
            r = _get_session().post(
                "%s/%s/viewInstance/update.html?id=%s" % (BASE, self.app, self.inst),
                data=urlencode(data), headers=self._headers(bool(container) or json_frag), timeout=TIMEOUT,
            )
        except Exception as e:
            raise SedarUnavailable("callback %s/%s failed: %s" % (node, name, e))
        return r.text

    def refresh_identity(self, html):
        """After a full-page navigation (a menu that pushes a new view), adopt the
        new view instance's id and key from the returned page. Each stack push is a
        fresh instance; a callback posted to the old id is refused."""
        m_inst = re.search(r"viewInstance/view\.html\?id=([0-9a-f]+)", html) or re.search(r"update\.html\?id=([0-9a-f]+)", html)
        m_key = re.search(r"viewInstanceKey:'([^']+)'", html)
        m_sid = re.search(r"sessionId:'([^']+)'", html)
        if m_inst and m_key:
            self.inst = m_inst.group(1)
            self.key = m_key.group(1)
            if m_sid:
                self.sid = m_sid.group(1)
            self.ref = "%s/%s/viewInstance/view.html?id=%s" % (BASE, self.app, self.inst)
            self.page = html
            return True
        return False

    def resource(self, url_or_node, timeout=DOC_TIMEOUT):
        """GET a resource on this session: a full URL from a result row, or a node id."""
        if url_or_node.startswith("http"):
            url = _html.unescape(url_or_node)
        else:
            url = "%s/%s/viewInstance/resource.html?node=%s&id=%s" % (BASE, self.app, url_or_node, self.inst)
        _pace()
        try:
            r = _get_session().get(url, headers={"Referer": self.ref}, timeout=timeout)
        except Exception as e:
            raise SedarUnavailable("resource fetch failed: %s" % e)
        return r


# --------------------------------------------------------------------------- #
# Parsers (pure; the contract the tests pin down)
# --------------------------------------------------------------------------- #
_TAGS = re.compile(r"<[^>]+>")
_WS = re.compile(r"\s+")


def _text(s):
    return _WS.sub(" ", _html.unescape(_TAGS.sub(" ", s or ""))).strip()


def filing_id(url, profile_no="", file="", submitted=""):
    """A stable id for a filing: the document's drmKey when the row carries one,
    otherwise a short digest of the profile, file name and submitted time, so the
    same filing keeps its id across refreshes and the cache never doubles it."""
    m = re.search(r"drmKey=([0-9a-f]+)", url or "")
    if m:
        return "drm:" + m.group(1)
    import hashlib
    seed = "|".join((profile_no or "", file or "", submitted or "", url or ""))
    return "h:" + hashlib.sha1(seed.encode("utf-8")).hexdigest()[:16]


_ISSUER = re.compile(r'appReceiveFocus">\s*([^<]*?\((\d{9})\))\s*</span>')
_DOC_LINK = re.compile(r'<a class="appDocumentView appResourceLink appDocumentLink" href="([^"]+)"[^>]*>\s*<span>(.*?)</span>', re.S)
_SUBMITTED = re.compile(r'<span aria-hidden="true">\s*(\d{1,2} \w{3} \d{4}[^<]*?)\s*</span>')
_SIZE = re.compile(r'(\d[\d.,]* ?(?:KB|MB|bytes))', re.I)


def parse_filings(html):
    """Document search result rows into filing dicts, newest first as the page gives them.

    Each row carries the issuer with its nine-digit profile number, the document
    file name, its submitted timestamp, its size, and a direct link that returns
    the document on the same session. The raw text is kept as SEDAR+ shows it."""
    out = []
    for m in _DOC_LINK.finditer(html):
        url = _html.unescape(m.group(1))
        before = html[max(0, m.start() - 2600):m.start()]
        after = html[m.end():m.end() + 1400]
        issuer_m = None
        for issuer_m in _ISSUER.finditer(before):
            pass  # the last issuer before this link is this row's
        sub = _SUBMITTED.search(after)
        size = _SIZE.search(after)
        profile_no = issuer_m.group(2) if issuer_m else ""
        file = _text(m.group(2))
        submitted = sub.group(1).strip() if sub else ""
        out.append({
            "id": filing_id(url, profile_no, file, submitted),
            "issuer": _text(issuer_m.group(1)) if issuer_m else "",
            "profileNo": profile_no,
            "file": file,
            "submitted": submitted,
            "submittedAt": _iso(submitted),
            "size": size.group(1) if size else "",
            "url": url,
        })
    return out


_MONTHS = {m: i for i, m in enumerate(
    ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"], 1)}


def _iso(submitted):
    """SEDAR+'s "13 Sep 2026 20:42 EDT" into a sortable "2026-09-13T20:42", the zone
    dropped (it is only ever Eastern). Returns "" when the text does not parse."""
    m = re.match(r"(\d{1,2}) (\w{3}) (\d{4})(?:\s+(\d{1,2}):(\d{2}))?", submitted or "")
    if not m or m.group(2) not in _MONTHS:
        return ""
    d, mon, y = int(m.group(1)), _MONTHS[m.group(2)], int(m.group(3))
    hh, mm = int(m.group(4) or 0), int(m.group(5) or 0)
    return "%04d-%02d-%02dT%02d:%02d" % (y, mon, d, hh, mm)


_RI_ROW = re.compile(r"<tr[^>]*appTblRow[^>]*>(.*?)</tr>", re.S)
_TD = re.compile(r"<td[^>]*>(.*?)</td>", re.S)


def parse_reporting_issuers(html):
    """Reporting-issuer search rows into {name, profileNo, provinces, jurisdiction, type}.

    The row's columns are: a select box, the issuer name, its nine-digit profile
    number, a blank, an identifier, the provinces it reports in, its principal
    jurisdiction, and its type. Everything is indexed off the profile-number cell,
    so a shifted or empty leading cell does not move the fields."""
    out = []
    for row in _RI_ROW.finditer(html):
        cells = [_text(c) for c in _TD.findall(row.group(1))]
        idx = next((i for i, c in enumerate(cells) if re.fullmatch(r"\d{9}", c)), None)
        if idx is None:
            continue
        cell = lambda i: cells[i] if 0 <= i < len(cells) else ""
        out.append({
            "name": cell(idx - 1),
            "profileNo": cells[idx],
            "provinces": cell(idx + 3),
            "jurisdiction": cell(idx + 4),
            "type": cell(idx + 5),
        })
    return out


# --------------------------------------------------------------------------- #
# High-level operations
# --------------------------------------------------------------------------- #
def resolve_profile(query):
    """Every SEDAR+ reporting-issuer profile matching a name or number, best first.

    Raises ProfileNotFound when nothing matches, SedarUnavailable on a site or
    dependency problem."""
    q = (query or "").strip()
    if not q:
        raise ProfileNotFound("empty query")
    with _lock:
        view = _View("searchReportingIssuers")
        action = _search_action(view.page)
        if not action:
            raise SedarUnavailable("could not find the reporting-issuer search control on the page")
        node, name, container = action
        html = view.callback(node, name, extra={"QueryString": q}, container=container)
    rows = parse_reporting_issuers(html)
    if not rows:
        raise ProfileNotFound("no SEDAR+ profile matched %r" % q)
    ql = q.lower()
    rows.sort(key=lambda r: (r["profileNo"] != q, ql not in r["name"].lower(), not r["name"].lower().startswith(ql)))
    return rows


def list_filings(query=None, profile_no=None, limit=SEARCH_LIMIT):
    """Filings for one issuer, resolving the issuer from `query` when no profile
    number is given. Returns {"profile": {...}, "filings": [...], "scoped": bool}.

    When neither is given, returns the newest filings across SEDAR+ (the search
    page's default view). `scoped` is False when the issuer could be resolved but
    the document search could not be constrained to it; the filings list is then
    only what the default view holds for that profile, which may be empty."""
    profile = None
    if not profile_no and query:
        matches = resolve_profile(query)
        profile = matches[0]
        profile_no = profile["profileNo"]
    scoped = None
    with _lock:
        if profile_no:
            html = _scoped_documents(profile_no, (profile or {}).get("name") or query)
            scoped = html is not None
            if html is None:
                html = _View("searchDocuments").page   # fall back to the default view
        else:
            html = _View("searchDocuments").page
    filings = parse_filings(html)
    if profile_no:
        filings = [f for f in filings if not f["profileNo"] or f["profileNo"] == profile_no]
    return {
        "profile": profile or ({"profileNo": profile_no} if profile_no else None),
        "scoped": scoped,
        "filings": filings[: max(1, int(limit))],
    }


def _scoped_documents(profile_no, name=None):
    """The document search results for one profile, or None if the chain could not
    be walked. SEDAR+ has no profile parameter on the document search; the way in
    is the issuer's own page: search the reporting-issuer list for the profile,
    open the issuer, and follow its 'Search and download documents for this
    profile' link. Each step pushes a new view instance, so the id is refreshed
    from every full-page response."""
    try:
        view = _View("searchReportingIssuers")
        action = _search_action(view.page)
        if not action:
            return None
        node, cbname, container = action
        ri = view.callback(node, cbname, extra={"QueryString": profile_no}, container=container)
        issuer_node = _issuer_menu_node(ri, name)
        if not issuer_node:
            return None
        profile_page = view.callback(issuer_node, "invokeMenuCb", html=ri)
        view.refresh_identity(profile_page)
        docs_node = _docs_menu_node(view.page)
        if not docs_node:
            return None
        docs = view.callback(docs_node, "invokeMenuCb", html=view.page)
        view.refresh_identity(docs)
        if "appDocumentLink" in view.page:
            return view.page
        action = _search_action(view.page)
        if action:
            return view.callback(action[0], action[1], container=action[2], html=view.page)
        return view.page
    except SedarUnavailable:
        return None


def newest(limit=30):
    """The newest filings across SEDAR+, as the search page opens (no issuer filter)."""
    with _lock:
        view = _View("searchDocuments")
        html = view.page
    return parse_filings(html)[: max(1, int(limit))]


def _is_document(response):
    """A real document, not the site's HTML error page. Document URLs are bound to
    the session that minted them, so a stale URL comes back as HTML; reject that."""
    ct = (response.headers.get("content-type") or "").lower()
    body = response.content or b""
    if response.status_code != 200 or not body:
        return False
    if "text/html" in ct:
        return False
    return body.lstrip()[:1] != b"<"


def _download_bytes(profile_no, doc_id, name=None):
    """The core download: re-scope to the profile, match the document by its drmKey,
    fetch it in that live session. Returns (bytes, content_type). A document URL is
    session-bound, so the URL is re-minted here rather than reused from storage."""
    key = (doc_id or "").split(":")[-1]   # accepts "drm:…", "sedar:drm:…" or a bare drmKey
    with _lock:
        html = _scoped_documents(profile_no, name)
        if html is None:
            raise SedarUnavailable("could not open the profile's documents to download from")
        row = next((f for f in parse_filings(html) if key and key in f["url"]), None)
        if not row:
            raise ProfileNotFound("no document %r in profile %s" % (doc_id, profile_no))
        try:
            r = _get_session().get(_html.unescape(row["url"]),
                                   headers={"Referer": BASE + "/csa-party/viewInstance/view.html"}, timeout=DOC_TIMEOUT)
        except Exception as e:
            raise SedarUnavailable("document fetch failed: %s" % e)
    if not _is_document(r):
        raise SedarUnavailable("document did not download (status %s)" % r.status_code)
    return r.content, r.headers.get("content-type", "application/pdf")


def download(profile_no, doc_id, dest, name=None):
    """Download one of a profile's documents to `dest`, matched by its id (the
    `drm:…` id from a filing row). Returns (path, content_type, bytes)."""
    data, ct = _download_bytes(profile_no, doc_id, name)
    with open(dest, "wb") as fh:
        fh.write(data)
    return dest, ct, len(data)


# --------------------------------------------------------------------------- #
# Provider interface (see disclosures.py)
# --------------------------------------------------------------------------- #
SOURCE = "SEDAR+"
CA_EXCHANGES = {"TSX", "TSXV", "TSX-V", "TSXV", "CSE", "CNSX", "NEO", "NEO EXCHANGE",
                "CBOE CANADA", "AQL", "TSX VENTURE", "CANADIAN SECURITIES EXCHANGE"}


def covers(symbol, exchange="", currency=""):
    """SEDAR+ applies to Canadian listings. A US listing (currency USD) is left to
    EDGAR; anything Canadian, or of unknown venue in CAD, is ours."""
    ex = (exchange or "").upper()
    cur = (currency or "").upper()
    if cur == "USD" or ex in ("NASDAQ", "NYSE", "AMEX", "ARCA", "US"):
        return False
    return cur == "CAD" or ex in CA_EXCHANGES or (not ex and not cur)


def _sedar_category(file):
    """Map a SEDAR+ document name to the shared category vocabulary."""
    f = (file or "").lower()
    if "news release" in f or "press release" in f:
        return D.NEWS
    if any(k in f for k in ("md&a", "financial statement", "annual report", "interim", "certification", "52-109", "financial report")):
        return D.FINANCIALS
    if "material change" in f:
        return D.EVENTS
    if any(k in f for k in ("circular", "proxy", "voting results", "meeting", "information circular")):
        return D.GOVERNANCE
    if any(k in f for k in ("prospectus", "offering", "45-106", "exempt distribution", "45-102", "rights offering", "45-108")):
        return D.OFFERINGS
    if any(k in f for k in ("insider", "early warning", "45-101", "issuer bid")):
        return D.INSIDER
    return D.OTHER


def _to_item(raw, profile_no):
    return {
        "id": "sedar:" + raw.get("id", ""),
        "source": SOURCE,
        "category": _sedar_category(raw.get("file")),
        "date": raw.get("submittedAt") or "",
        "dateText": raw.get("submitted") or "",
        "type": raw.get("file") or "",
        "title": "",
        "size": raw.get("size") or "",
        "url": raw.get("url") or "",
        "issuer": raw.get("issuer") or "",
        "profileNo": raw.get("profileNo") or profile_no or "",
    }


def fetch(symbol, name="", exchange="", currency="", limit=SEARCH_LIMIT):
    """One Canadian issuer's SEDAR+ filings as normalized disclosure items. Resolves
    the issuer from its name (or the bare symbol); returns [] when none matches."""
    try:
        result = list_filings(query=(name or symbol), limit=limit)
    except ProfileNotFound:
        return []
    profile_no = (result.get("profile") or {}).get("profileNo") or ""
    return [_to_item(r, profile_no) for r in result.get("filings") or []]


def document(row):
    """Download one SEDAR+ document named by a stored row (its id and profileNo).
    Returns (bytes, content_type)."""
    return _download_bytes((row or {}).get("profileNo") or "", (row or {}).get("id") or "", (row or {}).get("issuer"))


# --------------------------------------------------------------------------- #
# CLI: JSON on stdout, so Claude Code (or anything) can call it directly
# --------------------------------------------------------------------------- #
def _main(argv):
    import json

    if not argv or argv[0] in ("-h", "--help", "help"):
        sys.stderr.write(
            "sedar.py — SEDAR+ filings over HTTP\n"
            "  python3 sedar.py resolve <name|number>       profiles matching an issuer\n"
            "  python3 sedar.py filings <name|number> [n]   an issuer's filings (JSON)\n"
            "  python3 sedar.py newest [n]                  newest filings across SEDAR+\n"
            "  python3 sedar.py get <profileNo> <id> <dest.pdf>   download one document\n"
        )
        return 2
    if not available():
        print(json.dumps({"ok": False, "error": "curl_cffi not installed; run: pip install curl_cffi"}))
        return 1
    cmd = argv[0]
    try:
        if cmd == "resolve":
            print(json.dumps({"ok": True, "profiles": resolve_profile(argv[1])}, indent=2))
        elif cmd == "filings":
            limit = int(argv[2]) if len(argv) > 2 else SEARCH_LIMIT
            print(json.dumps({"ok": True, **list_filings(query=argv[1], limit=limit)}, indent=2))
        elif cmd == "newest":
            limit = int(argv[1]) if len(argv) > 1 else 30
            print(json.dumps({"ok": True, "filings": newest(limit)}, indent=2))
        elif cmd == "get":
            if len(argv) < 4:
                sys.stderr.write("usage: python3 sedar.py get <profileNo> <id> <dest.pdf>\n")
                return 2
            path, ct, n = download(argv[1], argv[2], argv[3])
            print(json.dumps({"ok": True, "path": path, "contentType": ct, "bytes": n}))
        else:
            sys.stderr.write("unknown command %r\n" % cmd)
            return 2
    except (ProfileNotFound, SedarUnavailable) as e:
        print(json.dumps({"ok": False, "error": str(e)}))
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(_main(sys.argv[1:]))
