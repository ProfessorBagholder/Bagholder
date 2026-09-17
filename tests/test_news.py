"""News: every per-symbol source parsed into rows, merged and kept per listing, tagged in the model."""
from __future__ import annotations

import os
import tempfile
import unittest
from datetime import datetime, timedelta, timezone
from unittest import mock

import bagholder
import model
import market
import news
import store


class ParseTest(unittest.TestCase):
    def test_tmx_items_carry_an_exact_time_and_a_page_link(self):
        data = {"data": {"news": [{"headline": "Shopify Delivers Big: 30%+ Growth Across&#xA0;GMV", "datetime": "2026-08-05T07:00:00-04:00", "source": "GlobeNewswire via QuoteMedia", "newsid": 4883675477075330},
                                  {"headline": "no id", "datetime": "2026-08-05T07:00:00-04:00"},
                                  {"headline": "bad time", "datetime": "yesterday", "newsid": 5}]}}
        rows = news.parse_tmx_news(data, "SHOP")
        self.assertEqual(rows, [{"id": "tmx:4883675477075330", "headline": "Shopify Delivers Big: 30%+ Growth Across GMV", "source": "GlobeNewswire",
                                 "url": "https://money.tmx.com/en/quote/SHOP/news/4883675477075330", "publishedAt": "2026-08-05T11:00:00Z", "kind": "release", "via": "tmx"}])

    def test_nasdaq_items_take_their_time_from_the_age_given(self):
        now = datetime(2026, 9, 11, 15, 30, tzinfo=timezone.utc)
        data = {"data": {"rows": [{"id": 28351741, "title": "Forget AMD. Here&#39;s Who Nvidia Really Needs to Be Worried About.", "publisher": "The Motley Fool", "created": "Sep 11, 2026", "ago": "17 minutes ago", "url": "/articles/forget-amd", "primarysymbol": "avgo", "related_symbols": ["avgo|stocks", "nvda|stocks"]},
                                  {"id": 2, "title": "Two hours", "publisher": "Zacks", "created": "Sep 11, 2026", "ago": "2 hours ago", "url": "https://www.nasdaq.com/articles/two", "related_symbols": ["NVDA|stocks"]},
                                  {"id": 3, "title": "Old", "publisher": "Barchart", "created": "Sep 3, 2026", "ago": "", "url": "/articles/old", "primarysymbol": "nvda"},
                                  {"id": 4, "publisher": "no title", "related_symbols": ["nvda|stocks"]},
                                  {"id": 5, "title": "Market wrap that never names it", "publisher": "Barchart", "created": "Sep 11, 2026", "ago": "3 minutes ago", "url": "/articles/wrap", "related_symbols": ["spy|etf", "aapl|stocks"]}]}}
        rows = news.parse_nasdaq_news(data, now, "NVDA")
        self.assertEqual([(r["id"], r["headline"], r["source"], r["url"], r["publishedAt"]) for r in rows],
                         [("nasdaq:28351741", "Forget AMD. Here's Who Nvidia Really Needs to Be Worried About.", "The Motley Fool", "https://www.nasdaq.com/articles/forget-amd", "2026-09-11T15:13:00Z"),
                          ("nasdaq:2", "Two hours", "Zacks", "https://www.nasdaq.com/articles/two", "2026-09-11T13:30:00Z"),
                          ("nasdaq:3", "Old", "Barchart", "https://www.nasdaq.com/articles/old", "2026-09-03T00:00:00Z")],
                         "an item Nasdaq does not tag with the symbol is left out")

    def test_the_wire_follows_the_venue(self):
        self.assertEqual(news.source_for("SHOP", "TSX", "CAD"), "tmx")
        self.assertEqual(news.source_for("NVDA", "NASDAQ", "USD"), "nasdaq")
        self.assertEqual(news.source_for("AAPL", "", "USD"), "nasdaq")
        self.assertEqual(news.source_for("QBTC", "NEO", "CAD"), "tmx")


