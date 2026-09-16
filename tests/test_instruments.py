"""Indices, futures, rates and currency pairs the watchlist can follow, and the bare-ticker convention."""
from __future__ import annotations

import os
import tempfile
import unittest
from unittest import mock

import bagholder
import exposure
import instruments
import market
import model
import news
import store


class SearchTest(unittest.TestCase):
    def test_aliases_find_what_people_type(self):
        self.assertEqual([r["symbol"] for r in instruments.search("WTI")][:1], ["CL"])
        self.assertEqual([r["symbol"] for r in instruments.search("crude")][:2], ["CL", "BZ"])
        self.assertEqual([r["symbol"] for r in instruments.search("NDX")][:1], ["NDX"])
        self.assertEqual([r["symbol"] for r in instruments.search("nasdaq 100")][:1], ["NDX"])
        self.assertEqual([r["symbol"] for r in instruments.search("VIX")][:1], ["VIX"])
        self.assertEqual([r["symbol"] for r in instruments.search("volatility")][:1], ["VIX"])
        self.assertEqual([r["symbol"] for r in instruments.search("gold")][:1], ["GC"])
        self.assertEqual([r["symbol"] for r in instruments.search("ES")][:1], ["ES"], "the S&P 500 E-mini, the after-hours read")
        self.assertEqual([r["symbol"] for r in instruments.search("futures")][:4], ["ES", "NQ", "YM", "RTY"])
        self.assertEqual([r["symbol"] for r in instruments.search("dow futures")][:1], ["YM"])
        self.assertEqual([r["symbol"] for r in instruments.search("nasdaq futures")][:1], ["NQ"])
        self.assertEqual((instruments.find("es", "cme")["yahoo"], instruments.KIND_LABEL["Future"]), ("ES=F", "Futures"))
        self.assertEqual(instruments.search("ZZZZ"), [])
        self.assertEqual(instruments.search("V"), [], "a single letter is not a search for every V")
        self.assertEqual([r["symbol"] for r in instruments.search("VI")][:1], ["VIX"])
        row = instruments.search("VIX")[0]
        self.assertEqual((row["name"], row["exchange"], row["currency"], row["kind"]), ("CBOE Volatility Index", "Index", "USD", "Index"))

    def test_an_alias_hit_ranks_as_the_exact_match_it_is(self):
        rows = bagholder.rank_search("WTI", instruments.search("WTI") + [{"symbol": "WTI", "name": "W&T Offshore", "exchange": "NYSE", "currency": "USD"}, {"symbol": "WTIB", "name": "USCF", "exchange": "NYSE", "currency": "USD"}])
        self.assertEqual([(r["symbol"], r["exchange"]) for r in rows][:3], [("CL", "NYMEX"), ("WTI", "NYSE"), ("WTIB", "NYSE")])

    def test_find_by_symbol_and_venue(self):
        self.assertEqual(instruments.find("cl", "nymex")["yahoo"], "CL=F")
        self.assertIsNone(instruments.find("CL", "TSX"), "a listing with the same letters is not the future")
        self.assertIsNone(instruments.find("SHOP", "TSX"))


class QuoteTest(unittest.TestCase):
    def test_yahoo_meta_becomes_a_quote(self):
        text = '{"chart": {"result": [{"meta": {"regularMarketPrice": 99.4, "chartPreviousClose": 93.03, "currency": "USD", "shortName": "Crude Oil Oct 26", "exchangeName": "NYM"}}]}}'
        q = market.parse_yahoo_quote(text)
        self.assertEqual((q["price"], round(q["priceChange"], 2), round(q["percentChange"], 2), q["prevClose"], q["currency"], q["name"]), (99.4, 6.37, 6.85, 93.03, "USD", "Crude Oil Oct 26"))
        self.assertIsNone(market.parse_yahoo_quote('{"chart": {"result": []}}'))

    def test_an_instrument_is_quoted_from_yahoo_under_its_own_key(self):
        needing = market.quote_symbols_needing_refresh([{"symbol": "VIX", "exchange": "Index", "currency": "USD", "kind": "Instrument", "yahoo": "^VIX", "quoteKey": "VIX@INDEX"}])
        self.assertEqual(needing, [("VIX@INDEX", "yahoo_quote", "^VIX")])


