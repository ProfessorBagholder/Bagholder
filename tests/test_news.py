"""News: two per-symbol wires parsed into rows, kept per listing, tagged in the model."""
from __future__ import annotations

import os
import tempfile
import unittest
from datetime import datetime, timezone
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
                                 "url": "https://money.tmx.com/en/quote/SHOP/news/4883675477075330", "publishedAt": "2026-08-05T11:00:00Z", "kind": "release"}])

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
    def test_a_wires_item_is_a_release_and_a_publishers_a_story(self):
        for wire in ("GlobeNewswire", "Business Wire", "PR Newswire", "ACCESS Newswire", "TheNewsWire", "Canada Newswire", "TMX Newsfile", "Marketwired", "CNW Group"):
            self.assertEqual(news.kind_of(wire), "release", wire)
        for pub in ("The Motley Fool", "Zacks", "Barchart", "RTTNews", "MarketBeat", "BNK Invest", "Fintel", ""):
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
            self.assertEqual((src, asked), ("tmx", ["QIMC:CNX", "CH"]), "each listing under the code its quote uses")
            self.assertEqual((rows[0]["kind"], rows[0]["url"]), ("release", "https://money.tmx.com/en/quote/QIMC:CNX/news/7"))
            # the record names the wrong venue: the lookup resolves the form that answers, as it does for a quote
            asked.clear()
            with mock.patch.object(market, "tmx_resolve", return_value="QIMC:CNX"):
                _, found = news.fetch_symbol("QIMC", "TSX-V", "CAD")
            self.assertEqual((asked, [r["id"] for r in found]), (["QIMC", "QIMC:CNX"], ["tmx:7"]))

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
        listings = [("SHOP", "TSX", "CAD"), ("NVDA", "NASDAQ", "USD"), ("BROKEN", "TSX", "CAD")]
        with mock.patch.object(news, "fetch_symbol", side_effect=fake):
            self.assertEqual(news.refresh(listings, now=now), 2, "a wire that fails leaves nothing behind and is asked again next time")
            self.assertEqual(calls, ["SHOP", "NVDA", "BROKEN"])
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
        self.assertEqual(news.stale([("SHOP", "TSX", "CAD")], now=later), [("SHOP", "TSX", "CAD")], "forgotten means stale")

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
