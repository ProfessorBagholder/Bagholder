"""Sector and country exposure: the parsers for each issuer's record, the names folded
onto one set, the look-through, and the Portfolio's slices."""
import json
import os
import sys
import tempfile
import unittest
from unittest import mock

sys.path.insert(0, os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
import exposure  # noqa: E402
import model  # noqa: E402
import store  # noqa: E402

ISHARES_CSV = """﻿Fund Holdings as of,"Sep 9, 2026"
\x20
Ticker,Name,Sector,Asset Class,Market Value,Weight (%),Notional Value,Shares,Price,Location,Exchange,Currency,FX Rate,Market Currency
"RY","ROYAL BANK OF CANADA","Financials","Equity","2,177,806,813.19","9.73","2,177,806,813.19","7,625,641.00","285.59","Canada","Toronto Stock Exchange","CAD","1.00","CAD"
"SHOP","SHOPIFY SUBORDINATE VOTING CLASS A","Information Technology","Equity","1,165,541,988.48","5.21","1,165,541,988.48","6,655,296.00","175.13","Canada","Toronto Stock Exchange","CAD","1.00","CAD"
"XEF","ISHARES MSCI EAFE IMI INDEX","Other","Equity","5,416,028,871.87","24.37","5,416,028,871.87","104,617,131.00","51.77","Canada","Toronto Stock Exchange","CAD","1.00","CAD"
"CAD","CAD CASH","Cash and/or Derivatives","Cash","31,678,794.26","0.14","31,678,794.26","31,678,794.00","100.00","Canada","-","CAD","1.00","CAD"
\x20
Fund Holdings as of,"Sep 9, 2026"
"""

EVOLVE_HTML = """<html><body><script>
var portfolioBreakdownData = {"data":{"geographic":[{"name":"BIGY","weight":"46.33%"}],"sector":[{"name":"Technology","weight":"27.86%"},{"name":"Financial","weight":"24.63%"},{"name":"Communications","weight":"15.72%"}]}};
var holdingsData = {"data":[{"ticker":"MSFT US EQUITY","weight_percent":"4.11%","position":"6301","security_name":"Microsoft Corp","gics_sector":"Technology","country":"BIGY","last_price":"491.65","value":"4,278,213"},{"ticker":"RY CN EQUITY","weight_percent":"2.00%","security_name":"Royal Bank of Canada","gics_sector":"Financial","country":"Canada"}]};
</script></body></html>"""

HARVEST_TABLE_HTML = """<table><tr><th>Name</th><th>Ticker</th><th>Weight</th><th>Sector</th><th>Country</th></tr>
<tr><td>iShares Bitcoin Trust ETF</td><td>IBIT US</td><td>130.3%</td><td>Bitcoin Holding</td><td>United States</td></tr>
<tr><td>Written Options</td><td></td><td>(4.5)%</td><td></td><td></td></tr>
<tr><td>Cash and other assets and liabilities</td><td></td><td>(25.8)%</td><td></td><td></td></tr></table>"""

HARVEST_NAMES_HTML = """<table><tr><th>Fund Details</th><th>As at 2026/09/09</th></tr><tr><td>Ticker</td><td>PLTE</td></tr><tr><td>Reference Asset</td><td>PLTR</td></tr></table>
<table><tr><th>HOLDING</th><th>As at 2026/08/31</th></tr><tr><td>Palantir Technologies Inc.</td><td>128.2%</td></tr><tr><td>Written Options</td><td>(2.8)%</td></tr><tr><td>Cash and other assets and liabilities</td><td>(25.4)%</td></tr></table>"""

HARVEST_FOF_HTML = """<table><tr><th>HOLDINGS</th><th>As at 2026/08/31</th></tr><tr><td>Harvest Apple Enhanced High Income Shares ETF</td><td>7.0%</td></tr><tr><td>Harvest NVIDIA Enhanced High Income Shares ETF</td><td>6.9%</td></tr></table>"""

NINEPOINT_HTML = """<div><table><tr><td>Facts</td></tr><tr><td>Ticker</td><td>CCHI:TSX</td></tr><tr><td>Underlying Stock**</td><td>Cameco Corp. (CCO:TSX)</td></tr></table></div>"""

YAHOO_JSON = {"quoteSummary": {"result": [{"topHoldings": {
    "holdings": [{"symbol": "AAPL", "holdingName": "Apple Inc", "holdingPercent": {"raw": 0.07}}, {"symbol": "RY.TO", "holdingName": "Royal Bank of Canada", "holdingPercent": {"raw": 0.03}}],
    "sectorWeightings": [{"realestate": {"raw": 0.02}}, {"technology": {"raw": 0.30}}, {"financial_services": {"raw": 0.20}}],
}}]}}


class NamesTest(unittest.TestCase):
    def test_sector_names_fold_onto_one_set(self):
        self.assertEqual(exposure.norm_sector("Technology"), "Information Technology")
        self.assertEqual(exposure.norm_sector("Financial"), "Financials")
        self.assertEqual(exposure.norm_sector("Communication"), "Communication Services")   # iShares' holdings file
        self.assertEqual(exposure.norm_sector("Consumer, Non-cyclical"), "Consumer Staples")
        self.assertEqual(exposure.norm_sector("Basic Materials"), "Materials")
        self.assertEqual(exposure.norm_sector("Bitcoin Holding"), "Digital assets")
        self.assertEqual(exposure.norm_sector("Cash and/or Derivatives"), "", "cash is no sector")
        self.assertEqual(exposure.norm_sector("Aerospace"), "Aerospace", "a name outside the set passes through")

    def test_country_names_and_venues(self):
        self.assertEqual(exposure.norm_country("USA"), "United States")
        self.assertEqual(exposure.norm_country("Korea, Republic of"), "South Korea")
        self.assertEqual(exposure.venue_country("TSX-V"), "Canada")
        self.assertEqual(exposure.venue_country("NASDAQ"), "United States")
        self.assertEqual(exposure.venue_country("OPRA"), "")

    def test_issuer_of_a_fund_name(self):
        self.assertEqual(exposure.issuer_of("Vanguard All-Equity ETF Portfolio - ETF"), "vanguard")
        self.assertEqual(exposure.issuer_of("iShares Core Equity ETF Portfolio"), "ishares")
        self.assertEqual(exposure.issuer_of("Harvest Diversified High Income Shares ETF - Class A"), "harvest")
        self.assertEqual(exposure.issuer_of("Ninepoint Partners LP - Cameco Highshares ETF"), "ninepoint")
        self.assertEqual(exposure.issuer_of("Evolve All-in-One UltraYield ETF"), "evolve")
        self.assertEqual(exposure.issuer_of("Shopify Inc."), "")
        self.assertTrue(exposure.is_fund("Global X High Interest Savings ETF"))
        self.assertFalse(exposure.is_fund("Shopify Inc."))


class ParsersTest(unittest.TestCase):
    def test_ishares_holdings_csv(self):
        rows, as_of = exposure.parse_ishares_csv(ISHARES_CSV)
        self.assertEqual(as_of, "Sep 9, 2026")
        self.assertEqual([(r["ticker"], r["weight"], r["sector"], r["country"], r["fund"]) for r in rows],
                         [("RY", 9.73, "Financials", "Canada", False), ("SHOP", 5.21, "Information Technology", "Canada", False), ("XEF", 24.37, "", "Canada", True)],
                         "cash is out; a fund row is marked to be looked through")

    def test_evolve_page(self):
        sectors, holdings = exposure.parse_evolve_page(EVOLVE_HTML)
        self.assertEqual(sectors, {"Information Technology": 27.86, "Financials": 24.63, "Communication Services": 15.72})
        self.assertEqual([(h["ticker"], h["weight"], h["sector"], h["country"]) for h in holdings],
                         [("MSFT", 4.11, "Information Technology", "United States"), ("RY", 2.0, "Financials", "Canada")],
                         "a sub-fund code in the country column is not a country; the Bloomberg market code gives it")

    def test_harvest_tables(self):
        rows, ref = exposure.parse_harvest_tables(exposure.html_tables(HARVEST_TABLE_HTML))
        self.assertEqual(ref, "")
        self.assertEqual([(h["ticker"], h["weight"], h["sector"], h["country"]) for h in rows], [("IBIT", 130.3, "Digital assets", "United States")], "options and cash rows are out")
        rows, ref = exposure.parse_harvest_tables(exposure.html_tables(HARVEST_NAMES_HTML))
        self.assertEqual(ref, "PLTR")
        self.assertEqual([(h["name"], h["weight"], h["fund"]) for h in rows], [("Palantir Technologies Inc.", 128.2, False)])
        rows, ref = exposure.parse_harvest_tables(exposure.html_tables(HARVEST_FOF_HTML))
        self.assertEqual([(h["name"], h["weight"], h["fund"]) for h in rows], [("Harvest Apple Enhanced High Income Shares ETF", 7.0, True), ("Harvest NVIDIA Enhanced High Income Shares ETF", 6.9, True)])

    def test_ninepoint_page(self):
        self.assertEqual(exposure.parse_ninepoint_page(NINEPOINT_HTML), ("CCHI", "CCO", "TSX"))

    def test_yahoo_summary(self):
        sectors, holdings = exposure.parse_yahoo_summary(YAHOO_JSON)
        self.assertEqual(sectors, {"Real Estate": 2.0, "Information Technology": 30.0, "Financials": 20.0})
        self.assertEqual([(h["ticker"], h["weight"], h["exchange"]) for h in holdings], [("AAPL", 7.0, ""), ("RY.TO", 3.0, "TSX")])


class LookthroughTest(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        os.environ["BAGHOLDER_HOME"] = self.tmp.name
        store.set_home(self.tmp.name)
        store.ensure()
        self.classified = {"RY": {"sector": "Financials", "industry": "Banking", "country": "Canada", "source": "TMX Money"},
                           "SHOP": {"sector": "Information Technology", "industry": "Software", "country": "Canada", "source": "TMX Money"},
                           "PLTR": {"sector": "Information Technology", "industry": "Software", "country": "United States", "source": "TMX Money"}}

    def tearDown(self):
        self.tmp.cleanup()
        os.environ.pop("BAGHOLDER_HOME", None)

    def _classify(self, symbol, exchange="", currency=""):
        return dict(self.classified.get(symbol.upper(), {"sector": "", "industry": "", "country": exposure.venue_country(exchange), "source": ""}))

    def test_holdings_are_spread_by_weight_and_the_rest_is_unclassified(self):
        rows = [{"ticker": "RY", "name": "", "weight": 60, "sector": "", "country": "", "exchange": "TSX", "currency": "CAD", "fund": False},
                {"ticker": "ZZZ", "name": "", "weight": 20, "sector": "", "country": "", "exchange": "", "currency": "", "fund": False},
                {"ticker": "SHOP", "name": "", "weight": 20, "sector": "Information Technology", "country": "Canada", "exchange": "", "currency": "", "fund": False}]
        with mock.patch.object(exposure, "classify_share", side_effect=self._classify):
            agg = exposure.lookthrough(rows)
        self.assertAlmostEqual(agg["sectors"]["Financials"], 0.6)
        self.assertAlmostEqual(agg["sectors"]["Information Technology"], 0.2)
        self.assertAlmostEqual(agg["countries"]["Canada"], 0.8)
        self.assertAlmostEqual(agg["coverage"], 0.8, msg="ZZZ has no record: its fifth is unclassified")

    def test_a_fund_held_by_a_fund_is_looked_through(self):
        def adapter(symbol, name, exchange):
            if symbol == "OUTER":
                return {"sectors": {}, "countries": {}, "holdings": [{"ticker": "INNER", "name": "Inner Index ETF", "weight": 50, "sector": "", "country": "", "exchange": "TSX", "currency": "CAD", "fund": True},
                                                                     {"ticker": "PLTR", "name": "", "weight": 50, "sector": "", "country": "", "exchange": "NYSE", "currency": "USD", "fund": False}], "source": "t", "asOf": ""}
            if symbol == "INNER":
                return {"sectors": {}, "countries": {}, "holdings": [{"ticker": "RY", "name": "", "weight": 100, "sector": "", "country": "", "exchange": "TSX", "currency": "CAD", "fund": False}], "source": "t", "asOf": ""}
            return None
        with mock.patch.object(exposure, "classify_share", side_effect=self._classify), mock.patch.dict(exposure.ADAPTERS, {"test": adapter}), \
             mock.patch.object(exposure, "issuer_of", return_value="test"):
            rec = exposure.fund_exposure("OUTER", "Test Outer ETF", "TSX")
        self.assertAlmostEqual(rec["sectors"]["Financials"], 0.5)
        self.assertAlmostEqual(rec["sectors"]["Information Technology"], 0.5)
        self.assertAlmostEqual(rec["countries"]["Canada"], 0.5)
        self.assertAlmostEqual(rec["countries"]["United States"], 0.5)
        self.assertEqual(rec["coverage"], 1.0)
        self.assertEqual(store.exposure_record("fund:INNER")["coverage"], 1.0, "the inner fund's record is kept for the next fund that holds it")

    def test_a_holding_stated_with_sector_and_country_is_taken_as_stated(self):
        rows = [{"ticker": "IBIT", "name": "iShares Bitcoin Trust ETF", "weight": 130.3, "sector": "Digital assets", "country": "United States", "exchange": "", "currency": "", "fund": True}]
        with mock.patch.object(exposure, "fund_exposure", side_effect=AssertionError("must not look it through")), mock.patch.object(exposure, "classify_share", side_effect=AssertionError("must not classify")):
            agg = exposure.lookthrough(rows)
        self.assertEqual((agg["sectors"], agg["countries"], agg["coverage"]), ({"Digital assets": 1.0}, {"United States": 1.0}, 1.0))

    def test_a_fund_named_without_a_ticker_is_resolved_then_looked_through(self):
        seen_calls = []
        def adapter(symbol, name, exchange):
            seen_calls.append(symbol)
            if symbol == "APLE":
                return {"sectors": {}, "countries": {}, "holdings": [{"ticker": "AAPL", "name": "AAPL", "weight": 100.0, "sector": "", "country": "", "exchange": "", "currency": "USD", "fund": False}], "source": "t", "asOf": ""}
            return None
        rows = [{"ticker": "", "name": "Harvest Apple Enhanced High Income Shares ETF", "weight": 7.0, "sector": "", "country": "", "exchange": "", "currency": "", "fund": True}]
        classified = dict(self.classified, AAPL={"sector": "Information Technology", "industry": "Hardware", "country": "United States", "source": "TMX Money"})
        with mock.patch.object(exposure, "resolve_name", return_value={"symbol": "APLE", "exchange": "TSX", "currency": "CAD"}), mock.patch.dict(exposure.ADAPTERS, {"harvest": adapter}), \
             mock.patch.object(exposure, "classify_share", side_effect=lambda sym, ex="", ccy="": dict(classified.get(sym.upper(), {"sector": "", "industry": "", "country": "", "source": ""}))):
            agg = exposure.lookthrough(rows)
        self.assertEqual(seen_calls, ["APLE"], "the fund is looked through under the ticker the directory gave")
        self.assertEqual((agg["sectors"], agg["countries"]), ({"Information Technology": 1.0}, {"United States": 1.0}))

    def test_a_bare_ticker_answered_with_a_depositary_receipt_is_retried_as_the_us_listing(self):
        answers = {"PLTR": {"name": "Palantir CDR (CAD Hedged)", "sector": "Technology", "industry": "Software", "exchangeName": "Toronto Stock Exchange"},
                   "PLTR:US": {"name": "Palantir Technologies Inc.", "sector": "Technology", "industry": "Software", "exchangeName": "Nasdaq Global Select"}}
        with mock.patch.object(exposure, "_tmx_record", side_effect=lambda k: dict(answers.get(k, {}))), mock.patch.object(exposure.market, "tmx_lookup", side_effect=lambda key, fn: (fn(key), key)):
            c = exposure.classify_share("PLTR")
        self.assertEqual((c["sector"], c["country"]), ("Information Technology", "United States"))

    def test_a_family_without_an_adapter_falls_back_to_yahoo(self):
        with mock.patch.object(exposure, "classify_share", side_effect=self._classify), \
             mock.patch.object(exposure, "FALLBACK", side_effect=lambda s, n, e: {"sectors": {"Financials": 100.0}, "countries": {}, "holdings": [{"ticker": "RY", "name": "", "weight": 100, "sector": "", "country": "", "exchange": "TSX", "currency": "CAD", "fund": False}], "source": "Yahoo Finance", "asOf": ""}):
            rec = exposure.fund_exposure("ZZZ", "Someone Else Global Equity ETF", "TSX")
        self.assertEqual(rec["sectors"], {"Financials": 1.0}, "the fund's stated sectors")
        self.assertEqual(rec["countries"], {"Canada": 1.0}, "the countries from its named holdings")
        self.assertEqual(rec["source"], "Yahoo Finance")

    def test_a_fund_no_source_covers_is_stored_as_unclassified(self):
        with mock.patch.object(exposure, "FALLBACK", side_effect=OSError("down")):
            rec = exposure.refresh_security({"id": "sec-s-1", "symbol": "ZZZ", "name": "Nobody Fund ETF", "primaryExchange": "TSX", "currency": "CAD"})
        self.assertEqual((rec["sectors"], rec["countries"], rec["coverage"]), ({}, {}, 0.0))
        self.assertEqual(store.exposure_record("sec-s-1")["coverage"], 0.0)

    def test_a_share_is_its_one_sector_and_country(self):
        with mock.patch.object(exposure, "classify_share", side_effect=self._classify):
            rec = exposure.refresh_security({"id": "sec-s-ry", "symbol": "RY", "name": "Royal Bank of Canada", "primaryExchange": "TSX", "currency": "CAD"})
        self.assertEqual((rec["sectors"], rec["countries"], rec["coverage"]), ({"Financials": 1.0}, {"Canada": 1.0}, 1.0))
        self.assertEqual(exposure.stale(["sec-s-ry", "sec-s-none"]), ["sec-s-none"])


class PortfolioSlicesTest(unittest.TestCase):
    def test_positions_spread_by_their_records(self):
        positions = [{"mv": 1000.0, "currency": "CAD", "securityId": "a", "short": False, "kind": "Shares"},
                     {"mv": 500.0, "currency": "USD", "securityId": "b", "short": False, "kind": "Shares"},
                     {"mv": 300.0, "currency": "CAD", "securityId": "c", "short": False, "kind": "Shares"},
                     {"mv": 100.0, "currency": "CAD", "securityId": "a", "short": True, "kind": "Shares"},
                     {"mv": 200.0, "currency": "CAD", "securityId": "btc", "short": False, "kind": "Crypto"},
                     {"mv": 50.0, "currency": "USD", "securityId": "opt", "short": True, "kind": "Options", "underlying": "AAPL"}]
        exposures = {"a": {"sectors": {"Financials": 1.0}, "countries": {"Canada": 1.0}, "coverage": 1.0},
                     "b": {"sectors": {"Information Technology": 0.5, "Energy": 0.25}, "countries": {"United States": 0.75}, "coverage": 0.75},
                     "share:AAPL::US": {"sectors": {"Information Technology": 1.0}, "countries": {"United States": 1.0}, "coverage": 1.0}}
        cad = lambda v, c: v * (2.0 if c == "USD" else 1.0)
        sectors, regions = model.exposure_slices(positions, exposures, cad)
        self.assertEqual([(s["name"], round(s["value"], 2)) for s in sectors], [("Financials", 1100.0), ("Information Technology", 600.0), ("Energy", 250.0), ("Digital assets", 200.0), ("Not classified", 550.0)],
                         "the same positions as Allocation, the short included; the contract counts as its underlying; b's uncovered quarter and c, which has no record, are unclassified; the coin is Digital assets")
        self.assertAlmostEqual(sum(s["share"] for s in sectors), 1.0)
        self.assertEqual([(r["name"], round(r["value"], 2)) for r in regions], [("Canada", 1100.0), ("United States", 850.0), ("Not classified", 750.0)], "a coin has no country")

    def test_a_stored_alias_folds_when_read(self):
        positions = [{"mv": 100.0, "currency": "CAD", "securityId": "a", "short": False, "kind": "Shares"},
                     {"mv": 100.0, "currency": "CAD", "securityId": "b", "short": False, "kind": "Shares"}]
        exposures = {"a": {"sectors": {"Communication": 1.0}, "countries": {}, "coverage": 1.0},   # iShares' word, kept before the alias was known
                     "b": {"sectors": {"Communication Services": 1.0}, "countries": {}, "coverage": 1.0}}
        sectors, _ = model.exposure_slices(positions, exposures, lambda v, c: v)
        self.assertEqual([(s["name"], round(s["value"], 2)) for s in sectors], [("Communication Services", 200.0)])


if __name__ == "__main__":
    unittest.main()


class MarketsTest(unittest.TestCase):
    def test_heatmap_tiles_take_the_dominant_sector(self):
        positions = [{"id": "p1", "mv": 1000.0, "currency": "CAD", "securityId": "a", "short": False, "kind": "Shares", "symbol": "XEQT", "exchange": "TSX", "percentChange": 0.4},
                     {"id": "p2", "mv": 500.0, "currency": "USD", "securityId": "b", "short": False, "kind": "Shares", "symbol": "NVDA", "exchange": "NASDAQ", "percentChange": -1.2},
                     {"id": "p3", "mv": 200.0, "currency": "CAD", "securityId": "btc", "short": False, "kind": "Crypto", "symbol": "BTC", "exchange": "Crypto", "percentChange": None},
                     {"id": "p4", "mv": 50.0, "currency": "USD", "securityId": "opt", "short": False, "kind": "Options", "symbol": "AAPL 20DEC26 200.00 CALL", "underlying": "AAPL", "exchange": "", "percentChange": 3.0},
                     {"id": "p5", "mv": 0.0, "currency": "CAD", "securityId": "z", "short": False, "kind": "Shares", "symbol": "ZERO", "exchange": "TSX", "percentChange": None},
                     {"id": "p6", "mv": 250.0, "currency": "USD", "securityId": "b", "short": False, "kind": "Shares", "symbol": "NVDA", "exchange": "NASDAQ", "percentChange": -1.2}]
        exposures = {"a": {"sectors": {"Financials": 0.3, "Information Technology": 0.45, "Energy": 0.25}},
                     "b": {"sectors": {"Information Technology": 1.0}},
                     "share:AAPL::US": {"sectors": {"Information Technology": 1.0}}}
        tiles = model.heatmap_items(positions, exposures, lambda v, c: v * (2.0 if c == "USD" else 1.0))
        self.assertEqual([(t["symbol"], t["value"], t["sector"], t["percentChange"]) for t in tiles],
                         [("XEQT", 1000.0, "Information Technology", 0.4), ("NVDA", 1500.0, "Information Technology", -1.2), ("BTC", 200.0, "Digital assets", None), ("AAPL 20DEC26 200.00 CALL", 100.0, "Information Technology", 3.0)],
                         "a fund sits under the sector it weights most, a coin under Digital assets, a contract under its underlying; nothing worth nothing; a symbol held in two accounts is one tile")

    def test_watch_rows_carry_the_quote_the_sector_and_the_holding(self):
        base = {"quotes": {"SHOP@TSX": {"price": 212.06, "priceChange": 1.56, "percentChange": 0.74}},
                "exposures": {"share:SHOP:": {"sectors": {"Information Technology": 1.0}}},
                "watchlist": [{"symbol": "SHOP", "exchange": "TSX", "name": "Shopify Inc.", "currency": "CAD"}, {"symbol": "RKLB", "exchange": "NASDAQ", "name": "Rocket Lab", "currency": "USD"}]}
        positions = [{"id": "p9", "symbol": "SHOP", "exchange": "TSX"}]
        rows = model.watch_rows(base, positions)
        self.assertEqual([(r["symbol"], r["last"], r["percentChange"], r["sector"], r["positionId"]) for r in rows],
                         [("SHOP", 212.06, 0.74, "Information Technology", "p9"), ("RKLB", None, None, "Not classified", None)],
                         "a quote and a record when the app has them, dashes otherwise; the held one names its holding")