class KindTest(unittest.TestCase):
    # its own store, as every class that reaches one has: without it this class wrote its fixtures
    # into the person's live database and read their remembered TMX forms back as if they were its own
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        os.environ["BAGHOLDER_HOME"] = self.tmp.name
        store.set_home(self.tmp.name)
        bagholder.set_home(self.tmp.name)
        store.ensure()

    def tearDown(self):
        self.tmp.cleanup()
        os.environ.pop("BAGHOLDER_HOME", None)

    def test_a_wires_item_is_a_release_and_a_publishers_a_story(self):
        for wire in ("GlobeNewswire", "Business Wire", "PR Newswire", "ACCESS Newswire", "Accesswire", "TheNewsWire", "Canada Newswire", "TMX Newsfile", "Marketwired", "CNW Group", "NewMediaWire"):
            self.assertEqual(news.kind_of(wire), "release", wire)
        for pub in ("The Motley Fool", "Zacks", "Barchart", "RTTNews", "MarketBeat", "BNK Invest", "Fintel", "", "WIRED", "MT Newswires", "Dow Jones Newswires"):
            self.assertEqual(news.kind_of(pub), "story", pub)
        tmx = news.parse_tmx_news({"data": {"news": [{"newsid": "1", "headline": "Closing", "source": "GlobeNewswire via QuoteMedia", "datetime": "2026-09-14T08:00:00-04:00"}]}}, "CH")
        self.assertEqual((tmx[0]["kind"], tmx[0]["source"]), ("release", "GlobeNewswire"))
        now = datetime(2026, 9, 15, 12, 0, tzinfo=timezone.utc)
        press = news.parse_nasdaq_news({"data": {"rows": [{"id": 9, "title": "Shopify Delivers Big", "publisher": "", "created": "Aug 5, 2026", "ago": "Aug 5, 2026", "url": "/press-release/x", "related_symbols": ["shop|stocks"]}]}}, now, "SHOP", kind="release")
        self.assertEqual((press[0]["kind"], press[0]["source"], press[0]["publishedAt"]), ("release", "Nasdaq", "2026-08-05T00:00:00Z"), "a release Nasdaq names no wire for reads as Nasdaq's")
        story = news.parse_nasdaq_news({"data": {"rows": [{"id": 8, "title": "Why SHOP", "publisher": "The Motley Fool", "created": "Sep 14, 2026", "ago": "1 day ago", "url": "/articles/y", "related_symbols": ["shop|stocks"]}]}}, now, "SHOP")
        self.assertEqual(story[0]["kind"], "story")

    def test_tmx_is_asked_under_the_code_the_quote_uses_and_resolves_a_wrong_venue(self):
        item = {"newsid": "7", "headline": "QIMC Engages Echo Seismic", "source": "TMX Newsfile via QuoteMedia", "datetime": "2026-09-14T09:13:00-04:00"}
        asked = []
        def post_json(url, body, ctx, headers=None, **kw):
            form = body["variables"]["symbol"]
            asked.append(form)
            return {"data": {"news": [item]}} if form in ("QIMC:CNX", "CH") else {"data": {"news": []}}
        with mock.patch.object(market, "_post_json", side_effect=post_json), mock.patch.object(news, "_pace"):
            src, rows = news.fetch_symbol("QIMC", "CSE", "CAD")
            news.fetch_symbol("CH", "TSX-V", "CAD")
            self.assertEqual((src, asked), ("tmx", ["QIMC:CNX", "QIMC:CNX", "CH", "CH"]),
                             "each listing under the code its quote uses, once for each of TMX's two tabs")
            self.assertEqual((rows[0]["kind"], rows[0]["url"]), ("release", "https://money.tmx.com/en/quote/QIMC:CNX/news/7"))
            # the record names the wrong venue: the lookup resolves the form that answers, as it does for a quote
            asked.clear()
            with mock.patch.object(market, "tmx_resolve", return_value="QIMC:CNX"):
                _, found = news.fetch_symbol("QIMC", "TSX-V", "CAD")
            self.assertEqual((asked, [r["id"] for r in found]), (["QIMC", "QIMC", "QIMC:CNX", "QIMC:CNX"], ["tmx:7"]))

    def test_tmx_names_a_listing_by_its_own_topic_codes(self):
        topic = "[ABHI:AQL,ABHI:CA,ART00001,CCHI:AQL,CCHI:CA,DIVIDEND]"
        self.assertTrue(news.tmx_names(topic, "CCHI"))
        self.assertTrue(news.tmx_names("[HG:CNX,MINING01]", "HG:CNX"))
        self.assertTrue(news.tmx_names("[ASTS,SPACE001]", "ASTS:US"), "a US listing's code is its bare ticker")
        self.assertFalse(news.tmx_names(topic, "CCH"), "a code is a whole ticker, not a prefix of one")
        self.assertFalse(news.tmx_names("[T,VZ,TMUS]", "T"), "AT&T's bare code is not Telus, a Canadian listing named `T:CA`")
        self.assertTrue(news.tmx_names("[T:CA,BCE:CA]", "T"))
        self.assertFalse(news.tmx_names("[HG,INSURE01]", "HG:CNX"), "the NYSE's HG is not the CSE's")
        self.assertFalse(news.tmx_names("[ASTS:CA]", "ASTS:US"))
        self.assertFalse(news.tmx_names("", "PNG"))

    def test_a_publishers_story_is_kept_only_where_tmx_tags_the_listing(self):
        data = {"data": {"news": [
            {"newsid": "1", "headline": "Kraken Robotics: Undersea Batteries Drive Solid Revenue Growth", "source": "SeekingAlpha via QuoteMedia",
             "datetime": "2026-09-05T10:00:00-04:00", "topic": "[PNG:CA,TECH0001]"},
            {"newsid": "2", "headline": "Most shorted stocks on Wall Street", "source": "SeekingAlpha via QuoteMedia",
             "datetime": "2026-09-05T11:00:00-04:00", "topic": "[ASTS,NBIS]"}]}}
        rows = news.parse_tmx_news(data, "PNG", media=True)
        self.assertEqual([(r["id"], r["kind"], r["source"]) for r in rows], [("tmx:1", "story", "SeekingAlpha")],
                         "a story TMX tags with another listing is not this one's")
        wire = news.parse_tmx_news({"data": {"news": [{"newsid": "3", "headline": "Kraken closes financing", "source": "GlobeNewswire via QuoteMedia",
                                                       "datetime": "2026-09-05T08:00:00-04:00"}]}}, "PNG")
        self.assertEqual(wire[0]["kind"], "release", "the press releases tab reads as it always did")

    def test_both_of_tmxs_tabs_are_read_and_a_failing_stories_tab_keeps_the_releases(self):
        release = {"newsid": "10", "headline": "Kraken closes financing", "source": "GlobeNewswire via QuoteMedia", "datetime": "2026-09-05T08:00:00-04:00", "topic": "[PNG:CA]"}
        story = {"newsid": "11", "headline": "3 Top Canadian Defence Stocks", "source": "Motley Fool Canada via QuoteMedia", "datetime": "2026-09-02T09:00:00-04:00", "topic": "[PNG:CA,DEFENCE1]"}
        tabs = []
        def post_json(url, body, ctx, headers=None, **kw):
            media = body["variables"].get("companyInNews")
            tabs.append(media)
            return {"data": {"news": [story if media else release]}}
        with mock.patch.object(market, "_post_json", side_effect=post_json), mock.patch.object(news, "_pace"), \
             mock.patch.object(market, "tmx_quote_symbol", return_value="PNG"):
            src, rows = news.fetch_symbol("PNG", "TSX-V", "CAD")
        self.assertEqual((src, tabs), ("tmx", [False, True]))
        self.assertEqual(sorted((r["id"], r["kind"]) for r in rows), [("tmx:10", "release"), ("tmx:11", "story")])
        def failing_stories(url, body, ctx, headers=None, **kw):
            if body["variables"].get("companyInNews"):
                raise OSError("down")
            return {"data": {"news": [release]}}
        with mock.patch.object(market, "_post_json", side_effect=failing_stories), mock.patch.object(news, "_pace"), \
             mock.patch.object(market, "tmx_quote_symbol", return_value="PNG"), mock.patch.object(news.sys, "stderr"):
            _, rows = news.fetch_symbol("PNG", "TSX-V", "CAD")
        self.assertEqual([r["id"] for r in rows], ["tmx:10"], "the releases still arrive when the stories tab fails")

    def test_a_ticker_the_app_has_never_seen_is_placed_before_a_wire_is_asked(self):
        """No directory carries every venue, so the venue comes from the app's own knowledge: the
        security records the sync brought, then TMX's resolver, which names the venue it verified
        by the quote. A ticker TMX cannot place is a US one."""
        seen = {}
        def get_text(url, ctx, headers=None, **kw):
            seen.setdefault("nasdaq", []).append(url)
            return '{"data": {"rows": []}}'
        def post_json(url, body, ctx, headers=None, **kw):
            seen["tmx"] = body["variables"]["symbol"]
            return {"data": {"news": [{"newsid": "3", "headline": "QIMC Engages", "source": "TMX Newsfile", "datetime": "2026-09-14T09:13:00-04:00"}]}}
        with mock.patch.object(news, "_pace"), mock.patch.object(market, "_get_text", side_effect=get_text), \
             mock.patch.object(market, "_post_json", side_effect=post_json), mock.patch.object(store, "list_securities", return_value=[]), \
             mock.patch.object(news, "_read_extra", return_value=[]), mock.patch.object(market, "tmx_listing", return_value=None):
            # a CSE listing no directory carries: TMX's resolver places it and the news is read under that form
            with mock.patch.object(market, "tmx_resolve", return_value="QIMC:CNX"), mock.patch.object(market, "tmx_remembered", side_effect=lambda k: "QIMC:CNX"):
                out = bagholder.news_symbol_payload("QIMC", "", "")
            self.assertEqual((out["source"], seen.get("tmx")), ("tmx", "QIMC:CNX"))
            seen.clear()
            # TMX cannot place it: Nasdaq, whose items name the symbols they belong to
            with mock.patch.object(market, "tmx_resolve", return_value=""):
                out = bagholder.news_symbol_payload("KO", "", "")
        self.assertEqual((out["source"], out["exchange"], "tmx" in seen), ("nasdaq", "NASDAQ", False))

    def test_a_ticker_with_no_venue_is_never_asked_of_tmx(self):
        """TMX's news answers on the bare ticker whatever venue it is asked under, so a name with no
        venue would come back as another company's. Only Nasdaq, whose items name their symbols."""
        seen = {}
        def post_json(url, body, ctx, headers=None, **kw):
            seen["tmx"] = body["variables"]["symbol"]
            return {"data": {"news": [{"newsid": "9", "headline": "IIROC Trading Halt - F", "source": "TMX Newsfile", "datetime": "2026-09-14T09:13:00-04:00"}]}}
        def get_text(url, ctx, headers=None, **kw):
            seen.setdefault("nasdaq", []).append(url)
            return '{"data": {"rows": [{"id": 5, "title": "Ford declares dividend", "publisher": "PR Newswire", "created": "Sep 14, 2026", "ago": "1 day ago", "url": "/a", "related_symbols": ["f|stocks"]}]}}'
        with mock.patch.object(news, "_pace"), mock.patch.object(market, "_post_json", side_effect=post_json), mock.patch.object(market, "_get_text", side_effect=get_text):
            src, rows = news.fetch_symbol("F", "", "")
        self.assertEqual((src, "tmx" in seen), ("nasdaq", False), "no venue: TMX is never asked")
        self.assertEqual([(r["kind"], r["headline"]) for r in rows], [("release", "Ford declares dividend")])

    def test_a_us_listing_reads_its_releases_beside_its_news_each_once(self):
        now = datetime(2026, 9, 15, 12, 0, tzinfo=timezone.utc)
        feeds = {"articlebysymbol": '{"data": {"rows": [{"id": 1, "title": "Why SHOP", "publisher": "Zacks", "created": "Sep 14, 2026", "ago": "1 day ago", "url": "/articles/a", "related_symbols": ["shop|stocks"]}, {"id": 2, "title": "Shopify Delivers Big", "publisher": "GlobeNewswire", "created": "Aug 5, 2026", "ago": "Aug 5, 2026", "url": "/articles/b", "related_symbols": ["shop|stocks"]}]}}',
                 "press_release": '{"data": {"rows": [{"id": 2, "title": "Shopify Delivers Big", "publisher": "", "created": "Aug 5, 2026", "ago": "Aug 5, 2026", "url": "/press-release/b", "related_symbols": ["shop|stocks"]}, {"id": 3, "title": "Shopify to Announce", "publisher": "", "created": "Jul 8, 2026", "ago": "Jul 8, 2026", "url": "/press-release/c", "related_symbols": ["shop|stocks"]}]}}'}
        asked = []
        def get_text(url, ctx, headers=None, **kw):
            asked.append(url)
            return feeds["press_release" if "press_release" in url else "articlebysymbol"]
        with mock.patch.object(market, "_get_text", side_effect=get_text), mock.patch.object(news, "_pace"):
            src, rows = news.fetch_symbol("SHOP", "NASDAQ", "USD", now=now)
        self.assertEqual(src, "nasdaq")
        self.assertEqual([(r["id"], r["kind"], r["source"]) for r in rows], [("nasdaq:1", "story", "Zacks"), ("nasdaq:2", "release", "GlobeNewswire"), ("nasdaq:3", "release", "Nasdaq")], "the news feed's own wire item is a release; the press feed adds what the news feed lacks, each once")
        self.assertEqual(len(asked), 2)


