"""News for the symbols the book holds and watches, from every public per-symbol source that
proved to carry it, merged: TMX Money for Canadian listings (its press releases and its "In The
Media" stories) and Nasdaq for US ones, and beside them Yahoo Finance's news gateway, Seeking
Alpha's per-symbol feed and Google News. No one source carries everything — TMX has no stories
at all for some of the funds a book holds, where Google does — so a listing's news is the union
of what each of them has for it, one row per story however many carry it.

An item belongs to a listing only when its source says so: TMX's press releases are the ones it
files under the listing, and its In The Media stories, Nasdaq, Yahoo and Seeking Alpha tag every
item with tickers, an item kept only where those tags name the listing. Google News tags nothing,
so an item from it is kept only when its own headline names the listing — the listing's name, or
its ticker in an exchange's own form — never because a search returned it."""
from __future__ import annotations

import hashlib
import html
import json
import re
import sys
import threading
import time
import unicodedata
import urllib.parse
from concurrent.futures import ThreadPoolExecutor
from datetime import datetime, timedelta, timezone
from email.utils import parsedate_to_datetime

import market
import store

# TMX's quote page has two news tabs on one query: "Press Releases", the companies' own wire items,
# and "In The Media", publishers' stories about the company (companyInNews). Both are read.
TMX_NEWS_QUERY = ("query getNewsForSymbol($symbol: String!, $page: Int!, $limit: Int!, $locale: String!, $companyInNews: Boolean) "
                  "{ news: getNewsForSymbol(symbol: $symbol, page: $page, limit: $limit, locale: $locale, companyInNews: $companyInNews) "
                  "{ headline datetime source newsid summary topic } }")
TMX_NEWS_URL = "https://money.tmx.com/en/quote/%s/news/%s"
NASDAQ_NEWS_URL = "https://api.nasdaq.com/api/news/topic/articlebysymbol?q=%s|STOCKS&offset=0&limit=%d"
NASDAQ_LATEST_URL = "https://api.nasdaq.com/api/news/topic/latestnews?offset=0&limit=%d"
NASDAQ_PRESS_URL = "https://api.nasdaq.com/api/news/topic/press_release?q=symbol:%s|assetclass:stocks&offset=0&limit=%d"   # a US listing's own releases, beside its news
# a release wire, by its name: GlobeNewswire, Business Wire, PR Newswire, ACCESS Newswire (Accesswire), TheNewsWire,
# Canada Newswire (CNW), TMX Newsfile, Marketwired, NewMediaWire, Cision, PRWeb. A newsroom whose name only contains
# the letters is a publisher: WIRED, and the plural news services, MT Newswires and Dow Jones Newswires, write stories.
WIRE_NAMES = re.compile(r"newswire(?!s)|business ?wire|accesswire|newmediawire|marketwired|newsfile|cision|\bcnw\b|prweb", re.I)
UA = "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/128.0 Safari/537.36"
NASDAQ_HEADERS = {"User-Agent": UA, "Accept": "application/json, text/plain, */*", "Origin": "https://www.nasdaq.com", "Referer": "https://www.nasdaq.com/"}
TMX_HEADERS = {"User-Agent": UA, "locale": "en", "Origin": "https://money.tmx.com", "Referer": "https://money.tmx.com/"}
PER_SYMBOL = 12
PER_LISTING = 60         # the newest items a listing keeps once every source is merged
# the market-wide feed is a listing of its own: Nasdaq's latest news, whatever it names
MARKET = ("*", "MARKET", "")
PER_MARKET = 50
FRESH_MINUTES = 15
KEEP = 4000           # items kept in the database, newest first: every listing's merged sources

_last_call = {}


_pace_lock = threading.Lock()
LISTINGS_AT_ONCE = 4     # listings read side by side in a pass; each host stays paced across all of them


def _pace(host, seconds=0.6):
    """Wait for this host's next turn. Turns are handed out under a lock, so listings read side by
    side still ask each host one at a time, `seconds` apart."""
    with _pace_lock:
        now = time.time()
        turn = max(now, _last_call.get(host, 0) + seconds)
        _last_call[host] = turn
    if turn > now:
        time.sleep(turn - now)


def _s(v):
    return "" if v is None else str(v)


YAHOO_GATEWAY = "https://nexus-gateway-prod.media.yahoo.com/"
# the query Yahoo's own quote page sends for a ticker's News tab, unchanged
YAHOO_NEWS_QUERY = 'query FinancePolarisTickerNews($listInput:LightyearListInput!,$clientContext:ClientContext!,$mlRecsInput:MLRecsInput!,$gqlContext:[GqlContext]=[],$imageResize:[ImageResizeInput!]!=[],$first:Int,$mlRecsFirst:Int,$after:String,$offset:Int){lightyearList(list_input:$listInput,cc:$clientContext,first:$first){...HydratedLightyearListStoryVideoStreamPolarisWithPagination}}\nfragment ResizedResolutions on ImageResized{url height width transformLabel}\nfragment Image on Image{type:imgType originalUrl:url originalHeight:height originalWidth:width resolutions:resized(resizeInput:$imageResize){...ResizedResolutions}}\nfragment ContentAttributes on ContentAttributes{description summary pubDate:publishTime displayTime isHosted canonicalUrl clickthroughUrl(cc:$clientContext) provider{displayName url providerContentUrl providerId} thumbnail{...Image} mabMeta{mabLogString}}\nfragment FinanceStockTickers on Finance{stockTickers{symbol}}\nfragment StoryData on Story{id:uuid __typename title previewUrl(cc:$clientContext) isPremiumNews isLiveBlog embeddedLiveBlog{status} contentAttributes{...ContentAttributes} finance{...FinanceStockTickers}}\nfragment VideoData on Video{id:uuid __typename title duration previewUrl(cc:$clientContext) liveEventInfo{scheduledStartTime scheduledStopTime status} contentAttributes{...ContentAttributes} finance{...FinanceStockTickers}}\nfragment OutlinkData on Outlink{__typename uuid description displayTime headline url provider{displayName url providerContentUrl providerId} contentAttributes{thumbnail{...Image}}}\nfragment HydratedAssetRefStoryOrVideo on AssetRef{__typename asset(gqlContext:$gqlContext){__typename ... on Story{...StoryData} ... on Video{...VideoData} ... on Outlink{...OutlinkData}}}\nfragment HydratedLightyearListStoryVideoStreamPolarisWithPagination on LightyearList{main:mlRecsStream(mlRecsInput:$mlRecsInput,first:$mlRecsFirst,after:$after,offset:$offset){edges{node{...HydratedAssetRefStoryOrVideo}} pagination:pageInfo{nextPage:hasNextPage endCursor} totalCount}}'
YAHOO_HEADERS = {"x-yahoo-cg-client-name": "finance", "Origin": "https://finance.yahoo.com", "Referer": "https://finance.yahoo.com/"}
SA_NEWS_URL = "https://seekingalpha.com/api/sa/combined/%s.xml"
GNEWS_URL = "https://news.google.com/rss/search?q=%s&hl=en-CA&gl=CA&ceid=CA:en"
FEED_HEADERS = {"User-Agent": UA, "Accept": "application/rss+xml, application/xml, text/xml, */*"}