class ConventionTest(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        os.environ["BAGHOLDER_HOME"] = self.tmp.name
        store.set_home(self.tmp.name)
        bagholder.set_home(self.tmp.name)
        store.ensure()
        model.invalidate()

    def tearDown(self):
        self.tmp.cleanup()

    def test_a_watched_listing_is_kept_as_its_bare_ticker(self):
        with mock.patch.object(market, "refresh_quotes", return_value=0), mock.patch.object(exposure, "share_exposure", return_value={}) as se:
            r = bagholder.watch_add({"symbol": "QNC.TO", "exchange": "TSX-V", "name": "Quantum eMotion Corp", "currency": "CAD"})
            import time
            for _ in range(100):   # the background read finishes inside the mocks, not in the next test
                if se.called:
                    break
                time.sleep(0.05)
        self.assertEqual([(w["symbol"], w["exchange"]) for w in r["watchlist"]], [("QNC", "TSX-V")], "Wealthsimple's .TO is not the app's convention")
        r = bagholder.watch_remove({"symbol": "QNC.TO", "exchange": "TSX-V"})
        self.assertEqual(r["watchlist"], [], "removing by either form works")

    def test_an_instrument_takes_the_directory_name_and_a_kind(self):
        with mock.patch.object(market, "refresh_quotes", return_value=0) as rq, mock.patch.object(exposure, "share_exposure") as se:
            r = bagholder.watch_add({"symbol": "CL", "exchange": "NYMEX", "name": "", "currency": ""})
            import time
            for _ in range(50):
                if rq.called:
                    break
                time.sleep(0.05)
        self.assertEqual((r["watchlist"][0]["name"], r["watchlist"][0]["currency"]), ("Crude Oil (WTI)", "USD"))
        self.assertFalse(se.called, "a future has no sector record to read")
        base = model.base_model()
        self.assertEqual(model.watch_symbols(base), [{"symbol": "CL", "exchange": "NYMEX", "currency": "USD", "kind": "Instrument", "quoteKey": "CL@NYMEX", "yahoo": "CL=F"}])
        rows = model.watch_rows(dict(base, quotes={"CL@NYMEX": {"price": 99.4, "priceChange": 6.37, "percentChange": 6.85}}), [])
        self.assertEqual((rows[0]["sector"], rows[0]["kind"], rows[0]["last"]), ("Commodities", "Commodity", 99.4), "an instrument groups under its kind on the heatmap")
        self.assertEqual(bagholder.news_listings(), [news.MARKET], "no news wire for a future: only the market feed")

    def test_a_coin_from_the_book_is_quoted_by_coinbase(self):
        store.add_watch("BTC", "Crypto", "Bitcoin", "CAD")
        model.invalidate()
        base = model.base_model()
        self.assertEqual(model.watch_symbols(base), [{"symbol": "BTC", "exchange": "CRYPTO", "currency": "USD", "kind": "Crypto", "quoteKey": "BTC@CRYPTO"}], "the USD pair, whatever currency the book holds the coin in")
        self.assertEqual(market.quote_symbols_needing_refresh(model.watch_symbols(base)), [("BTC@CRYPTO", "coinbase", "BTC-USD")])
        row = model.watch_rows(dict(base, quotes={"BTC@CRYPTO": {"price": 150000.0}}), [])[0]
        self.assertEqual((row["sector"], row["kind"], row["last"]), ("Digital assets", "Crypto", 150000.0))
        self.assertEqual(bagholder.news_listings(), [news.MARKET], "no news wire for a coin: only the market feed")


if __name__ == "__main__":
    unittest.main()


class CoinChangeTest(unittest.TestCase):
    """A coin's day change: the spot against the previous UTC day's close on its Coinbase market."""

    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        os.environ["BAGHOLDER_HOME"] = self.tmp.name
        store.set_home(self.tmp.name)
        bagholder.set_home(self.tmp.name)
        store.ensure()

    def tearDown(self):
        self.tmp.cleanup()

    def test_previous_close_from_the_usd_market_when_the_pair_has_none(self):
        from datetime import datetime, timezone
        now = datetime(2026, 9, 11, 16, 0, tzinfo=timezone.utc)
        store.upsert_fx_rates([{"date": "2026-09-10", "rate": 1.38}, {"date": "2026-09-11", "rate": 1.39}]) if hasattr(store, "upsert_fx_rates") else None
        bars = [{"time": int(datetime(2026, 9, 9, tzinfo=timezone.utc).timestamp()), "open": 1, "high": 1, "low": 1, "close": 76000.0, "volume": 1},
                {"time": int(datetime(2026, 9, 10, tzinfo=timezone.utc).timestamp()), "open": 1, "high": 1, "low": 1, "close": 77000.0, "volume": 1},
                {"time": int(datetime(2026, 9, 11, tzinfo=timezone.utc).timestamp()), "open": 1, "high": 1, "low": 1, "close": 78000.0, "volume": 1}]
        markets = {"BTC-CAD": "", "BTC-USD": "BTC-USD"}
        with mock.patch.object(market, "coinbase_market", side_effect=lambda p, *a, **k: markets.get(p, "")), mock.patch.object(market, "fetch_coinbase_candles", return_value=bars) as fc, mock.patch.object(market, "in_position_currency", side_effect=lambda b, q, c: [dict(x, close=x["close"] * 1.38) for x in b]), mock.patch.object(market, "_get_text", return_value='{"data": {"amount": "107907.0", "base": "BTC", "currency": "CAD"}}'):
            rec = market.fetch_coinbase_spot("BTC-CAD", None, now)
            self.assertEqual((rec["price"], rec["prevClose"], round(rec["percentChange"], 2)), (107907.0, 77000.0 * 1.38, round((107907.0 - 106260.0) / 106260.0 * 100, 2)),
                             "yesterday's close, not today's running bar, converted to the pair's currency")
            self.assertEqual(fc.call_args[0][0], "BTC-USD")
            market.fetch_coinbase_spot("BTC-CAD", None, now)
            self.assertEqual(fc.call_count, 1, "the previous close is remembered for the day")
        with mock.patch.object(market, "coinbase_market", return_value=""), mock.patch.object(market, "_get_text", return_value='{"data": {"amount": "2.0", "currency": "CAD"}}'):
            rec = market.fetch_coinbase_spot("XYZ-CAD", None, now)
            self.assertEqual(rec, {"price": 2.0, "currency": "CAD"}, "no market, no change: the price alone")


class RateContractTest(unittest.TestCase):
    """The two contracts the market prices policy with are quoted as 100 minus the rate they
    settle against, so the rate is the price subtracted from 100 — the contract's own
    definition — and the tile carries both."""

    def test_the_directory_finds_them_by_the_words_people_type(self):
        self.assertEqual([r["symbol"] for r in instruments.search("fed")], ["ZQ"])
        self.assertEqual([r["symbol"] for r in instruments.search("sofr")], ["SR3"])
        self.assertEqual([r["symbol"] for r in instruments.search("fed funds")], ["ZQ"])
        self.assertEqual(instruments.label("ZQ"), "FED FUNDS")

    def test_the_rate_is_the_price_taken_from_a_hundred_and_nothing_else_carries_one(self):
        self.assertEqual(instruments.implied_rate("ZQ", 96.13), 3.87)
        self.assertEqual(instruments.implied_rate("SR3", 95.765), 4.235)
        self.assertIsNone(instruments.implied_rate("ES", 7674.0), "an index future prices no rate")
        self.assertIsNone(instruments.implied_rate("ZQ", None), "and an unquoted contract prices none either")

    def test_a_tile_carries_the_rate_beside_the_published_price_and_the_day_runs_the_other_way(self):
        base = {"tiles": [{"symbol": "ZQ", "exchange": "CBOT"}, {"symbol": "ES", "exchange": "CME"}],
                "quotes": {model.watch_quote_key("ZQ", "CBOT"): {"price": 96.13, "priceChange": -0.157, "percentChange": -0.163},
                           model.watch_quote_key("ES", "CME"): {"price": 7674.0, "priceChange": 18.0, "percentChange": 0.24}}}
        rows = {r["symbol"]: r for r in model.tile_rows(base)}
        self.assertEqual((rows["ZQ"]["last"], rows["ZQ"]["rate"], rows["ZQ"]["rateChange"]), (96.13, 3.87, 0.157),
                         "the price as published, the rate it prices, and a day that cut the price raised the rate")
        self.assertNotIn("rate", rows["ES"], "nothing else carries one")