class SourcesTest(unittest.TestCase):
    """Yahoo's gateway, Seeking Alpha's feed and Google News beside the wire: each item kept only
    where its source names the listing, the stories merged into one list per listing."""

    def test_yahoo_keeps_what_its_ticker_tags_name(self):
        asset = lambda uuid, title, tickers, provider="Newsfile", when="2026-09-14T13:13:00Z": {"node": {"asset": {
            "id": uuid, "title": title, "contentAttributes": {"pubDate": when, "provider": {"displayName": provider}, "canonicalUrl": "https://finance.yahoo.com/news/" + uuid},
            "finance": {"stockTickers": [{"symbol": t} for t in tickers]}}}}
        data = {"data": {"lightyearList": {"main": {"edges": [
            asset("a1", "Kraken Robotics Announces Q2 Results", ["PNG.V", "KRKNF"]),
            asset("a2", "3 Defence Stocks To Watch", ["LMT", "RTX"], provider="Motley Fool"),
            asset("a3", "Kraken Wins Navy Contract", ["PNG.V"], provider="The Globe and Mail", when="2026-09-15T10:00:00.000Z"),
            asset("a4", "no date", ["PNG.V"], when="")]}}}}
        rows = news.parse_yahoo_news(data, "PNG.V")
        self.assertEqual([(r["id"], r["kind"], r["source"], r["publishedAt"]) for r in rows],
                         [("yahoo:a1", "release", "Newsfile", "2026-09-14T13:13:00Z"), ("yahoo:a3", "story", "The Globe and Mail", "2026-09-15T10:00:00Z")],
                         "an item Yahoo tags with other tickers is theirs; a wire's item is a release")
        with mock.patch.object(market, "tmx_remembered", side_effect=lambda k: k + ":CNX" if k == "QIMC" else k):
            self.assertEqual([news.yahoo_form(*x) for x in (("PNG", "TSX-V", "CAD"), ("HG", "CSE", "CAD"), ("HBIX", "Cboe Canada", "CAD"), ("ASTS", "NASDAQ", "USD"),
                                                            ("LUNR", "NASDAQ", ""), ("QIMC", "", "CAD"), ("VEQT", "", "CAD"), ("F", "", ""))],
                             ["PNG.V", "HG.CN", "HBIX.NE", "ASTS", "LUNR", "QIMC.CN", "VEQT.TO", ""],
                             "the venue decides before the currency: a US listing with no currency is not `LUNR.TO`, another company")

    def test_yahoo_keeps_a_canadian_companys_items_tagged_with_its_us_twin(self):
        asset = lambda uuid, title, tickers: {"node": {"asset": {"id": uuid, "title": title, "finance": {"stockTickers": [{"symbol": t} for t in tickers]},
                                                                 "contentAttributes": {"pubDate": "2026-09-14T13:13:00Z", "provider": {"displayName": "PR Newswire"}}}}}
        data = {"data": {"lightyearList": {"main": {"edges": [
            asset("a1", "CHARBONE Announces Closing of $1.5M Drawdown", ["CH.V", "CHHYF"]),
            asset("a2", "Charbone Announces Its First Hydrogen Supply Hub", ["CHHYF"]),
            asset("a3", "ESGFIRE Initiates Coverage on Charbone Corporation", ["CHHYF", "PLUG", "FCEL"]),
            asset("a4", "Presenting on Emerging Growth Conference 90 Day 1", ["ASPI", "IBX.AX", "STLNF"]),
            asset("a5", "CHARBONE to Present at the Hydrogen East Conference", []),
            asset("a6", "Hydrogen prices climb", [])]}}}}
        self.assertEqual([r["id"] for r in news.parse_yahoo_news(data, "CH.V", "CH", "Charbone Hydrogen Corp")], ["yahoo:a1", "yahoo:a2", "yahoo:a3", "yahoo:a5"],
                         "the twin Yahoo tags beside the listing names it; an untagged item counts where its headline names the listing")
        partner = {"data": {"lightyearList": {"main": {"edges": [
            asset("p1", "Kraken and Saab sign sonar partnership", ["PNG.V", "KRKNF", "SAABF"]),
            asset("p2", "Kraken Robotics orders", ["PNG.V", "KRKNF"]),
            asset("p3", "Saab raises its outlook", ["SAABF"])]}}}}
        self.assertEqual([r["id"] for r in news.parse_yahoo_news(partner, "PNG.V", "PNG", "Kraken Robotics Inc.")], ["yahoo:p1", "yahoo:p2"],
                         "a partner's symbol on an item naming several is not the listing's twin")
        us = {"data": {"lightyearList": {"main": {"edges": [asset("b1", "Palantir wins Army deal", ["PLTR"]), asset("b2", "Kraken Robotics orders", ["KRKNF"])]}}}}
        self.assertEqual([r["id"] for r in news.parse_yahoo_news(us, "PLTR", "PLTR", "Palantir Technologies Inc")], ["yahoo:b1"], "a US listing has no twin")

    def test_seeking_alpha_keeps_what_its_symbol_tags_name(self):
        xml = """<rss><channel>
          <item><title>Kraken Robotics: Undersea Batteries Drive Growth</title><link>https://seekingalpha.com/article/1</link>
            <guid isPermaLink="false">Article:1</guid><pubDate>Fri, 05 Sep 2026 10:00:00 -0400</pubDate><sa:symbol>PNG:CA</sa:symbol><sa:symbol>KRKNF</sa:symbol></item>
          <item><title>Most shorted stocks</title><link>https://seekingalpha.com/news/2</link><guid>MarketCurrent:2</guid>
            <pubDate>Fri, 05 Sep 2026 11:00:00 -0400</pubDate><sa:symbol>ASTS</sa:symbol></item>
        </channel></rss>"""
        rows = news.parse_sa_news(xml, "PNG:CA")
        self.assertEqual([(r["headline"], r["source"], r["url"], r["publishedAt"]) for r in rows],
                         [("Kraken Robotics: Undersea Batteries Drive Growth", "Seeking Alpha", "https://seekingalpha.com/article/1", "2026-09-05T14:00:00Z")])
        self.assertEqual([news.sa_form(*x) for x in (("PNG", "TSX-V", "CAD"), ("VEQT", "TSX", "CAD"), ("ASTS", "NASDAQ", "USD"), ("HG", "CSE", "CAD"), ("HBIX", "Cboe Canada", "CAD"))],
                         ["PNG:CA", "VEQT:CA", "ASTS", "", ""], "Seeking Alpha has no form for the CSE or Cboe Canada")

    def test_a_name_is_searched_as_the_press_writes_it(self):
        cases = {"Harvest Reddit Enhanced High Income Shares ETF (the “ETF”)": "Harvest Reddit Enhanced High Income Shares ETF",
                 "Harvest Diversified High Income Shares ETF - Class A": "Harvest Diversified High Income Shares ETF",
                 "Ninepoint Partners LP - Cameco Highshares ETF": "Ninepoint Cameco Highshares ETF",
                 "Vanguard All-Equity ETF Portfolio - ETF": "Vanguard All-Equity ETF Portfolio",
                 "Palantir Technologies Inc (Class A)": "Palantir Technologies",
                 "Nebius Group N.V. Class A": "Nebius",
                 "Micron Technology, Inc.": "Micron Technology",
                 "Charbone Hydrogen Corp": "Charbone Hydrogen",
                 "": ""}
        for raw, want in cases.items():
            self.assertEqual(news.search_name(raw), want, raw)
        self.assertEqual(news.google_queries("HG", "CSE", "CAD", "Hydrograph Clean Power Inc."), ['"Hydrograph Clean Power"', '"CSE:HG"'])
        self.assertEqual(news.google_queries("CH", "TSX-V", "CAD", "Charbone Hydrogen Corp"), ['"Charbone Hydrogen"', '"TSXV:CH"'])
        self.assertEqual(news.google_queries("HBIX", "Cboe Canada", "CAD", ""), ['"NEO:HBIX"'], "no name: the ticker alone")
        self.assertEqual(news.google_queries("MU", "NASDAQ", "USD", "Micron Technology, Inc."), ['"Micron Technology"', '"NASDAQ:MU"'])

    def test_google_keeps_a_headline_only_where_it_names_the_listing(self):
        # headlines Google News returned for the searches, each with what it must be
        cases = [
            ("PLTE", "Harvest Palantir Enhanced High Income Shares ETF - Class A", False, [
                ("(PLTE) Equity Market Report (PLTE:CA)", True),
                ("Canadian ETF Express | Harvest Palantir Enhanced High Income Shares ETF Was the Top Gainer, Rising 32.29%", True),
                ("Harvest High Income Shares ETFs Announces August 2026 Distributions", True),
                ("The Ultimate Investor Guide to High-Income TSX ETFs Generating Monthly Cash Flow", False),
                ("Canadian ETF Express | GLOBAL X INVESTMENTS CANADA INC. BETAPRO NATURAL GAS LEVERAGED DAILY BULL Was the Top Gainer, Rising 3.65%", False)]),
            ("CH", "Charbone Hydrogen Corp", False, [
                ("CHARBONE Announces Change of Corporate Name and Registered Address", True),
                ("Charbone Reports Q2 2026 Financial Results, Confirming 155% Gas Income Growth", True),
                ("Boeing Announces Second Quarter Deliveries", False)]),
            ("HG", "Hydrograph Clean Power Inc.", False, [
                ("HydroGraph Announces Change of Auditor", True),
                ("Is HydroGraph Clean Power (CNSX:HG) Fully Valued After Wider Losses And Fresh Funding?", True),
                ("HydroGraph Clean Power (HG.C): A year ago this thing looked insane. Then it got bigger.", True),
                ("Widespread intensification of global river hydrograph flashiness under climate change", False)]),
            ("YES", "Char Technologies Ltd.", False, [
                ("CHAR Tech Receives Patent Notice of Allowance for Pyrogas Treatment to Syngas", True),
                ("CHAR Technologies Ltd. (CVE:YES): Are Analysts Optimistic?", True),
                ("UW Works with Wyoming DEQ-AML, UR Energy on Soil Reclamation Project Using Coal Char", False),
                ("Canada’s Energy Trade Is Alive Again — Why NG Energy International Corp (TSXV:GASX) Matters Now", False)]),
            ("QIMC", "Quebec Innovative Materials Corp", False, [
                ("Québec Innovative Materials Corp. Engages Echo Seismic and Strum Consulting", True),
                ("QIMC launches 78-km natural hydrogen survey", True),
                ("The hunt is on for natural 'white' hydrogen in Nova Scotia’s underground", False)]),
            ("SXHI", "Ninepoint SpaceX HighShares ETF", False, [
                ("SXHI: SpaceX High-Income ETF's 9.17% Screener Yield Puts Private-Space Exposure in the Spotlight", True),
                ("Ninepoint Partners Announces June 2026 Cash Distributions", True),
                ("Retail investors can now buy Canadian and US IPOs at offering price", False)]),
            ("EASY", "Evolve All-in-One UltraYield ETF", False, [
                ("EASY WAYS TO RETIRE EARLY", False),
                ("Evolve Sets September 2026 Distributions Across UltraYield ETFs and Income Funds", True)]),
            ("QNC", "Quantum Emotion Corp", False, [
                ("Quantum eMotion Submits Quantum Entropy Source for NIST Validation", True),
                ("$Xanadu Quantum Technologies (XNDU.US)$", False),
                ("Why investors are watching Quantum stocks", False)]),
            ("HG", "Hydrograph Clean Power Inc.", False, [
                ("MDI joins HydroGraph partner network", True),
                ("Sparc reports positive results using HydroGraph's Fractal Graphene in solvent-based coatings", True)]),
            ("CH", "Charbone Hydrogen Corp", False, [
                ("The Supply Gap No One Is Filling: How CHARBONE Is Building the UHP Industrial Gas Platform", True)]),
            ("VEQT", "Vanguard All-Equity ETF Portfolio - ETF", False, [
                ("Vanguard Investments Canada Announces Final 2025 Annual Capital Gains Distributions for the Vanguard ETFs", True),
                ("15 cheap, but well-rated ETFs", False),
                ("No Time to Invest? Buy Any of These 3 Vanguard ETF Portfolios to Be Set for Life", False)]),
            ("NA", "National Bank of Canada", False, [
                ("National Bank of Canada Reports Record Quarter", True),
                ("National Bank Financial raises its target on Cameco", True),
                ("National Bank of Greece posts record profit", False)]),
            ("RY", "Royal Bank of Canada", False, [
                ("Royal Bank of Scotland to cut jobs", False)]),
            ("EASY", "Evolve All-in-One UltraYield ETF", False, [
                ("How Canada's ETF Industry Continues to Evolve", False)]),
            ("HHIS", "Harvest Diversified High Income Shares ETF - Class A", False, [
                ("Investors Rush to Harvest Tax Losses Before Year End", False),
                ("Harvest ETFs Announces August 2026 Distributions", True)]),
            ("CH", "Charbone Hydrogen Corp", False, [
                ("Why Charbone shares jumped 30%", True)]),
            ("BMO", "Bank of Montreal", False, [
                ("Bank of Montreal Reports Third Quarter Results", True),
                ("Bank of Canada holds rates steady", False)]),
            ("CNQ", "Canadian Natural Resources Limited", False, [
                ("Canadian Natural Resources to buy oil sands assets", True),
                ("Canadian dollar weakens as oil slides", False),
                ("Canadian natural gas prices slump", False),
                ("Canadian stocks close higher", False)]),
            ("HXS", "Global X S&P 500 Index Corporate Class ETF", False, [
                ("Global stocks slide on rate fears", False)]),
            ("CH", "", True, [
                ("Chile ETF (CH) hits a new high", True),
                ("NYSE:CH moves", True),
                ("TSXV:CH moves", False)]),
        ]
        for sym, name, us, heads in cases:
            for head, want in heads:
                self.assertEqual(news.names_listing(head, sym, name, us), want, "%s: %s" % (sym, head))

    def test_google_items_lose_the_publisher_suffix_quote_pages_and_undated_pages(self):
        item = lambda title, source, when, link: "<item><title>%s</title><link>%s</link><pubDate>%s</pubDate><source url=\"x\">%s</source></item>" % (title, link, when, source)
        xml = "<rss><channel>%s</channel></rss>" % "".join([
            item("HydroGraph Announces Change of Auditor - Investing News Network", "Investing News Network", "Mon, 31 Aug 2026 12:00:00 GMT", "https://news.google.com/rss/articles/a"),
            item("HG Stock Price and Chart — CSE:HG - tradingview.com", "tradingview.com", "Thu, 01 Jan 1970 00:00:00 GMT", "https://news.google.com/rss/articles/b"),
            item("HydroGraph Clean Power Stock Price, News, Quote &amp; History - Investing News Network", "Investing News Network", "Tue, 13 Jan 2026 00:00:00 GMT", "https://news.google.com/rss/articles/c"),
            item("Hydrograph Clean Power Inc Revenue Breakdown – CSE:HG - tradingview.com", "tradingview.com", "Fri, 31 Jul 2026 00:00:00 GMT", "https://news.google.com/rss/articles/d"),
            item("HG Forecast — Price Target — Prediction for 2027 - TradingView", "TradingView", "Fri, 31 Jul 2026 00:00:00 GMT", "https://news.google.com/rss/articles/e"),
            item("What is HydroGraph Clean Power rStock | How RHG Works - MEXC", "MEXC", "Fri, 31 Jul 2026 00:00:00 GMT", "https://news.google.com/rss/articles/f"),
            item("$HydroGraph Clean Power (HGRAF.US)$ - Moomoo", "Moomoo", "Fri, 31 Jul 2026 00:00:00 GMT", "https://news.google.com/rss/articles/g")])
        rows = news.parse_google_news(xml, "HG", "Hydrograph Clean Power Inc.")
        self.assertEqual([(r["headline"], r["source"], r["publishedAt"], r["kind"]) for r in rows],
                         [("HydroGraph Announces Change of Auditor", "Investing News Network", "2026-08-31T12:00:00Z", "story")])
        self.assertTrue(rows[0]["id"].startswith("gnews:"))


