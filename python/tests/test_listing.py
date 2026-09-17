"""One listing's own page: what the server answers for a listing whether or not the book holds it."""
from __future__ import annotations

import os
import sys
import tempfile
import unittest
from unittest import mock

sys.path.insert(0, os.path.dirname(os.path.dirname(os.path.abspath(__file__))))

import bagholder  # noqa: E402
import market  # noqa: E402
import model  # noqa: E402
import store  # noqa: E402

FILL = {"when": "2026-03-02T14:31:00Z", "side": "BUY", "qty": 10, "price": 5.0}
LATER = {"when": "2026-04-09T15:02:00Z", "side": "SELL", "qty": -10, "price": 6.5}
EARLIER = {"when": "2026-01-05T14:40:00Z", "side": "BUY", "qty": 4, "price": 4.0}


class ListingPageTest(unittest.TestCase):
    # its own store: the listing reads the security records, which are the person's own otherwise
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        os.environ["BAGHOLDER_HOME"] = self.tmp.name
        store.set_home(self.tmp.name)
        bagholder.set_home(self.tmp.name)
        store.ensure()

    def tearDown(self):
        self.tmp.cleanup()
        os.environ.pop("BAGHOLDER_HOME", None)


    def book(self, positions=(), trades=(), watchlist=()):
        return mock.patch.object(model, "base_model", return_value={"positions": list(positions), "trades": list(trades), "watchlist": list(watchlist)})

    def quote(self, price=1.25, change=-2.0):
        return mock.patch.object(market, "peek_quote", return_value={"price": price, "percentChange": change})

    def test_a_listing_the_book_holds_answers_with_the_holding_whose_page_it_is(self):
        held = [{"id": "rt:1", "symbol": "QNC", "exchange": "TSX-V", "currency": "CAD", "kind": "Shares", "name": "Quantum eMotion Corp"}]
        with self.book(positions=held), self.quote():
            out = bagholder.listing_payload("QNC", "TSX-V")
        self.assertEqual((out["ok"], out["positionId"]), (True, "rt:1"))

    def test_a_listing_traded_before_carries_the_executions_of_those_trades_in_time(self):
        trades = [{"id": "t2", "symbol": "QNC", "exchange": "TSX-V", "currency": "CAD", "kind": "Shares", "name": "Quantum eMotion Corp", "fills": [FILL, LATER]},
                  {"id": "t1", "symbol": "QNC", "exchange": "TSX-V", "currency": "CAD", "kind": "Shares", "fills": [EARLIER]},
                  {"id": "t3", "symbol": "QNC 16JAN26 5.00 CALL", "underlying": "QNC", "exchange": "TSX-V", "kind": "Options", "fills": [{"when": "2026-02-02T14:00:00Z", "side": "BUY", "qty": 1, "price": 1.1}]},
                  {"id": "t4", "symbol": "QNC", "exchange": "NYSE", "currency": "USD", "kind": "Shares", "fills": [{"when": "2026-02-03T14:00:00Z", "side": "BUY", "qty": 9, "price": 2.2}]}]
        with self.book(trades=trades), self.quote(price=1.8, change=1.5):
            out = bagholder.listing_payload("QNC", "TSX-V")
        self.assertIsNone(out.get("positionId"))
        self.assertEqual([f["when"] for f in out["fills"]], [EARLIER["when"], FILL["when"], LATER["when"]],
                         "the listing's own trades, oldest first; an option is not the share, and another venue is another listing")
        self.assertEqual((out["name"], out["exchange"], out["currency"], out["kind"]), ("Quantum eMotion Corp", "TSX-V", "CAD", "Shares"))
        self.assertEqual((out["price"], out["percentChange"]), (1.8, 1.5))

    def test_a_listing_never_traded_is_named_by_the_watchlist_and_has_no_executions(self):
        watch = [{"symbol": "YES", "exchange": "TSX-V", "currency": "CAD", "kind": "Shares", "name": "Char Technologies Ltd."}]
        with self.book(watchlist=watch), self.quote(price=0.265, change=0.0):
            out = bagholder.listing_payload("YES", "TSX-V")
        self.assertEqual(out["fills"], [])
        self.assertEqual((out["name"], out["currency"]), ("Char Technologies Ltd.", "CAD"))

    def test_a_listing_the_book_has_never_seen_answers_with_what_was_asked_for(self):
        with self.book(), self.quote(price=284.21, change=-0.34), mock.patch.object(bagholder, "_instrument_meta", return_value=("RY", "", "")):
            out = bagholder.listing_payload("RY", "TSX", "CAD", "Royal Bank of Canada")
        self.assertEqual((out["ok"], out["symbol"], out["exchange"], out["name"], out["fills"]), (True, "RY", "TSX", "Royal Bank of Canada", []))
        self.assertEqual(out["price"], 284.21)

    def test_a_ticker_with_no_venue_matches_the_book_whatever_venue_it_holds_it_on(self):
        trades = [{"id": "t1", "symbol": "SHOP.TO", "exchange": "TSX", "currency": "CAD", "kind": "Shares", "name": "Shopify Inc.", "fills": [FILL]}]
        with self.book(trades=trades), self.quote():
            out = bagholder.listing_payload("SHOP")
        self.assertEqual((out["symbol"], out["exchange"], out["name"]), ("SHOP", "TSX", "Shopify Inc."))
        self.assertEqual(len(out["fills"]), 1)

    def test_a_ticker_that_is_not_one_is_refused(self):
        with self.book():
            self.assertFalse(bagholder.listing_payload("  ")["ok"])


if __name__ == "__main__":
    unittest.main()