def kind_of(source):
    """What an item is, told by where it came from: a wire carries the company's own
    release, a publisher writes a story about it."""
    return "release" if WIRE_NAMES.search(_s(source)) else "story"


def clean_text(t):
    return re.sub(r"\s+", " ", html.unescape(_s(t))).strip()


def tmx_names(topic, symbol):
    """Whether TMX's own topic codes on an item name the listing, asked in the venue's form: a
    Canadian listing's code carries its market (`PNG:CA`, `HG:CNX`, `HBIX:AQL`) and a US listing's
    is the bare ticker (`ASTS`), so Telus (`T`) is never named by AT&T's `T`, nor HydroGraph (`HG`)
    by the NYSE's. The codes are TMX's tag, not a reading of the headline."""
    bare, _, suffix = _s(symbol).strip().upper().partition(":")
    if not bare:
        return False
    us = suffix == "US"
    for code in (c.strip().upper() for c in _s(topic).strip().strip("[]").split(",")):
        head, _, market_ = code.partition(":")
        if head == bare and ((us and market_ in ("", "US")) or (not us and market_ in ("CA", "CNX", "AQL"))):
            return True
    return False


def parse_tmx_news(data, symbol, media=False):
    """TMX's items for a symbol (in the venue's form, as `tmx_quote_symbol` gives it) into news
    rows: the headline, its exact time, the wire it came on, and TMX's page for it. `media` is the
    In The Media tab: publishers' stories, each kept only where TMX's own topic codes name the
    listing, since a story is about a company only as TMX tags it."""
    rows = []
    for it in ((data or {}).get("data") or {}).get("news") or []:
        if not isinstance(it, dict) or not it.get("newsid"):
            continue
        if media and not tmx_names(it.get("topic"), symbol):
            continue
        when = _s(it.get("datetime"))
        try:
            ts = datetime.fromisoformat(when).astimezone(timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ")
        except ValueError:
            continue
        source = clean_text(it.get("source")).replace(" via QuoteMedia", "")
        rows.append({"id": "tmx:%s" % it["newsid"], "headline": clean_text(it.get("headline")), "source": source,
                     "url": TMX_NEWS_URL % (symbol, it["newsid"]), "publishedAt": ts,
                     "kind": "story" if media else kind_of(source), "via": "tmx-media" if media else "tmx"})
    return rows


_AGO = re.compile(r"(\d+)\s+(minute|hour|day)s?\s+ago", re.I)


def nasdaq_when(row, now):
    """Nasdaq gives a day and an age ('17 minutes ago'): the time is the age taken off now,
    to the minute; older items keep the day alone at midnight UTC."""
    m = _AGO.search(_s(row.get("ago")))
    if m:
        n, unit = int(m.group(1)), m.group(2).lower()
        delta = timedelta(minutes=n) if unit == "minute" else timedelta(hours=n) if unit == "hour" else timedelta(days=n)
        return (now - delta).strftime("%Y-%m-%dT%H:%M:00Z")
    try:
        return datetime.strptime(_s(row.get("created")), "%b %d, %Y").strftime("%Y-%m-%dT00:00:00Z")
    except ValueError:
        return ""


def parse_nasdaq_news(data, now=None, symbol="", kind=None):
    """Nasdaq pads a symbol's feed with market-wide pieces; an item is kept only when the
    symbol is among the ones Nasdaq itself lists for it (a feed asked without a symbol keeps
    all). An item's kind is the feed's when it has one (the press-release feed), else told
    by its publisher; a release Nasdaq names no wire for reads as Nasdaq's."""
    now = now or datetime.now(timezone.utc)
    want = _s(symbol).strip().lower()
    rows = []
    for it in ((data or {}).get("data") or {}).get("rows") or []:
        if not isinstance(it, dict) or not it.get("id") or not it.get("title"):
            continue
        if want:
            named = {_s(x).split("|")[0].strip().lower() for x in (it.get("related_symbols") or [])} | {_s(it.get("primarysymbol")).strip().lower()}
            if want not in named:
                continue
        when = nasdaq_when(it, now)
        if not when:
            continue
        url = _s(it.get("url"))
        source = clean_text(it.get("publisher")) or ("Nasdaq" if kind == "release" else "")
        rows.append({"id": "nasdaq:%s" % it["id"], "headline": clean_text(it.get("title")), "source": source,
                     "url": url if url.startswith("http") else "https://www.nasdaq.com" + url, "publishedAt": when, "kind": kind or kind_of(source)})
    return rows


def _iso(value):
    """A feed's date — RFC 822 or ISO — in the app's own form; "" when it is not a real date (a
    quote page Google dates 1970)."""
    text = _s(value).strip()
    if not text:
        return ""
    try:
        when = parsedate_to_datetime(text) if not text[:4].isdigit() else datetime.fromisoformat(text.replace("Z", "+00:00"))
    except (TypeError, ValueError):
        return ""
    if when.tzinfo is None:
        when = when.replace(tzinfo=timezone.utc)
    return when.astimezone(timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ") if when.year >= 2000 else ""


def _tag(xml, name):
    m = re.search(r"<%s[^>]*>(.*?)</%s>" % (name, name), xml, re.S)
    if not m:
        return ""
    text = m.group(1)
    cdata = re.match(r"\s*<!\[CDATA\[(.*?)\]\]>\s*$", text, re.S)
    return clean_text(cdata.group(1) if cdata else text)


# --- Yahoo Finance: the news gateway behind a quote page's News tab ----------------------------

def yahoo_form(symbol, exchange, currency):
    """The ticker the gateway tags items with: `PNG.V`, `SXHI.TO`, `HG.CN`, `HBIX.NE`, or `ASTS`, as
    every other Yahoo read names the listing; a Canadian listing with no venue takes the one TMX
    answered to."""
    if market.tmx_form(exchange, currency) is None:
        return ""
    forms = market.yahoo_forms({"symbol": symbol, "exchange": exchange, "currency": currency}) or []
    if forms and not _s(exchange).strip() and forms[0].endswith(".TO"):
        remembered = _s(market.tmx_remembered(market.tmx_symbol(symbol)))
        suffix = {":CNX": ".CN", ":AQL": ".NE"}.get(remembered[len(market.tmx_bare(remembered)):])
        if suffix:
            return market.yahoo_root(symbol) + suffix
    return _s(forms[0] if forms else "").upper()


_OTC_TWIN = re.compile(r"[A-Z]{4}[FY]")


def parse_yahoo_news(data, form, symbol="", name=""):
    """The gateway's items for a ticker, kept where Yahoo's own ticker tags name it. A Canadian
    company's items are often tagged only with its US over-the-counter twin (`CHHYF` for `CH.V`): a
    twin Yahoo tags alone beside the listing's own ticker on an item names it too, never a partner's
    symbol on an item that names several. An item Yahoo tags with nothing is kept when its headline names the listing."""
    assets = []
    edges = ((((data or {}).get("data") or {}).get("lightyearList") or {}).get("main") or {}).get("edges") or []
    for edge in edges:
        asset = ((edge or {}).get("node") or {}).get("asset") or {}
        if asset.get("id") and asset.get("title"):
            tags = {_s(t.get("symbol")).strip().upper() for t in ((asset.get("finance") or {}).get("stockTickers") or []) if isinstance(t, dict)} - {""}
            assets.append((asset, tags))
    twins = set()
    if "." in form:
        for _, tags in assets:
            other = tags - {form}
            if form in tags and len(other) == 1 and _OTC_TWIN.fullmatch(next(iter(other))):
                twins |= other
    us = "." not in form
    rows = []
    for asset, tags in assets:
        title = clean_text(asset.get("title"))
        if not (form in tags or tags & twins or (not tags and names_listing(title, symbol or form.split(".")[0], name, us))):
            continue
        attrs = asset.get("contentAttributes") or {}
        when = _iso(attrs.get("pubDate"))
        if not when:
            continue
        source = clean_text((attrs.get("provider") or {}).get("displayName")) or "Yahoo Finance"
        url = _s(attrs.get("canonicalUrl") or attrs.get("clickthroughUrl"))
        rows.append({"id": "yahoo:%s" % asset["id"], "headline": title, "source": source,
                     "url": url, "publishedAt": when, "kind": kind_of(source), "via": "yahoo"})
    return rows


def fetch_yahoo(symbol, exchange, currency, name="", ssl_context=None):
    """None when there is nothing to ask: no answer, so it never counts as the listing having been read."""
    form = yahoo_form(symbol, exchange, currency)
    if not form:
        return None
    alias = "finance-US-en-US-ticker-all"
    variables = {"clientContext": {"device": "DESKTOP", "region": "US", "site": "finance", "lang": "en-US"},
                 "gqlContext": [{"listAlias": "list=" + alias}], "imageResize": [],
                 "listInput": {"disableDedupe": False, "enableBlockedContent": False, "filterClientContext": False, "getFullList": True,
                               "enableQueryTimeLicenseCheck": True, "queryVariables": {"tickerSymbol": [form]}, "slug": "list=" + alias},
                 "mlRecsInput": {"count": 200, "instance": "FINANCE"}, "first": 100, "mlRecsFirst": 50}
    _pace("nexus-gateway-prod.media.yahoo.com", 1.0)
    data = market._post_json(YAHOO_GATEWAY, {"query": YAHOO_NEWS_QUERY, "operationName": "FinancePolarisTickerNews", "variables": variables},
                             ssl_context, YAHOO_HEADERS)
    return parse_yahoo_news(data, form, symbol, name)


# --- Seeking Alpha: a ticker's combined feed ----------------------------------------------------

def sa_form(symbol, exchange, currency):
    """Seeking Alpha's name for a listing: `PNG:CA` for the TSX and TSX-V, the bare ticker for a US
    one. It has no form for the CSE or Cboe Canada, which it reaches only through a US OTC symbol the
    app does not keep, so those are left to the other sources."""
    form = market.tmx_form(exchange, currency)
    sym = market.tmx_symbol(symbol).upper()
    if form == ":US":
        return sym
    if form == "":
        return sym + ":CA"
    return ""


def parse_sa_news(xml, form):
    """The feed's items, kept only where their own `sa:symbol` tags name the listing."""
    rows = []
    for item in re.findall(r"<item>(.*?)</item>", _s(xml), re.S):
        symbols = {clean_text(x).upper() for x in re.findall(r"<sa:symbol>(.*?)</sa:symbol>", item, re.S)}
        if form.upper() not in symbols:
            continue
        guid, title, when = _tag(item, "guid"), _tag(item, "title"), _iso(_tag(item, "pubDate"))
        if not (guid and title and when):
            continue
        rows.append({"id": "sa:%s" % hashlib.sha1(guid.encode("utf-8")).hexdigest()[:16], "headline": title, "source": "Seeking Alpha",
                     "url": _tag(item, "link") or guid, "publishedAt": when, "kind": "story", "via": "sa"})
    return rows


def fetch_sa(symbol, exchange, currency, ssl_context=None):
    """None when Seeking Alpha has no feed for the listing: nothing was read, and nothing failed."""
    form = sa_form(symbol, exchange, currency)
    if not form:
        return None
    _pace("seekingalpha.com", 1.0)
    try:
        xml = market._get_text(SA_NEWS_URL % urllib.parse.quote(form, safe=":"), ssl_context, headers=FEED_HEADERS)
    except Exception as e:
        if getattr(e, "code", None) == 404 or "404" in str(e):
            return None
        raise
    return parse_sa_news(xml, form)


# --- Google News: every publisher, no tags ------------------------------------------------------

_CORPORATE = {"inc", "incorporated", "corp", "corporation", "ltd", "limited", "plc", "co", "company", "holdings", "holding", "group",
              "nv", "sa", "ag", "se", "lp", "llc", "the", "class", "units", "unit", "shares", "common", "ordinary", "adr", "trust"}
_LABEL = re.compile(r"^(class|series)\s+\w+(\s+(units?|shares?))?$|^(units?|shares?|etf|fund|common( shares)?)$", re.I)
_FUNDISH = re.compile(r"\b(etf|fund|portfolio|trust)\b", re.I)
# the pages a quote site keeps per ticker, which Google lists beside the news: a price, a chart, statements
_QUOTE_PAGE = re.compile(r"price and chart|\b(stock|share) price\s*[,&]|share price - |holdings list|^technical analysis of|etf profile:|stock forecast and price target|"
                         r"^etfs investing in|forecast\s*[–—-]\s*price target|price prediction|tokenomics|price today: live|\brstock\b|"
                         r"\s[–—]\s(?:TSX|TSXV|CSE|NEO|NASDAQ|NYSE|AMEX|OTC)\s?:\s?[A-Z0-9.]+\s*$|^\$[^$]+\$$", re.I)
_CA_VENUES = ("TSX", "TSXV", "TSX-V", "CVE", "CSE", "CNSX", "CN", "NEO", "CBOE CANADA")
_US_VENUES = ("NASDAQ", "NYSE", "NYSEARCA", "NYSE ARCA", "NYSEAMERICAN", "NYSE AMERICAN", "AMEX", "BATS", "CBOE")
_CA_SUFFIXES = (".TO", ".V", ".CN", ".C", ".NE", ":CA", ":CNX", ":AQL")


# words that join a name's words and say nothing themselves
_JOINERS = {"of", "and", "de", "du", "des", "la", "le", "et", "for", "on", "at", "y"}
# words many companies' and funds' names start with, which name none of them alone: places, trades, kinds of company
_GENERIC = {"canadian", "canada", "american", "america", "national", "international", "global", "general", "united", "universal",
            "northern", "southern", "eastern", "western", "northwest", "pacific", "atlantic", "arctic", "central", "british",
            "european", "chinese", "mexican", "brazil", "quebec", "ontario", "alberta", "manitoba", "california", "nevada", "arizona",
            "alaska", "texas", "frontier", "pioneer", "liberty", "patriot", "heritage", "capital", "energy", "energies", "silver",
            "golden", "digital", "quantum", "advanced", "applied", "intuitive", "precision", "premium", "select", "strategic",
            "strategy", "summit", "bright", "lithium", "uranium", "copper", "nickel", "cobalt", "graphite", "metals", "mining",
            "resources", "minerals", "petroleum", "natural", "health", "healthcare", "medical", "pharma", "therapeutics",
            "sciences", "science", "technology", "technologies", "software", "systems", "network", "networks", "solutions",
            "services", "industries", "industrial", "financial", "finance", "investment", "investments", "partners", "income",
            "dividend", "growth", "equity", "innovation", "innovative", "materials", "hydrogen", "battery", "electric", "motors",
            "aerospace", "defence", "defense", "security", "securities", "standard", "interactive", "entertainment", "communications",
            "telecom", "wireless", "insurance", "realty", "properties", "estate", "infrastructure", "renewable", "renewables",
            "environmental", "agricultural", "foods", "brands", "consumer", "retail", "bancorp", "banking", "credit", "mortgage",
            "royalty", "royalties", "exploration", "minerals", "robotics", "biotech", "semiconductor", "semiconductors", "solar"}


def _fold(text):
    """Accents off, so `Québec` and `Quebec` are one word."""
    return "".join(c for c in unicodedata.normalize("NFKD", _s(text)) if not unicodedata.combining(c))


def _words(text):
    return re.findall(r"[a-z0-9]+", _fold(text).lower())


def search_name(name):
    """A listing's name as the press writes it, from the book's record of it: without what the
    record appends (`(the "ETF")`, `- Class A`, `- ETF`), and a manager's name put in front of its
    fund's (`Ninepoint Partners LP - Cameco Highshares ETF` is `Ninepoint Cameco Highshares ETF`)."""
    text = re.sub(r"\([^)]*\)", " ", _s(name))
    parts = [p.strip(" .,-") for p in re.split(r"\s+[-–—]\s+", text)]
    parts = [p for p in parts if p and not _LABEL.match(p)]
    if not parts:
        return ""
    if len(parts) > 1 and not _FUNDISH.search(parts[0]) and any(_FUNDISH.search(p) for p in parts[1:]):
        fund = next(p for p in parts[1:] if _FUNDISH.search(p))
        brand = [w for w in parts[0].split() if w.lower().strip(".,") not in _CORPORATE and w.lower().strip(".,") != "partners"]
        head = brand[0] if brand else ""
        text = fund if not head or fund.lower().startswith(head.lower()) else head + " " + fund
    else:
        text = parts[0]
    text = re.sub(r"\s+(class|series)\s+[a-z]\b.*$", "", text, flags=re.I)
    words = text.split()
    while words and re.sub(r"[^a-z]", "", words[-1].lower()) in _CORPORATE:
        words.pop()
    return clean_text(" ".join(words).strip(" ,"))


def _brand(name):
    """The words of a name that name the company: its search name less the corporate ones."""
    return [w for w in _words(search_name(name)) if w not in _CORPORATE]


def names_listing(headline, symbol, name, us=False):
    """Whether a headline names the listing, which is the only way a Google item is kept, since a
    search returns whatever mentions a name anywhere on a page:
    - its ticker in a venue's own form: `TSXV:QNC`, `CNSX:HG`, `PLTE:CA`, `CCHI.TO`, `(PLTE)`, `$QNC`
      (a Canadian venue's for a Canadian listing, a US one's for a US listing);
    - its ticker as a word in capitals, three letters or more, in a headline that is not all capitals;
    - its name as far as its second word that means something, written as a name (`Quantum eMotion`,
      `Bank of Montreal`, the last word possibly shortened: `CHAR Tech`; never `National Bank of Greece`
      for National Bank of Canada, nor `Canadian natural gas`);
    - its name's first word alone, six letters or more and not one many names start with, capitalised in
      a sentence-case headline (`Why Charbone shares jumped`) or, in a title-case one, opening it or styled
      as the company styles it (`Harvest ETFs Announces`, `MDI Joins HydroGraph`)."""
    head = _fold(headline)
    letters = re.findall(r"[A-Za-z]", head)
    mostly_caps = bool(letters) and sum(c.isupper() for c in letters) > 0.7 * len(letters)
    sym = market.tmx_symbol(symbol).upper()
    if sym:
        e = re.escape(sym)
        venues = "|".join(re.escape(v).replace(r"\ ", r"\s?") for v in (_US_VENUES if us else _CA_VENUES))
        suffixes = "" if us else "|".join(re.escape(x) for x in _CA_SUFFIXES)
        forms = [r"(?:%s)\s?:\s?%s" % (venues, e), r"\(%s\)" % e, r"\$%s" % e]
        if suffixes:
            forms.append(r"%s(?:%s)" % (e, suffixes))
        if re.search(r"(?<![A-Za-z0-9])(?:%s)(?![A-Za-z0-9])" % "|".join(forms), head, re.I):
            return True
        if len(sym) >= 3 and not mostly_caps and re.search(r"(?<![A-Za-z0-9.$])%s(?![A-Za-z0-9])" % e, head):
            return True
    brand = _brand(name)
    tokens = re.findall(r"[A-Za-z0-9]+", head)
    words = [t.lower() for t in tokens]
    named = lambda t: mostly_caps or any(c.isupper() for c in t)
    meaning = [i for i, w in enumerate(brand) if w not in _JOINERS]
    if len(meaning) >= 2:
        # the name as far as its second word that means something, written as a name: `Quantum eMotion`,
        # `CHAR Tech` (the last word possibly shortened), never `Canadian natural gas`; where the name goes
        # on with a joiner the headline shares, the word after it must be the name's too, so `National Bank
        # of Greece` is not `National Bank of Canada`
        prefix = brand[:meaning[1] + 1]
        n = len(prefix)
        for i in range(len(words) - n + 1):
            chunk = words[i:i + n]
            if chunk[:-1] != prefix[:-1] or not (chunk[-1] == prefix[-1] or (len(chunk[-1]) >= 4 and prefix[-1].startswith(chunk[-1]))):
                continue
            if not all(named(t) for t in tokens[i:i + n] if t.lower() not in _JOINERS):
                continue
            j, k = i + n, n
            while k < len(brand) and brand[k] in _JOINERS and j < len(words) and words[j] == brand[k]:
                j, k = j + 1, k + 1
            if k > n and (k >= len(brand) or j >= len(words) or words[j] != brand[k]):
                continue
            return True
    # the first word alone, where it is the company's own word and written as a name, and never a word many
    # names start with (`Canadian` dollar, `Global` stocks, `Quantum` computing). In a headline written in
    # sentence case a capital says so (`Why Charbone shares jumped`); in title case every word has one, so
    # there the word must open the headline (`Harvest ETFs Announces`) or be styled as the company styles it
    # (`MDI Joins HydroGraph`, `How CHARBONE Is Building`), never `How Canada's ETF Industry Continues to Evolve`
    if meaning and meaning[0] == 0:
        first = brand[0]
        if len(first) >= 6 and not first.isdigit() and first not in _GENERIC:
            long_words = [t for t in tokens if len(t) > 3 and t.isalpha()]
            title_case = len(long_words) >= 3 and sum(t[0].isupper() for t in long_words) > 0.6 * len(long_words)
            for i, t in enumerate(tokens):
                if t.lower() != first or not t[0].isupper():
                    continue
                if i == 0 or (any(c.isupper() for c in t[1:]) and not mostly_caps) or not (title_case or mostly_caps):
                    return True
    return False


def parse_google_news(xml, symbol, name, us=False):
    """Google's items for a search, each kept only where its headline names the listing and is
    not a quote site's page for it. Google's titles end in ` - Publisher`, which is taken off so
    the same story from another source is one row; a page Google dates before 2000 is not news."""
    rows = []
    for item in re.findall(r"<item>(.*?)</item>", _s(xml), re.S):
        title, source, link = _tag(item, "title"), _tag(item, "source"), _tag(item, "link")
        when = _iso(_tag(item, "pubDate"))
        if not (title and link and when):
            continue
        if source and title.endswith(" - " + source):
            title = title[: -len(" - " + source)].rstrip()
        if _QUOTE_PAGE.search(title) or not names_listing(title, symbol, name, us):
            continue
        rows.append({"id": "gnews:%s" % hashlib.sha1(link.encode("utf-8")).hexdigest()[:16], "headline": title,
                     "source": source or "Google News", "url": link, "publishedAt": when, "kind": kind_of(source), "via": "gnews"})
    return rows


def google_queries(symbol, exchange, currency, name=""):
    """What Google is asked for a listing: its name in quotes when the book has one, and its
    ticker in its venue's form (`"TSXV:CH"`, `"CSE:HG"`, `"NEO:HBIX"`)."""
    sym = market.tmx_symbol(symbol).upper()
    out = []
    clean = search_name(name)
    if clean and clean.upper() != sym:
        out.append('"%s"' % clean)
    form = market.tmx_form(exchange, currency)
    ex = _s(exchange).strip().upper()
    venue = {":CNX": "CSE", ":AQL": "NEO", "": "TSXV" if ex in ("TSX-V", "TSXV") else "TSX"}.get(form)
    if form == ":US":
        venue = "NYSE" if ex.startswith("NYSE") else "NASDAQ" if ex in ("NASDAQ", "") else None
    if sym and venue:
        out.append('"%s:%s"' % (venue, sym))
    return out


def fetch_google(symbol, exchange, currency, name="", ssl_context=None):
    """None when there is nothing to search for."""
    us = market.tmx_form(exchange, currency) == ":US"
    queries = google_queries(symbol, exchange, currency, name)
    if not queries:
        return None
    rows, seen = [], set()
    for query in queries:
        _pace("news.google.com", 1.5)
        xml = market._get_text(GNEWS_URL % urllib.parse.quote(query), ssl_context, headers=FEED_HEADERS)
        for r in parse_google_news(xml, symbol, name, us):
            if r["id"] not in seen:
                seen.add(r["id"])
                rows.append(r)
    return rows


# --- the merge ----------------------------------------------------------------------------------

# Beside the listing's wire (TMX's or Nasdaq's), read in this order; a later source's copy of a story
# an earlier one carries is the same row. Each is read at most this often per listing, Google and
# Seeking Alpha less often than the wire so neither is asked more than it tolerates.
EXTRA_SOURCES = ("yahoo", "sa", "gnews")
SOURCE_MINUTES = {"yahoo": FRESH_MINUTES, "sa": 30, "gnews": 30}


def sources_for(symbol, exchange, currency, name=""):
    """The sources beside the wire that have something to ask for a listing: Yahoo a ticker form,
    Seeking Alpha a feed, Google a search. A listing with no venue and no currency is left to the wire."""
    if symbol == MARKET[0] or market.tmx_form(exchange, currency) is None:
        return []
    have = {"yahoo": bool(yahoo_form(symbol, exchange, currency)), "sa": bool(sa_form(symbol, exchange, currency)),
            "gnews": bool(google_queries(symbol, exchange, currency, name))}
    return [k for k in EXTRA_SOURCES if have[k]]


def _read_extra(key, symbol, exchange, currency, name, ssl_context):
    if key == "yahoo":
        return fetch_yahoo(symbol, exchange, currency, name, ssl_context)
    if key == "sa":
        return fetch_sa(symbol, exchange, currency, ssl_context)
    return fetch_google(symbol, exchange, currency, name, ssl_context)


SAME_STORY_HOURS = 26     # one story's copies from several sources; a wire's day-only time can sit a day off another's


def same_story(when_a, when_b):
    """Two copies of one headline are one story when they were published within a day of each other; a
    trading halt, a resumption or a distribution notice repeats its title word for word months later."""
    try:
        a = datetime.fromisoformat(_s(when_a).replace("Z", "+00:00"))
        b = datetime.fromisoformat(_s(when_b).replace("Z", "+00:00"))
    except ValueError:
        return False
    return abs(a - b) <= timedelta(hours=SAME_STORY_HOURS)


def news_text(headline):
    """A headline as the same story reads under any source: lower case, punctuation and spacing gone."""
    return " ".join(_words(headline))


def origin(row_id):
    """Which source an item came from, by its id: `tmx`, `nasdaq`, `yahoo`, `sa` or `gnews`."""
    return _s(row_id).split(":", 1)[0]


def _stamp_key(source, symbol, exchange):
    return "news_source_fetched:%s:%s" % (source, store.news_key(symbol, exchange))


def _due(source, symbol, exchange, now):
    last = store.get_meta(_stamp_key(source, symbol, exchange))
    try:
        age = now - datetime.fromisoformat(last.replace("Z", "+00:00")) if last else None
    except ValueError:
        age = None
    return age is None or age > timedelta(minutes=SOURCE_MINUTES.get(source, FRESH_MINUTES))


def fetch_listing(symbol, exchange, currency, ssl_context=None, now=None, name="", force=False):
    """Every source's items for one listing, merged newest first: (wire, rows, sources asked).

    The wire and every source that is due are read at once. A source that fails, answers with
    nothing, or is not due this pass keeps the items it had stored, so its stories stay on the list
    until it answers again. Rows is None only when nothing answered, and the listing's stored news then stands as it
    was. One story carried by several sources is one row: the same headline keeps the copy of the
    first source in the order the wire, Yahoo, Seeking Alpha, Google, where the copies were published within
    a day of each other."""
    now = now or datetime.now(timezone.utc)
    if symbol == MARKET[0]:
        src, rows = fetch_symbol(symbol, exchange, currency, ssl_context, now)
        return src, rows, ({src} if rows is not None else set())
    extras = [k for k in sources_for(symbol, exchange, currency, name) if force or _due(k, symbol, exchange, now)]
    results = {}
    with ThreadPoolExecutor(max_workers=1 + len(extras)) as pool:
        wire = pool.submit(fetch_symbol, symbol, exchange, currency, ssl_context, now)
        jobs = {k: pool.submit(_read_extra, k, symbol, exchange, currency, name, ssl_context) for k in extras}
        src, primary = wire.result()
        for k, job in jobs.items():
            try:
                got = job.result()
                if got is not None:        # None: the source had nothing to ask for this listing
                    results[k] = got
            except Exception as e:
                sys.stderr.write("bagholder news: %s from %s failed: %s\n" % (symbol, k, str(e) or e.__class__.__name__))
    answered = set(results) | ({src} if primary is not None else set())
    if not answered:
        return src, None, answered
    answered |= set(extras)      # every source asked this pass waits its turn again, a failing one too
    stored, stored_feed = {}, {}
    for r in store.news_for(symbol, exchange):
        row = {"id": r["id"], "headline": r["headline"], "source": r["wire"], "url": r["url"], "publishedAt": r["publishedAt"],
               "kind": r["kind"], "via": r["source"] or origin(r["id"])}
        stored.setdefault(origin(r["id"]), []).append(row)
        stored_feed.setdefault(row["via"], []).append(row)
    merged, ids, texts = [], set(), {}
    def add(items):
        for r in items or []:
            text = news_text(r.get("headline"))
            if r["id"] in ids or any(same_story(r.get("publishedAt"), w) for w in texts.get(text, ()) if text):
                continue
            ids.add(r["id"])
            if text:
                texts.setdefault(text, []).append(r.get("publishedAt"))
            merged.append(r)
    # a source that answers with nothing for a listing it had items for has not lost its history: a
    # throttled or degraded answer reads that way, and its stored items stay until it answers again
    add(primary or stored.get("tmx", []) + stored.get("nasdaq", []))
    for feed in getattr(primary, "missing", ()):
        add(stored_feed.get(feed, []))
    for k in EXTRA_SOURCES:
        add(results.get(k) or stored.get(k, []))
    merged = [dict(r, via=r.get("via") or origin(r["id"])) for r in merged]
    merged.sort(key=lambda r: r.get("publishedAt") or "", reverse=True)
    return src, merged[:PER_LISTING], answered


def read_listing(symbol, exchange, currency, ssl_context=None, now=None, name="", force=False, on_new=None):
    """One listing's news read from every source and stored in place of what it had: (wire, rows),
    rows None when nothing answered. `on_new` is handed the rows and the ids the listing lacked."""
    now = now or datetime.now(timezone.utc)
    src, rows, answered = fetch_listing(symbol, exchange, currency, ssl_context, now, name=name, force=force)
    if rows is None:
        return src, None
    before, before_text, first_read = set(), {}, set()
    if on_new:
        # new is what the listing did not hold under any source: not an id it had, not a story it had under
        # another source's id, and nothing from a source read for the listing the first time, whose
        # back catalogue is history, as every stream's is when it is first met
        had = store.news_for(symbol, exchange)
        before = {r["id"] for r in had}
        for r in had:
            if news_text(r["headline"]):
                before_text.setdefault(news_text(r["headline"]), []).append(r["publishedAt"])
        first_read = {k for k in answered & set(EXTRA_SOURCES) if not store.get_meta(_stamp_key(k, symbol, exchange))}
    store.replace_news(symbol, exchange, src, rows, now=now)
    stamp = now.strftime("%Y-%m-%dT%H:%M:%SZ")
    for k in answered & set(EXTRA_SOURCES):
        store.set_meta(_stamp_key(k, symbol, exchange), stamp)
    if on_new:
        try:
            new_ids = {_s(r.get("id")) for r in rows
                       if _s(r.get("id")) not in before and origin(r.get("id")) not in first_read
                       and not any(same_story(r.get("publishedAt"), w) for w in before_text.get(news_text(r.get("headline")), ()))}
            on_new(symbol, exchange, rows, new_ids)
        except Exception as e:
            sys.stderr.write("bagholder news: %s items not told: %s\n" % (symbol, str(e) or e.__class__.__name__))
    return src, rows


class WireAnswer(list):
    """A wire's rows, and the feeds of it that failed or answered nothing this time (`tmx`, the press
    releases; `tmx-media`, In The Media; `nasdaq`; `nasdaq-press`), whose stored items stand in for them."""

    def __init__(self, rows=(), missing=()):
        super().__init__(rows)
        self.missing = set(missing)


def source_for(symbol, exchange, currency):
    """Which wire answers for a listing: TMX for the Canadian venues it carries, Nasdaq for US ones and for the market feed."""
    if (symbol, _s(exchange).upper()) == (MARKET[0], MARKET[1]):
        return "nasdaq"
    form = market.tmx_form(exchange, currency)
    if form == ":US":
        return "nasdaq"
    if form is None:
        return ""
    return "tmx"


def fetch_symbol(symbol, exchange, currency, ssl_context=None, now=None):
    """The latest items for one listing from its wire, as rows; [] when the wire has none or fails."""
    src = source_for(symbol, exchange, currency)
    sym = market.tmx_symbol(symbol)
    if not sym:
        return src, []
    if not src:
        # A ticker asked for with no venue at all is an ambiguous name: TMX's news answers on the bare
        # ticker whatever venue it is asked under, so `F` there is a Canadian company's halt notice and
        # not Ford's releases. Only Nasdaq is asked, whose items name the symbols they belong to and are
        # kept only when this one is among them, so nothing comes back rather than another company's news.
        src = "nasdaq"
    try:
        if symbol == MARKET[0]:
            _pace("api.nasdaq.com")
            text = market._get_text(NASDAQ_LATEST_URL % PER_MARKET, ssl_context, headers=NASDAQ_HEADERS)
            return src, parse_nasdaq_news(json.loads(text), now, "")
        if src == "tmx":
            # TMX names a listing by its venue, and the news query answers nothing under the wrong
            # name: the same code the quote asks under, through the same lookup, so a record with a
            # wrong or missing venue resolves here as it does everywhere else and is remembered once
            code = market.tmx_quote_symbol(symbol, exchange, currency)
            if not code:
                return src, []
            def ask(form):
                # both tabs: the press releases, then the stories publishers wrote about the company.
                # A tab that fails is left out rather than failing the other one, and named, so the
                # stories it had stored stand in for it.
                rows, missing = [], set()
                for media in (False, True):
                    _pace("app-money.tmx.com")
                    try:
                        data = market._post_json("https://app-money.tmx.com/graphql",
                                                 {"operationName": "getNewsForSymbol",
                                                  "variables": {"symbol": form, "page": 1, "limit": PER_SYMBOL, "locale": "en", "companyInNews": media},
                                                  "query": TMX_NEWS_QUERY},
                                                 ssl_context, TMX_HEADERS)
                    except Exception as e:
                        if not media:
                            raise
                        sys.stderr.write("bagholder news: %s stories from tmx failed: %s\n" % (form, e))
                        missing.add("tmx-media")
                        continue
                    got = parse_tmx_news(data, form, media=media)
                    if not got:
                        missing.add("tmx-media" if media else "tmx")
                    rows.extend(got)
                return WireAnswer(rows, missing)
            return src, market.tmx_lookup(code, ask, ssl_context)[0]
        _pace("api.nasdaq.com")
        text = market._get_text(NASDAQ_NEWS_URL % (sym, PER_SYMBOL), ssl_context, headers=NASDAQ_HEADERS)
        rows = WireAnswer(dict(r, via="nasdaq") for r in parse_nasdaq_news(json.loads(text), now, sym))
        if not rows:
            rows.missing.add("nasdaq")
        # the listing's own releases come on a feed of their own; each once, beside the stories
        try:
            _pace("api.nasdaq.com")
            text = market._get_text(NASDAQ_PRESS_URL % (sym, PER_SYMBOL), ssl_context, headers=NASDAQ_HEADERS)
            seen = {r["id"] for r in rows}
            press = [dict(r, via="nasdaq-press") for r in parse_nasdaq_news(json.loads(text), now, sym, kind="release") if r["id"] not in seen]
            if not press:
                rows.missing.add("nasdaq-press")
            rows.extend(press)
        except Exception as e:
            rows.missing.add("nasdaq-press")
            sys.stderr.write("bagholder news: %s releases from nasdaq failed: %s\n" % (sym, e))
        return src, rows
    except Exception as e:
        sys.stderr.write("bagholder news: %s from %s failed: %s\n" % (sym, src, e))
        return src, None


def stale(listings, now=None, minutes=FRESH_MINUTES):
    """The listings with a source to read, as (symbol, exchange, currency, name): the wire older than
    `minutes`, or any source beside it that has something to ask and is due. Freshness is each
    source's own: a listing whose wire was just read by a copy of the app that did not ask the other
    sources, or by a pass where one of them was not yet due, still has them to read."""
    now = now or datetime.now(timezone.utc)
    fetched = store.news_fetched_at()
    out = []
    for listing in listings:
        symbol, exchange, currency, name = (tuple(listing) + ("",))[:4]
        last = fetched.get(store.news_key(symbol, exchange)) or ""
        try:
            age = now - datetime.fromisoformat(last.replace("Z", "+00:00")) if last else None
        except ValueError:
            age = None
        if age is None or age > timedelta(minutes=minutes) or any(_due(k, symbol, exchange, now) for k in sources_for(symbol, exchange, currency, name)):
            out.append((symbol, exchange, currency, name))
    return out


def refresh(listings, ssl_context=None, now=None, on_new=None, on_start=None, on_done=None, at_once=LISTINGS_AT_ONCE):
    """Read every source for every stale listing, a few listings side by side; each answer replaces
    that listing's rows. Returns how many answered. `on_new(symbol, exchange, rows, new_ids)` is handed
    everything the listing's sources answered with and the ids it did not have before; what is worth
    telling about is the notifier's to decide. `on_start(listings)` is told what the pass will read and
    `on_done(listing, answered)` each listing as it lands, so a page can say a read is under way and
    show each listing's items as they arrive rather than at the end of the pass."""
    due = stale(listings, now=now)
    if on_start:
        on_start(due)

    def one(listing):
        symbol, exchange, currency, name = (tuple(listing) + ("",))[:4]
        rows = None
        try:
            _, rows = read_listing(symbol, exchange, currency, ssl_context, now, name=name, on_new=on_new)
        except Exception as e:
            sys.stderr.write("bagholder news: %s read failed: %s\n" % (symbol, str(e) or e.__class__.__name__))
        finally:
            if on_done:
                try:
                    on_done(listing, rows is not None)
                except Exception:
                    pass
        return rows is not None

    if not due:
        return 0
    with ThreadPoolExecutor(max_workers=max(1, min(at_once, len(due)))) as pool:
        done = sum(1 for ok in pool.map(one, due) if ok)
    if done:
        store.trim_news(KEEP)
    return done