class MergeTest(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        os.environ["BAGHOLDER_HOME"] = self.tmp.name
        store.set_home(self.tmp.name)
        bagholder.set_home(self.tmp.name)
        store.ensure()

    def tearDown(self):
        self.tmp.cleanup()
        os.environ.pop("BAGHOLDER_HOME", None)

    @staticmethod
    def row(i, headline, when, source="Pub", kind="story"):
        return {"id": i, "headline": headline, "source": source, "url": "u-" + i, "publishedAt": when, "kind": kind}

    def test_every_source_is_merged_one_row_per_story_the_wires_copy_first(self):
        now = datetime(2026, 9, 16, 12, 0, tzinfo=timezone.utc)
        wire = [self.row("tmx:1", "Charbone Closes Loan", "2026-09-08T12:00:00Z", "TheNewsWire", "release")]
        answers = {"yahoo": [self.row("yahoo:u1", "CHARBONE closes loan.", "2026-09-08T12:00:00Z", "TheNewsWire", "release"),
                             self.row("yahoo:u2", "Charbone delivers electrolyzer", "2026-09-09T12:00:00Z", "BNN Bloomberg")],
                   "sa": [self.row("sa:1", "Charbone: a hydrogen story", "2026-09-10T12:00:00Z", "Seeking Alpha")],
                   "gnews": [self.row("gnews:1", "Charbone delivers electrolyzer", "2026-09-09T12:05:00Z", "The Globe and Mail"),
                             self.row("gnews:2", "Charbone Reports Q2 2026 Financial Results", "2026-08-27T12:00:00Z", "The Globe and Mail")]}
        asked = []
        def extra(key, symbol, exchange, currency, name, ssl_context):
            asked.append((key, name))
            return answers[key]
        with mock.patch.object(news, "fetch_symbol", return_value=("tmx", wire)), mock.patch.object(news, "_read_extra", side_effect=extra):
            src, rows = news.read_listing("CH", "TSX-V", "CAD", now=now, name="Charbone Hydrogen Corp")
        self.assertEqual(sorted(asked), [("gnews", "Charbone Hydrogen Corp"), ("sa", "Charbone Hydrogen Corp"), ("yahoo", "Charbone Hydrogen Corp")])
        self.assertEqual((src, [r["id"] for r in rows]), ("tmx", ["sa:1", "yahoo:u2", "tmx:1", "gnews:2"]),
                         "newest first; the same headline from a later source is the earlier source's row")
        stored = store.news_for("CH", "TSX-V")
        self.assertEqual([(r["id"], r["source"], r["wire"]) for r in stored],
                         [("sa:1", "sa", "Seeking Alpha"), ("yahoo:u2", "yahoo", "BNN Bloomberg"), ("tmx:1", "tmx", "TheNewsWire"), ("gnews:2", "gnews", "The Globe and Mail")],
                         "each row is stored under the source it was read from")

    def test_a_source_that_fails_or_is_not_due_keeps_its_stored_stories(self):
        now = datetime(2026, 9, 16, 12, 0, tzinfo=timezone.utc)
        first = {"yahoo": [self.row("yahoo:u1", "Yahoo story", "2026-09-10T12:00:00Z")],
                 "sa": [self.row("sa:1", "SA story", "2026-09-11T12:00:00Z")],
                 "gnews": [self.row("gnews:1", "Google story", "2026-09-12T12:00:00Z")]}
        with mock.patch.object(news, "fetch_symbol", return_value=("tmx", [self.row("tmx:1", "Wire item", "2026-09-09T12:00:00Z")])), \
             mock.patch.object(news, "_read_extra", side_effect=lambda k, *a: first[k]):
            news.read_listing("CH", "TSX-V", "CAD", now=now)
        asked = []
        def later(key, *a):
            asked.append(key)
            if key == "yahoo":
                raise OSError("down")
            return []
        # fifteen minutes on: the wire and Yahoo are due, Seeking Alpha and Google are not; Yahoo fails
        with mock.patch.object(news, "fetch_symbol", return_value=("tmx", [self.row("tmx:2", "New wire item", "2026-09-16T12:10:00Z")])), \
             mock.patch.object(news, "_read_extra", side_effect=later), mock.patch.object(news.sys, "stderr"):
            _, rows = news.read_listing("CH", "TSX-V", "CAD", now=now + timedelta(minutes=16))
        self.assertEqual(asked, ["yahoo"], "Seeking Alpha and Google are read every thirty minutes")
        self.assertEqual([r["id"] for r in rows], ["tmx:2", "gnews:1", "sa:1", "yahoo:u1"],
                         "the wire's item is replaced; the failing and the resting sources keep what they had")
        # nothing answers at all: the stored list stands untouched
        with mock.patch.object(news, "fetch_symbol", return_value=("tmx", None)), \
             mock.patch.object(news, "_read_extra", side_effect=OSError("down")), mock.patch.object(news.sys, "stderr"):
            self.assertEqual(news.read_listing("CH", "TSX-V", "CAD", now=now + timedelta(minutes=45), force=True), ("tmx", None))
        self.assertEqual([r["id"] for r in store.news_for("CH", "TSX-V")], ["tmx:2", "gnews:1", "sa:1", "yahoo:u1"])
        # every source answers, Yahoo with nothing at all: a list it had does not vanish on one empty answer
        with mock.patch.object(news, "fetch_symbol", return_value=("tmx", [self.row("tmx:2", "New wire item", "2026-09-16T12:10:00Z")])), \
             mock.patch.object(news, "_read_extra", side_effect=lambda k, *a: [] if k == "yahoo" else first[k]):
            _, rows = news.read_listing("CH", "TSX-V", "CAD", now=now + timedelta(minutes=60), force=True)
        self.assertIn("yahoo:u1", [r["id"] for r in rows])

    def test_a_release_is_new_once_whichever_source_carries_it_and_a_first_read_source_is_history(self):
        now = datetime(2026, 9, 16, 12, 0, tzinfo=timezone.utc)
        told = []
        on_new = lambda sym, ex, rows, new: told.append(sorted(new))
        release = lambda i, when="2026-09-16T11:00:00Z": self.row(i, "Charbone Closes Loan", when, "TheNewsWire", "release")
        # the wire is read, Yahoo for the first time: Yahoo's whole list is history
        with mock.patch.object(news, "fetch_symbol", return_value=("tmx", [self.row("tmx:1", "Old wire item", "2026-09-01T12:00:00Z")])), \
             mock.patch.object(news, "_read_extra", side_effect=lambda k, *a: [self.row("yahoo:old", "Charbone old release", "2026-09-10T12:00:00Z", "NewMediaWire", "release")] if k == "yahoo" else None):
            news.read_listing("CH", "TSX-V", "CAD", now=now, on_new=on_new)
        self.assertEqual(told[-1], ["tmx:1"], "a source met for the first time brings history, not news (the wire's own first read is the notifier's to judge)")
        # TMX fails this pass and Yahoo carries a new release: it is new
        with mock.patch.object(news, "fetch_symbol", return_value=("tmx", None)), \
             mock.patch.object(news, "_read_extra", side_effect=lambda k, *a: [release("yahoo:new")] if k == "yahoo" else None):
            news.read_listing("CH", "TSX-V", "CAD", now=now + timedelta(minutes=16), on_new=on_new)
        self.assertEqual(told[-1], ["yahoo:new"])
        # TMX answers again with the same release under its own id: not new a second time
        with mock.patch.object(news, "fetch_symbol", return_value=("tmx", [release("tmx:999")])), \
             mock.patch.object(news, "_read_extra", side_effect=lambda k, *a: [release("yahoo:new")] if k == "yahoo" else None):
            _, rows = news.read_listing("CH", "TSX-V", "CAD", now=now + timedelta(minutes=32), on_new=on_new)
        self.assertIn("tmx:999", [r["id"] for r in rows], "the wire's copy is the row")
        self.assertEqual(told[-1], [], "the same headline under the wire's id is the story already told")

    def test_a_headline_repeated_months_later_is_a_new_release(self):
        now = datetime(2026, 9, 16, 12, 0, tzinfo=timezone.utc)
        told = []
        halt = lambda i, when: self.row(i, "IIROC Trading Halt - QNC", when, "TMX Newsfile", "release")
        with mock.patch.object(news, "fetch_symbol", return_value=("tmx", [halt("tmx:1", "2026-06-10T14:00:00Z")])), mock.patch.object(news, "_read_extra", return_value=None):
            news.read_listing("QNC", "TSX-V", "CAD", now=now)
        with mock.patch.object(news, "fetch_symbol", return_value=("tmx", [halt("tmx:2", "2026-09-16T13:00:00Z"), halt("tmx:1", "2026-06-10T14:00:00Z")])), \
             mock.patch.object(news, "_read_extra", return_value=None):
            _, rows = news.read_listing("QNC", "TSX-V", "CAD", now=now + timedelta(minutes=16), on_new=lambda s, e, r, new: told.append(sorted(new)))
        self.assertEqual(([r["id"] for r in rows], told), (["tmx:2", "tmx:1"], [["tmx:2"]]), "the same words months apart are two halts, the second one new")

    def test_a_wire_feed_that_fails_keeps_its_stored_items(self):
        now = datetime(2026, 9, 16, 12, 0, tzinfo=timezone.utc)
        release = {"newsid": "10", "headline": "Kraken closes financing", "source": "GlobeNewswire via QuoteMedia", "datetime": "2026-09-05T08:00:00-04:00"}
        story = {"newsid": "11", "headline": "3 Top Canadian Defence Stocks", "source": "Motley Fool Canada via QuoteMedia", "datetime": "2026-09-02T09:00:00-04:00", "topic": "[PNG:CA]"}
        def both(url, body, ctx, headers=None, **kw):
            return {"data": {"news": [story if body["variables"].get("companyInNews") else release]}}
        def stories_down(url, body, ctx, headers=None, **kw):
            if body["variables"].get("companyInNews"):
                raise OSError("down")
            return {"data": {"news": [release]}}
        def read(post, minutes):
            with mock.patch.object(market, "_post_json", side_effect=post), mock.patch.object(news, "_pace"), \
                 mock.patch.object(market, "tmx_quote_symbol", return_value="PNG"), mock.patch.object(news, "_read_extra", return_value=None), \
                 mock.patch.object(news.sys, "stderr"):
                return news.read_listing("PNG", "TSX-V", "CAD", now=now + timedelta(minutes=minutes))[1]
        read(both, 0)
        rows = read(stories_down, 16)
        self.assertEqual(sorted(r["id"] for r in rows), ["tmx:10", "tmx:11"], "In The Media failing leaves the stories it had")
        self.assertEqual({r["id"]: r["source"] for r in store.news_for("PNG", "TSX-V")}, {"tmx:10": "tmx", "tmx:11": "tmx-media"})

    def test_a_source_with_nothing_to_ask_does_not_count_as_an_answer(self):
        # a CSE listing: Seeking Alpha has no feed for it; everything that can be asked fails
        with mock.patch.object(news, "fetch_symbol", return_value=("tmx", None)), mock.patch.object(news, "fetch_yahoo", side_effect=OSError("down")), \
             mock.patch.object(news, "fetch_google", side_effect=OSError("down")), mock.patch.object(market, "_get_text") as get, mock.patch.object(news.sys, "stderr"):
            self.assertEqual(news.read_listing("HG", "CSE", "CAD", force=True, name="Hydrograph Clean Power Inc."), ("tmx", None),
                             "nothing answered, so the listing is asked again next pass")
        get.assert_not_called()
        self.assertIsNone(news.fetch_sa("HG", "CSE", "CAD"))

    def test_a_listing_is_due_while_any_of_its_sources_is(self):
        """Freshness is each source's own: a wire read moments ago by a copy of the app that asked
        nothing else leaves the other sources to read, and a source with nothing to ask keeps nothing due."""
        now = datetime(2026, 9, 16, 12, 0, tzinfo=timezone.utc)
        store.replace_news("CH", "TSX-V", "tmx", [self.row("tmx:1", "Wire item", "2026-09-16T11:00:00Z")], now=now)
        store.replace_news("HG", "CSE", "tmx", [self.row("tmx:2", "Wire item", "2026-09-16T11:00:00Z")], now=now)
        listings = [("CH", "TSX-V", "CAD", "Charbone Hydrogen Corp"), ("HG", "CSE", "CAD", "Hydrograph Clean Power Inc.")]
        self.assertEqual([l[0] for l in news.stale(listings, now=now + timedelta(minutes=1))], ["CH", "HG"], "the wire is fresh, the other sources never read")
        self.assertNotIn("sa", news.sources_for("HG", "CSE", "CAD", "Hydrograph Clean Power Inc."), "Seeking Alpha has no CSE feed")
        started, landed = [], []
        with mock.patch.object(news, "fetch_symbol", return_value=("tmx", [])), mock.patch.object(news, "_read_extra", return_value=[]):
            self.assertEqual(news.refresh(listings, now=now + timedelta(minutes=1), on_start=lambda due: started.extend(l[0] for l in due),
                                          on_done=lambda l, ok: landed.append((l[0], ok))), 2)
        self.assertEqual((sorted(started), sorted(landed)), (["CH", "HG"], [("CH", True), ("HG", True)]))
        self.assertEqual(news.stale(listings, now=now + timedelta(minutes=2)), [], "every source read: nothing due, a source with nothing to ask included")
        self.assertEqual([l[0] for l in news.stale(listings, now=now + timedelta(minutes=17))], ["CH", "HG"])

    def test_a_listing_with_no_venue_is_left_to_the_wire(self):
        with mock.patch.object(news, "fetch_symbol", return_value=("nasdaq", [])), mock.patch.object(news, "_read_extra") as extra:
            news.read_listing("F", "", "")
        extra.assert_not_called()

    def test_a_searched_ticker_is_read_from_every_source_under_the_name_tmx_gives(self):
        with mock.patch.object(store, "list_securities", return_value=[]), \
             mock.patch.object(market, "tmx_listing", return_value={"symbol": "SXHI", "name": "Ninepoint SpaceX HighShares ETF", "exchange": "TSX", "currency": "CAD"}), \
             mock.patch.object(news, "read_listing", return_value=("tmx", [])) as read, mock.patch.object(bagholder, "_ssl_context", return_value=None):
            out = bagholder.news_symbol_payload("SXHI", "", "")
        self.assertEqual(out["exchange"], "TSX")
        self.assertEqual(read.call_args.args[:3], ("SXHI", "TSX", "CAD"))
        self.assertEqual((read.call_args.kwargs["name"], read.call_args.kwargs["force"]), ("Ninepoint SpaceX HighShares ETF", True))


class StoreTest(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        os.environ["BAGHOLDER_HOME"] = self.tmp.name
        store.set_home(self.tmp.name)
        bagholder.set_home(self.tmp.name)
        store.ensure()
        model.invalidate()

    def tearDown(self):
        self.tmp.cleanup()

    def test_refresh_reads_only_stale_listings_and_replaces_their_rows(self):
        now = datetime(2026, 9, 11, 15, 30, tzinfo=timezone.utc)
        answers = {"SHOP": [{"id": "tmx:1", "headline": "One", "source": "GlobeNewswire", "url": "u1", "publishedAt": "2026-09-11T14:00:00Z"}],
                   "NVDA": [{"id": "nasdaq:9", "headline": "Nine", "source": "Zacks", "url": "u9", "publishedAt": "2026-09-11T15:00:00Z"}]}
        calls = []
        def fake(symbol, exchange, currency, ssl_context=None, now=None):
            calls.append(symbol)
            return ("tmx" if exchange == "TSX" else "nasdaq"), answers.get(symbol)
        def others(key, symbol, exchange, currency, name, ssl_context):
            if symbol == "BROKEN":
                raise OSError("down")
            return []
        listings = [("SHOP", "TSX", "CAD"), ("NVDA", "NASDAQ", "USD"), ("BROKEN", "TSX", "CAD")]
        with mock.patch.object(news, "fetch_symbol", side_effect=fake), mock.patch.object(news, "_read_extra", side_effect=others), mock.patch.object(news.sys, "stderr"):
            self.assertEqual(news.refresh(listings, now=now), 2, "a listing no source answers for leaves nothing behind and is asked again next time")
            self.assertEqual(sorted(calls), ["BROKEN", "NVDA", "SHOP"], "listings are read side by side")
            calls.clear()
            self.assertEqual(news.refresh(listings, now=now), 0)
            self.assertEqual(calls, ["BROKEN"], "fresh listings are not asked again within fifteen minutes")
            answers["SHOP"] = [{"id": "tmx:2", "headline": "Two", "source": "CNW", "url": "u2", "publishedAt": "2026-09-11T16:00:00Z"}]
            later = datetime(2026, 9, 11, 16, 0, tzinfo=timezone.utc)
            news.refresh(listings, now=later)
        rows = store.snapshot()["news"]
        self.assertEqual([(r["id"], r["symbol"], r["wire"]) for r in rows], [("tmx:2", "SHOP", "CNW"), ("nasdaq:9", "NVDA", "Zacks")], "newest first; a listing's rows are replaced by its wire's latest")
        store.forget_news("SHOP", "TSX")
        self.assertEqual([r["id"] for r in store.snapshot()["news"]], ["nasdaq:9"])
        self.assertEqual(news.stale([("SHOP", "TSX", "CAD")], now=later), [("SHOP", "TSX", "CAD", "")], "forgotten means stale")

    def test_rows_keep_their_kind_and_an_old_table_is_told_by_its_wires(self):
        store.replace_news("SHOP", "NASDAQ", "nasdaq", [{"id": "nasdaq:1", "headline": "a", "source": "Zacks", "url": "u", "publishedAt": "2026-09-14T00:00:00Z", "kind": "story"},
                                                       {"id": "nasdaq:2", "headline": "b", "source": "GlobeNewswire", "url": "u", "publishedAt": "2026-08-05T00:00:00Z", "kind": "release"}])
        kinds = {r["id"]: r["kind"] for r in store.list_news()} if hasattr(store, "list_news") else None
        with store._lock:
            conn = store._connect()
            try:
                conn.execute("UPDATE news SET kind = NULL")
                conn.commit()
                store._ensure_news_columns(conn)
                conn.commit()
                told = {r["id"]: r["kind"] for r in conn.execute("SELECT id, kind FROM news").fetchall()}
            finally:
                conn.close()
        self.assertEqual(told, {"nasdaq:1": "story", "nasdaq:2": "release"}, "a table from before releases were told apart is told by the wire names it holds")

    def test_trim_keeps_the_newest(self):
        store.replace_news("A", "TSX", "tmx", [{"id": "tmx:%d" % i, "headline": str(i), "source": "", "url": "", "publishedAt": "2026-09-%02dT00:00:00Z" % i} for i in range(1, 6)])
        store.trim_news(2)
        self.assertEqual([r["id"] for r in store.snapshot()["news"]], ["tmx:5", "tmx:4"])


class ModelTest(unittest.TestCase):
    def test_rows_are_tagged_with_what_the_book_holds_or_watches(self):
        base = {"news": [{"id": "tmx:1", "symbol": "SHOP", "exchange": "TSX", "wire": "GlobeNewswire", "headline": "One", "url": "u1", "publishedAt": "2026-09-11T14:00:00Z"},
                         {"id": "tmx:1", "symbol": "HHIS", "exchange": "TSX", "wire": "GlobeNewswire", "headline": "One", "url": "u1", "publishedAt": "2026-09-11T14:00:00Z"},
                         {"id": "nasdaq:9", "symbol": "NVDA", "exchange": "NASDAQ", "wire": "Zacks", "headline": "Nine", "url": "u9", "publishedAt": "2026-09-11T15:00:00Z"}]}
        positions = [{"id": "p1", "symbol": "HHIS", "exchange": "TSX", "percentChange": 0.6}]
        watch = [{"symbol": "SHOP", "exchange": "TSX", "percentChange": 3.28}, {"symbol": "NVDA", "exchange": "NASDAQ", "percentChange": None}]
        rows = model.news_rows(base, positions, watch)
        self.assertEqual([(r["id"], [(t["symbol"], t["held"], t["watched"], t["percentChange"], t["positionId"]) for t in r["tags"]]) for r in rows],
                         [("nasdaq:9", [("NVDA", False, True, None, None)]), ("tmx:1", [("SHOP", False, True, 3.28, None), ("HHIS", True, False, 0.6, "p1")])],
                         "newest first; an item two listings share is one row with both tags")

    def test_the_market_feed_is_a_listing_of_its_own_with_no_tag(self):
        self.assertEqual(news.source_for(*news.MARKET), "nasdaq")
        data = {"data": {"rows": [{"id": 1, "title": "Stocks Settle Lower", "publisher": "Barchart", "url": "/articles/a", "ago": "7 minutes ago", "related_symbols": ["ryam|stocks"]},
                                  {"id": 2, "title": "Value ETFs", "publisher": "Zacks", "url": "/articles/b", "ago": "2 hours ago", "related_symbols": ["mu|stocks"]}]}}
        now = datetime(2026, 9, 11, 16, 0, tzinfo=timezone.utc)
        self.assertEqual([r["id"] for r in news.parse_nasdaq_news(data, now, "")], ["nasdaq:1", "nasdaq:2"], "asked without a symbol, the feed keeps every item")
        base = {"news": [{"id": "nasdaq:1", "symbol": "*", "exchange": "MARKET", "wire": "Barchart", "headline": "Stocks Settle Lower", "url": "u1", "publishedAt": "2026-09-11T15:53:00Z"},
                         {"id": "nasdaq:2", "symbol": "*", "exchange": "MARKET", "wire": "Zacks", "headline": "Value ETFs", "url": "u2", "publishedAt": "2026-09-11T14:00:00Z"},
                         {"id": "nasdaq:2", "symbol": "MU", "exchange": "NASDAQ", "wire": "Zacks", "headline": "Value ETFs", "url": "u2", "publishedAt": "2026-09-11T14:00:00Z"}]}
        rows = model.news_rows(base, [], [{"symbol": "MU", "exchange": "NASDAQ", "percentChange": -0.18}])
        self.assertEqual([(r["id"], r["market"], [t["symbol"] for t in r["tags"]]) for r in rows], [("nasdaq:1", True, []), ("nasdaq:2", True, ["MU"])],
                         "a market item carries no tag; the same story read for a watched listing is one row, tagged, and still the market's")

    def test_the_same_headline_under_other_ids_is_one_row(self):
        base = {"news": [{"id": "tmx:1", "symbol": "ENB", "exchange": "TSX", "wire": "PR Newswire", "headline": "Enbridge Announces Retirement of Greg Ebel", "url": "u1", "publishedAt": "2026-09-08T12:00:00Z"},
                         {"id": "tmx:2", "symbol": "ENB", "exchange": "TSX", "wire": "Canada Newswire", "headline": "Enbridge Announces Retirement of Greg Ebel", "url": "u2", "publishedAt": "2026-09-08T12:01:00Z"},
                         {"id": "nasdaq:7", "symbol": "AAPL", "exchange": "NASDAQ", "wire": "Barchart", "headline": "Stocks Shake Off CPI Report", "url": "u7", "publishedAt": "2026-09-11T18:07:00Z"},
                         {"id": "nasdaq:8", "symbol": "MSFT", "exchange": "NASDAQ", "wire": "Barchart", "headline": "Stocks Shake Off CPI Report", "url": "u8", "publishedAt": "2026-09-11T18:07:00Z"},
                         {"id": "nasdaq:9", "symbol": "*", "exchange": "MARKET", "wire": "Barchart", "headline": "Stocks shake off CPI report.", "url": "u9", "publishedAt": "2026-09-11T18:07:00Z"}]}
        rows = model.news_rows(base, [{"id": "p1", "symbol": "AAPL", "exchange": "NASDAQ"}, {"id": "p2", "symbol": "MSFT", "exchange": "NASDAQ"}], [{"symbol": "ENB", "exchange": "TSX"}])
        self.assertEqual([(r["id"], r["market"], sorted(t["symbol"] for t in r["tags"]), r["publishedAt"]) for r in rows],
                         [("nasdaq:7", True, ["AAPL", "MSFT"], "2026-09-11T18:07:00Z"), ("tmx:2", False, ["ENB"], "2026-09-08T12:01:00Z")],
                         "a release on two wires is one row (the newest kept); a story per symbol feed and in the market feed is one row, tagged, the market's")

    def test_a_french_release_beside_its_english_original_is_one_story(self):
        base = {"news": [{"id": "tmx:1", "symbol": "CH", "exchange": "TSX-V", "wire": "TheNewsWire", "headline": "CHARBONE annonce la clôture du tirage de 1,5 M$ auprès de RiverFort pour accélérer sa croissance", "url": "u1", "publishedAt": "2026-09-08T12:05:00Z"},
                         {"id": "tmx:2", "symbol": "CH", "exchange": "TSX-V", "wire": "TheNewsWire", "headline": "CHARBONE Announces Closing of $1.5M Drawdown with RiverFort to Accelerate Growth", "url": "u2", "publishedAt": "2026-09-08T12:00:00Z"},
                         {"id": "tmx:3", "symbol": "CH", "exchange": "TSX-V", "wire": "TheNewsWire", "headline": "Charbone annonce un tirage de 1,5 M$ du prêt convertible de 10 M$, accélérant sa croissance", "url": "u3", "publishedAt": "2026-09-02T12:00:00Z"},
                         {"id": "tmx:4", "symbol": "ENB", "exchange": "TSX", "wire": "Canada Newswire", "headline": "Enbridge annonce ses résultats du deuxième trimestre", "url": "u4", "publishedAt": "2026-08-01T12:00:00Z"}]}
        rows = model.news_rows(base, [{"id": "p1", "symbol": "CH", "exchange": "TSX-V"}], [])
        self.assertEqual([r["id"] for r in rows], ["tmx:2", "tmx:3", "tmx:4"],
                         "the French twin of an English release goes; a French release with no English twin within three hours stays")

    def test_the_books_form_and_the_bare_ticker_are_one_listing(self):
        base = {"news": [{"id": "tmx:7", "symbol": "QNC.TO", "exchange": "TSX-V", "wire": "TMX Newsfile", "headline": "Seven", "url": "u7", "publishedAt": "2026-09-08T13:00:00Z"},
                         {"id": "tmx:7", "symbol": "QNC", "exchange": "TSX-V", "wire": "TMX Newsfile", "headline": "Seven", "url": "u7", "publishedAt": "2026-09-08T13:00:00Z"}]}
        positions = [{"id": "p2", "symbol": "QNC.TO", "exchange": "TSX-V", "percentChange": -1.67}]
        watch = [{"symbol": "QNC", "exchange": "TSX-V", "percentChange": -1.67}]
        rows = model.news_rows(base, positions, watch)
        self.assertEqual([(t["symbol"], t["held"], t["watched"], t["percentChange"], t["positionId"]) for t in rows[0]["tags"]], [("QNC", True, True, -1.67, "p2")], "one tag, held and watched, the bare ticker")


if __name__ == "__main__":
    unittest.main()
