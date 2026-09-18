"""The heatmap's market universes: Nasdaq's screener sliced into US and International, TMX's TSX 60."""
from __future__ import annotations

import os
import tempfile
import unittest

import bagholder
import model
import store
import universes


class ScreenerTest(unittest.TestCase):
    ROWS = {"data": {"rows": [
        {"symbol": "NVDA", "name": "NVIDIA Corporation Common Stock", "lastsale": "$219.41", "pctchange": "0.808%", "marketCap": "5350000000000.00", "sector": "Technology", "country": "United States"},
        {"symbol": "JPM", "name": "JP Morgan", "lastsale": "$210.00", "pctchange": "-0.5%", "marketCap": "600000000000.00", "sector": "Finance", "country": "United States"},
        {"symbol": "TSM", "name": "Taiwan Semiconductor", "lastsale": "$250.00", "pctchange": "1.2%", "marketCap": "1300000000000.00", "sector": "Technology", "country": "Taiwan"},
        {"symbol": "SHOP", "name": "Shopify", "lastsale": "$130.00", "pctchange": "3.3%", "marketCap": "170000000000.00", "sector": "Technology", "country": "Canada"},
        {"symbol": "XYZ", "name": "No cap", "lastsale": "$1.00", "pctchange": "N/A", "marketCap": "", "sector": "Miscellaneous", "country": "United States"},
        {"symbol": "ABC", "name": "No country", "lastsale": "$2.00", "pctchange": "0.1%", "marketCap": "100.00", "sector": "Telecommunications", "country": ""},
    ]}}

    def test_rows_are_parsed_and_sectors_folded(self):
        rows = universes.parse_screener(self.ROWS)
        self.assertEqual([(r["symbol"], r["last"], r["percentChange"], r["cap"], r["sector"], r["country"]) for r in rows][:2],
                         [("NVDA", 219.41, 0.808, 5.35e12, "Information Technology", "United States"), ("JPM", 210.0, -0.5, 6e11, "Financials", "United States")])
        self.assertEqual([r["sector"] for r in rows][4:], ["Not classified", "Communication Services"])
        self.assertIsNone(rows[4]["percentChange"])

    def test_us_and_international_are_the_largest_by_cap(self):
        rows = universes.parse_screener(self.ROWS)
        self.assertEqual([r["symbol"] for r in universes.us_rows(rows, 5)], ["NVDA", "JPM"], "US companies with a market cap, largest first")
        self.assertEqual([r["symbol"] for r in universes.intl_rows(rows, 5)], ["TSM"], "foreign companies listed in the US; Canada and blanks excluded")
        self.assertEqual(universes.us_rows(rows, 1)[0]["value"], 5.35e12, "sized by market cap")


class CanadaTest(unittest.TestCase):
    def test_constituents_and_tile_quote(self):
        cons = universes.parse_constituents({"data": {"constituents": [{"symbol": "RY", "quotedMarketValue": 398317400940, "longName": "Royal Bank of Canada", "weight": 9.823, "exchange": "TSX"}, {"weight": 1}]}})
        self.assertEqual(cons, [{"symbol": "RY", "name": "Royal Bank of Canada", "weight": 9.823, "cap": 398317400940.0, "exchange": "TSX"}])
        q = universes.parse_tile_quote({"data": {"getQuoteBySymbol": {"symbol": "RY", "name": "Royal Bank", "price": 180.1, "percentChange": 0.42, "sector": "Financial Services"}}})
        self.assertEqual(q, {"percentChange": 0.42, "sector": "Financials", "name": "Royal Bank"})
        self.assertIsNone(universes.parse_tile_quote({"data": {"getQuoteBySymbol": None}}))


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

    def test_replace_and_snapshot(self):
        before = store.data_version()
        store.replace_universe("ca", [{"symbol": "RY", "name": "Royal Bank", "value": 9.823, "percentChange": 0.42, "sector": "Financials", "country": "Canada"}], now="2026-09-11T16:00:00Z")
        store.replace_universe("ca", [{"symbol": "TD", "name": "TD", "value": 6.9, "percentChange": None, "sector": "Financials", "country": "Canada"}], now="2026-09-11T16:30:00Z")
        u = store.snapshot()["universes"]
        self.assertEqual([(r["symbol"], r["value"], r["percentChange"], r["fetchedAt"]) for r in u["ca"]], [("TD", 6.9, None, "2026-09-11T16:30:00Z")], "an answer replaces the universe's rows")
        self.assertNotIn("us", u)
        self.assertNotEqual(store.data_version(), before)


if __name__ == "__main__":
    unittest.main()
