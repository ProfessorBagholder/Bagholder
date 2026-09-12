"""Tests for the derived model (model.py), market data (market.py) and the
store tables and routes that back the v2 UI."""

from __future__ import annotations

import json
import os
import tempfile
import threading
import time
import unittest
from http.server import ThreadingHTTPServer
from unittest import mock
from urllib.request import Request, urlopen

import bagholder
import market
market.YAHOO_MIN_INTERVAL_SEC = 0   # tests never wait between mocked Yahoo calls
import model
import store


def act(**o):
    base = {
        "id": o.get("id") or "act-%s" % id(o),
        "accountId": "acct-1",
        "accountType": "Trading",
        "symbol": "LUNR 15JAN27 12.00 CALL",
        "name": "LUNR",
        "currency": "USD",
        "commission": 0,
        "category": "other",
        "activityType": "",
        "activitySubType": "",
        "rawType": "",
        "quantity": 0,
        "unitPrice": 0,
        "netCashAmount": 0,
        "transactionDate": "2026-01-01",
        "occurredAt": "",
        "securityId": "",
    }
    base.update(o)
    if not base["occurredAt"]:
        base["occurredAt"] = base["transactionDate"] + "T15:00:00+00:00"
    return base


def buy(id, symbol, qty, px, day, **extra):
    o = dict(
        id=id,
        category="trade",
        activityType="Trade",
        activitySubType="BUY",
        rawType="DIY_BUY",
        quantity=qty,
        unitPrice=px,
        netCashAmount=-qty * px,
        transactionDate=day,
        symbol=symbol,
        currency="CAD",
    )
    o.update(extra)
    return act(**o)


def sell(id, symbol, qty, px, day, **extra):
    o = dict(
        id=id,
        category="trade",
        activityType="Trade",
        activitySubType="SELL",
        rawType="DIY_SELL",
        quantity=-qty,
        unitPrice=px,
        netCashAmount=qty * px,
        transactionDate=day,
        symbol=symbol,
        currency="CAD",
    )
    o.update(extra)
    return act(**o)


class FifoPortTest(unittest.TestCase):
    """Scenarios ported one-for-one from the ledger.html engine tests."""

    def test_multileg_zero_qty_closes_short(self):
        lunr = [
            act(id="sto", category="trade", activityType="OPTIONS_SELL", activitySubType="SELLTOOPEN",
                rawType="OPTIONS_SELL", quantity=-16, unitPrice=6.2225, netCashAmount=9956, transactionDate="2026-01-10"),
            act(id="ml1", activityType="OPTIONS_MULTILEG", activitySubType="FILLED", rawType="OPTIONS_MULTILEG",
                quantity=0, netCashAmount=-128, transactionDate="2026-03-01"),
            act(id="ml2", activityType="OPTIONS_MULTILEG", activitySubType="FILLED", rawType="OPTIONS_MULTILEG",
                quantity=0, netCashAmount=-2025, transactionDate="2026-03-01"),
        ]
        r = model.match_fifo(lunr)
        self.assertEqual(r["open"], [])
        real = [t for t in r["closed"] if "rolled-out" not in t["flags"]]
        self.assertEqual(len(real), 2)
        by_qty = sorted(real, key=lambda t: t["quantity"])
        self.assertEqual(by_qty[0]["quantity"], 1)
        self.assertAlmostEqual(by_qty[0]["exitPrice"], 1.28)
        self.assertEqual(by_qty[1]["quantity"], 15)
        self.assertAlmostEqual(by_qty[1]["exitPrice"], 1.35)
        self.assertTrue(all(t["openDirection"] == "SHORT" for t in r["closed"]))
        want = (6.2225 - 1.28) * 1 * 100 + (6.2225 - 1.35) * 15 * 100
        self.assertAlmostEqual(sum(t["pnl"] for t in r["closed"]), want)
        self.assertTrue(all(t["rt"] == "rt:sto" for t in real))

    def test_roll_carries_the_unposted_leg_to_the_next_buy_back(self):
        # STO 16 Jan27 calls; roll to Jan28 (only the closing leg is posted);
        # STO 6 more Jan28; buy back all 22. Nothing stays open.
        acts = [
            act(id="sto", category="trade", activityType="OPTIONS_SELL", activitySubType="SELLTOOPEN", rawType="OPTIONS_SELL",
                quantity=-16, unitPrice=6.2225, netCashAmount=9956, transactionDate="2025-10-01", symbol="LUNR 15JAN27 12.00 CALL"),
            act(id="ml", activityType="OPTIONS_MULTILEG", activitySubType="FILLED", rawType="OPTIONS_MULTILEG",
                quantity=0, netCashAmount=-2160, transactionDate="2025-11-14", symbol="LUNR 15JAN27 12.00 CALL"),
            act(id="sto2", category="trade", activityType="OPTIONS_SELL", activitySubType="SELLTOOPEN", rawType="OPTIONS_SELL",
                quantity=-6, unitPrice=6.75, netCashAmount=4050, transactionDate="2025-12-10", symbol="LUNR 21JAN28 12.00 CALL"),
            act(id="btc", category="trade", activityType="OPTIONS_BUY", activitySubType="BUYTOOPEN", rawType="OPTIONS_BUY",
                quantity=22, unitPrice=13.3, netCashAmount=-29260, transactionDate="2026-06-26", symbol="LUNR 21JAN28 12.00 CALL"),
        ]
        r = model.match_fifo(acts)
        self.assertEqual(r["unmatched"], [])
        self.assertEqual(r["open"], [])
        total = sum(t["pnl"] for t in r["closed"])
        self.assertAlmostEqual(total, 9956 - 2160 + 4050 - 29260)
        rolled_in = [t for t in r["closed"] if "rolled-in" in t["flags"]]
        self.assertAlmostEqual(sum(t["quantity"] for t in rolled_in), 16)
        self.assertTrue(all(t["symbol"] == "LUNR 21JAN28 12.00 CALL" for t in rolled_in))
        # everything the buy-back closed is one position, so one trade row
        jan28 = {t["rt"] for t in r["closed"] if t["symbol"] == "LUNR 21JAN28 12.00 CALL"}
        self.assertEqual(len(jan28), 1)

    def test_credit_roll_up_moves_shorts_to_the_new_strike(self):
        # 5 short 10 calls rolled up to 12 calls for a credit (two multileg fills
        # posted on the 10 call), then the 12 calls are bought back.
        acts = [
            act(id="sto", category="trade", activityType="OPTIONS_SELL", activitySubType="SELLTOOPEN", rawType="OPTIONS_SELL",
                quantity=-5, unitPrice=3.0, netCashAmount=1500, transactionDate="2025-11-12", symbol="BBAI 21JAN28 10.00 CALL"),
            act(id="cr1", activityType="OPTIONS_MULTILEG", activitySubType="FILLED", rawType="OPTIONS_MULTILEG",
                quantity=0, netCashAmount=14, transactionDate="2026-06-09", symbol="BBAI 21JAN28 10.00 CALL"),
            act(id="cr2", activityType="OPTIONS_MULTILEG", activitySubType="FILLED", rawType="OPTIONS_MULTILEG",
                quantity=0, netCashAmount=56, transactionDate="2026-06-17", symbol="BBAI 21JAN28 10.00 CALL"),
            act(id="btc", category="trade", activityType="OPTIONS_BUY", activitySubType="BUYTOOPEN", rawType="OPTIONS_BUY",
                quantity=5, unitPrice=0.85, netCashAmount=-425, transactionDate="2026-06-26", symbol="BBAI 21JAN28 12.00 CALL"),
        ]
        r = model.match_fifo(acts)
        self.assertEqual(r["unmatched"], [])
        self.assertEqual(r["open"], [])
        self.assertAlmostEqual(sum(t["pnl"] for t in r["closed"]), 1500 + 14 + 56 - 425)

    def test_buy_back_closes_older_contracts_of_a_rolled_chain(self):
        # Short Dec puts rolled forward (only one leg posted, tagged with a contract
        # never opened); the June buy-back of 26 closes 11 known shorts, the
        # carried leg and the 9 old Dec puts, and nothing stays open.
        acts = [
            act(id="s1", category="trade", activityType="OPTIONS_SELL", activitySubType="SELLTOOPEN", rawType="OPTIONS_SELL", quantity=-3, unitPrice=0.12, netCashAmount=36, transactionDate="2025-12-05", symbol="BBAI 26DEC25 5.50 PUT"),
            act(id="s2", category="trade", activityType="OPTIONS_SELL", activitySubType="SELLTOOPEN", rawType="OPTIONS_SELL", quantity=-5, unitPrice=0.2, netCashAmount=100, transactionDate="2025-12-11", symbol="BBAI 02JAN26 5.50 PUT"),
            act(id="s3", category="trade", activityType="OPTIONS_SELL", activitySubType="SELLTOOPEN", rawType="OPTIONS_SELL", quantity=-1, unitPrice=0.4, netCashAmount=40, transactionDate="2025-12-15", symbol="BBAI 26DEC25 6.00 PUT"),
            act(id="s4", category="trade", activityType="OPTIONS_SELL", activitySubType="SELLTOOPEN", rawType="OPTIONS_SELL", quantity=-6, unitPrice=0.2, netCashAmount=120, transactionDate="2025-12-12", symbol="BBAI 19DEC25 6.00 PUT"),
            act(id="ml1", activityType="OPTIONS_MULTILEG", activitySubType="FILLED", rawType="OPTIONS_MULTILEG", quantity=0, netCashAmount=-18, transactionDate="2025-12-15", symbol="BBAI 19DEC25 6.00 PUT"),
            act(id="ml2", activityType="OPTIONS_MULTILEG", activitySubType="FILLED", rawType="OPTIONS_MULTILEG", quantity=0, netCashAmount=-1830, transactionDate="2025-12-18", symbol="BBAI 18JUN26 5.00 PUT"),
            act(id="s5", category="trade", activityType="OPTIONS_SELL", activitySubType="SELLTOOPEN", rawType="OPTIONS_SELL", quantity=-11, unitPrice=2.4, netCashAmount=2640, transactionDate="2026-02-27", symbol="BBAI 21JAN28 5.00 PUT"),
            act(id="btc", category="trade", activityType="OPTIONS_BUY", activitySubType="BUYTOOPEN", rawType="OPTIONS_BUY", quantity=26, unitPrice=2.74, netCashAmount=-7124, transactionDate="2026-06-29", symbol="BBAI 21JAN28 5.00 PUT"),
        ]
        r = model.match_fifo(acts)
        self.assertEqual(r["unmatched"], [])
        self.assertEqual(r["open"], [])
        self.assertAlmostEqual(sum(t["pnl"] for t in r["closed"]), 36 + 100 + 40 + 120 - 18 - 1830 + 2640 - 7124)
        self.assertTrue(all(t["symbol"] == "BBAI 21JAN28 5.00 PUT" for t in r["closed"] if t["exitDate"] == "2026-06-29"))

    def test_plain_option_buys_without_a_roll_stay_long(self):
        r = model.match_fifo([
            act(id="bto", category="trade", activityType="OPTIONS_BUY", activitySubType="BUYTOOPEN", rawType="OPTIONS_BUY",
                quantity=10, unitPrice=1.27, netCashAmount=-1270, transactionDate="2026-06-15", symbol="QNC 20NOV26 3.00 CALL"),
        ])
        self.assertEqual(len(r["open"]), 1)
        self.assertEqual(r["open"][0]["direction"], "LONG")

    def test_short_expiry_closes_short(self):
        r = model.match_fifo([
            act(id="sto2", category="trade", activityType="OPTIONS_SELL", activitySubType="SELLTOOPEN",
                rawType="OPTIONS_SELL", quantity=-5, unitPrice=2, netCashAmount=1000, transactionDate="2026-01-10",
                symbol="ABC 15JAN27 10.00 CALL"),
            act(id="exp", activityType="OPTIONS_SHORT_EXPIRY", activitySubType="EXPIRED", rawType="OPTIONS_SHORT_EXPIRY",
                quantity=5, transactionDate="2027-01-15", symbol="ABC 15JAN27 10.00 CALL"),
        ])
        self.assertEqual(r["open"], [])
        self.assertEqual(len(r["closed"]), 1)
        self.assertEqual(r["closed"][0]["exitPrice"], 0)
        self.assertEqual(r["closed"][0]["quantity"], 5)
        self.assertAlmostEqual(r["closed"][0]["pnl"], 1000)

    def test_shares_round_trip(self):
        r = model.match_fifo([buy("b", "AAA", 10, 12, "2026-01-10"), sell("s", "AAA", 10, 15, "2026-02-10")])
        self.assertEqual(len(r["closed"]), 1)
        self.assertEqual(r["closed"][0]["quantity"], 10)
        self.assertAlmostEqual(r["closed"][0]["pnl"], 30)
        self.assertEqual(r["closed"][0]["holdDays"], 31)
        self.assertFalse(model.is_option_symbol("AAA"))
        self.assertTrue(model.is_option_symbol("LUNR 15JAN27 12.00 CALL"))

    def test_credit_multilegs_on_a_short_are_a_roll(self):
        r = model.match_fifo([
            act(id="bbai-sto", category="trade", activityType="OPTIONS_SELL", activitySubType="SELLTOOPEN",
                rawType="OPTIONS_SELL", quantity=-3, unitPrice=1.2, netCashAmount=360, transactionDate="2026-01-05",
                symbol="BBAI 21JAN28 10.00 CALL"),
            act(id="bbai-cr1", category="trade", activityType="OPTIONS_SELL", activitySubType="SELLTOCLOSE",
                rawType="OPTIONS_MULTILEG", quantity=0, netCashAmount=14, transactionDate="2026-02-01",
                symbol="BBAI 21JAN28 10.00 CALL"),
            act(id="bbai-cr2", activityType="OPTIONS_MULTILEG", activitySubType="FILLED", rawType="OPTIONS_MULTILEG",
                quantity=0, netCashAmount=56, transactionDate="2026-02-01", symbol="BBAI 21JAN28 10.00 CALL"),
        ])
        self.assertEqual(r["unmatched"], [])
        self.assertEqual(r["open"], [])
        self.assertAlmostEqual(sum(t["pnl"] for t in r["closed"]), 360 + 14 + 56)

    def test_long_expiry_and_same_day_expiry(self):
        r = model.match_fifo([
            act(id="lunr-bto", category="trade", activityType="OPTIONS_BUY", activitySubType="BUYTOOPEN",
                rawType="OPTIONS_BUY", quantity=2, unitPrice=0.4, netCashAmount=-80, transactionDate="2025-07-01",
                symbol="LUNR 22AUG25 8.00 CALL"),
            act(id="lunr-exp", category="option_event", activityType="EXPIR", activitySubType="BUY",
                rawType="OPTIONS_EXPIRY", quantity=2, transactionDate="2025-08-22", symbol="LUNR 22AUG25 8.00 CALL"),
        ])
        self.assertEqual(r["unmatched"], [])
        self.assertEqual(r["open"], [])
        self.assertEqual(len(r["closed"]), 1)
        self.assertEqual(r["closed"][0]["openDirection"], "LONG")
        self.assertAlmostEqual(r["closed"][0]["pnl"], -80)
        r = model.match_fifo([
            act(id="spy-bto", category="trade", activityType="OPTIONS_BUY", activitySubType="BUYTOOPEN",
                rawType="OPTIONS_BUY", quantity=1, unitPrice=1.1, netCashAmount=-110, transactionDate="2025-07-17",
                symbol="SPY 17JUL25 624.00 PUT"),
            act(id="spy-exp", activityType="OPTIONS_EXPIRY", activitySubType="EXPIRED", rawType="OPTIONS_EXPIRY",
                quantity=1, transactionDate="2025-07-17", symbol="SPY 17JUL25 624.00 PUT"),
        ])
        self.assertEqual(r["unmatched"], [])
        self.assertEqual(r["open"], [])
        self.assertEqual(len(r["closed"]), 1)

    def test_debit_multileg_opens_long_and_sto_opens_short(self):
        r = model.match_fifo([
            act(id="put-ml", activityType="OPTIONS_MULTILEG", activitySubType="FILLED", rawType="OPTIONS_MULTILEG",
                quantity=0, netCashAmount=-90, transactionDate="2026-01-30", symbol="BBAI 30JAN26 6.00 PUT"),
        ])
        self.assertEqual(r["unmatched"], [])
        self.assertEqual(len(r["open"]), 1)
        self.assertEqual(r["open"][0]["direction"], "LONG")
        r = model.match_fifo([
            act(id="sto-only", category="trade", activityType="OPTIONS_SELL", activitySubType="SELLTOOPEN",
                rawType="OPTIONS_SELL", quantity=-4, unitPrice=2, netCashAmount=800, transactionDate="2026-01-01",
                symbol="XYZ 15JAN27 5.00 CALL"),
        ])
        self.assertEqual(r["unmatched"], [])
        self.assertEqual(len(r["open"]), 1)
        self.assertEqual(r["open"][0]["direction"], "SHORT")
        self.assertEqual(r["open"][0]["qty"], 4)

    def test_assignment_keeps_premium(self):
        r = model.match_fifo([
            act(id="asts-sto", category="trade", activityType="OPTIONS_SELL", activitySubType="SELLTOOPEN",
                rawType="OPTIONS_SELL", quantity=-1, unitPrice=4.7475, netCashAmount=474.75, transactionDate="2025-01-15",
                symbol="ASTS 07MAR25 31.00 CALL"),
            act(id="asts-asg", category="option_event", activityType="ASSIGN", activitySubType="BUYTOCLOSE",
                rawType="OPTIONS_ASSIGN", quantity=1, unitPrice=31, netCashAmount=-3100, transactionDate="2025-03-07",
                symbol="ASTS 07MAR25 31.00 CALL"),
        ])
        self.assertEqual(r["unmatched"], [])
        self.assertEqual(r["open"], [])
        self.assertEqual(len(r["closed"]), 1)
        self.assertEqual(r["closed"][0]["exitPrice"], 0)
        self.assertAlmostEqual(r["closed"][0]["pnl"], 474.75)

    def test_same_day_roll_folds_into_far_contract(self):
        r = model.match_fifo([
            act(id="aug-sto", category="trade", activityType="OPTIONS_SELL", activitySubType="SELLTOOPEN",
                rawType="OPTIONS_SELL", quantity=-1, unitPrice=3, netCashAmount=300, transactionDate="2026-01-01",
                symbol="ZZZ 21AUG26 10.00 CALL"),
            act(id="aug-cover", category="trade", activityType="OPTIONS_BUY", activitySubType="BUYTOCLOSE",
                rawType="OPTIONS_BUY", quantity=1, unitPrice=1, netCashAmount=-100, transactionDate="2026-08-15",
                symbol="ZZZ 21AUG26 10.00 CALL"),
            act(id="jan-sto", category="trade", activityType="OPTIONS_SELL", activitySubType="SELLTOOPEN",
                rawType="OPTIONS_SELL", quantity=-1, unitPrice=2, netCashAmount=200, transactionDate="2026-08-15",
                symbol="ZZZ 15JAN27 12.00 CALL"),
            act(id="jan-cover", category="trade", activityType="OPTIONS_BUY", activitySubType="BUYTOCLOSE",
                rawType="OPTIONS_BUY", quantity=1, unitPrice=0.5, netCashAmount=-50, transactionDate="2026-12-01",
                symbol="ZZZ 15JAN27 12.00 CALL"),
        ])
        self.assertEqual(r["unmatched"], [])
        self.assertEqual(r["open"], [])
        self.assertEqual(len(r["closed"]), 1)
        self.assertEqual(r["closed"][0]["symbol"], "ZZZ 15JAN27 12.00 CALL")
        self.assertAlmostEqual(r["closed"][0]["entryPrice"], 4)
        self.assertAlmostEqual(r["closed"][0]["pnl"], 350)
        self.assertIn("rolled", r["closed"][0]["flags"])

    def test_stkdis_name_change_nets_to_zero(self):
        r = model.match_fifo([
            buy("b", "OLD", 100, 2, "2026-01-01"),
            act(id="out", category="trade", activityType="STKDIS", activitySubType="SELL", rawType="CORPORATE_ACTION",
                quantity=-100, transactionDate="2026-02-01", symbol="OLD", currency="CAD"),
            act(id="in", category="trade", activityType="STKDIS", activitySubType="BUY", rawType="CORPORATE_ACTION",
                quantity=100, transactionDate="2026-02-01", symbol="NEW", currency="CAD"),
            sell("s", "NEW", 100, 3, "2026-03-01"),
        ])
        # Parity with ledger.html: the +N leg opens NEW at $0 and the sell
        # closes it; the OLD lot is only reused when NEW runs out of lots.
        self.assertEqual(r["unmatched"], [])
        self.assertEqual(len(r["closed"]), 1)
        self.assertEqual(r["closed"][0]["symbol"], "NEW")
        self.assertAlmostEqual(r["closed"][0]["pnl"], 300)
        self.assertEqual([l["symbol"] for l in r["open"]], ["OLD"])
        r = model.match_fifo([
            buy("b", "OLD", 100, 2, "2026-01-01"),
            act(id="out", category="trade", activityType="STKDIS", activitySubType="SELL", rawType="CODE_CHANGE",
                quantity=-100, transactionDate="2026-02-01", symbol="OLD", currency="CAD"),
            sell("s", "NEW", 100, 3, "2026-03-01"),
        ])
        self.assertEqual(r["unmatched"], [])
        self.assertEqual(len(r["closed"]), 1)
        self.assertEqual(r["closed"][0]["symbol"], "NEW")
        self.assertAlmostEqual(r["closed"][0]["pnl"], 100)
        self.assertEqual(r["open"], [])


class SplitTest(unittest.TestCase):
    def test_reverse_split_marker_rescales_open_lots(self):
        acts = [
            buy("b1", "MSTY", 100, 7.0, "2025-12-01"),
            buy("b2", "MSTY", 75, 6.9, "2025-12-05"),
            act(id="ca", category="trade", activityType="STKDIS", activitySubType="BUY", rawType="CORPORATE_ACTION",
                quantity=0, transactionDate="2025-12-08", symbol="MSTY", currency="CAD"),
            buy("b3", "MSTY", 4, 34.0, "2025-12-11"),
            sell("s1", "MSTY", 39, 31.0, "2026-01-16"),
        ]
        r = model.match_fifo(model.normalize_activities(acts))
        self.assertEqual(r["unmatched"], [])
        self.assertEqual(r["open"], [])
        self.assertAlmostEqual(sum(t["quantity"] for t in r["closed"]), 39)
        first = min(r["closed"], key=lambda t: t["entryDate"])
        self.assertAlmostEqual(first["entryPrice"], 35.0)
        self.assertAlmostEqual(sum(t["pnl"] for t in r["closed"]), 39 * 31 - (100 * 7 + 75 * 6.9 + 4 * 34))

    def test_forward_split_and_no_marker_without_prices(self):
        acts = [
            buy("b1", "NVDA", 10, 1000.0, "2024-05-01"),
            act(id="ca", category="trade", activityType="STKDIS", activitySubType="BUY", rawType="CORPORATE_ACTION",
                quantity=0, transactionDate="2024-06-10", symbol="NVDA", currency="CAD"),
            buy("b2", "NVDA", 5, 98.0, "2024-06-12", currency="USD"),
        ]
        acts[0]["currency"] = "USD"
        r = model.match_fifo(model.normalize_activities(acts))
        self.assertAlmostEqual(sum(l["qty"] for l in r["open"]), 105)
        big = max(r["open"], key=lambda l: l["qty"])
        self.assertAlmostEqual(big["price"], 100.0)
        self.assertIn("split 10:1", big["flags"])
        r = model.match_fifo(model.normalize_activities([
            buy("b1", "AAA", 10, 10.0, "2024-05-01"),
            act(id="ca", category="trade", activityType="STKDIS", activitySubType="BUY", rawType="CORPORATE_ACTION",
                quantity=0, transactionDate="2024-06-10", symbol="AAA", currency="CAD"),
        ]))
        self.assertEqual(r["open"][0]["qty"], 10)


class RoundTripTest(unittest.TestCase):
    def _trades(self, acts, groups=None, journal=None):
        norm = model.normalize_activities(acts)
        fifo = model.match_fifo(norm)
        model.apply_fx(fifo["closed"], {})
        by_id = {a["id"]: a for a in norm}
        return model.build_trades(fifo["closed"], fifo["open"], groups or [], by_id, model.Securities([]), journal or {})

    def test_flat_to_flat_twice_is_two_trades(self):
        trades = self._trades([
            buy("b1", "AAA", 100, 10, "2026-01-01"),
            sell("s1", "AAA", 100, 12, "2026-01-10"),
            buy("b2", "AAA", 50, 11, "2026-02-01"),
            sell("s2", "AAA", 50, 9, "2026-02-10"),
        ])
        self.assertEqual(len(trades), 2)
        self.assertEqual({t["id"] for t in trades}, {"rt:b1", "rt:b2"})
        self.assertTrue(all(t["status"] == "closed" for t in trades))
        pnl = {t["id"]: t["pnl"] for t in trades}
        self.assertAlmostEqual(pnl["rt:b1"], 200)
        self.assertAlmostEqual(pnl["rt:b2"], -100)

    def test_scale_in_and_out_is_one_trade_with_legs(self):
        trades = self._trades([
            buy("b1", "AAA", 100, 10, "2026-01-01"),
            sell("s1", "AAA", 50, 12, "2026-01-10"),
            buy("b2", "AAA", 100, 11, "2026-01-15"),
            sell("s2", "AAA", 150, 13, "2026-02-01"),
        ])
        self.assertEqual(len(trades), 1)
        t = trades[0]
        self.assertEqual(t["id"], "rt:b1")
        self.assertEqual(t["status"], "closed")
        self.assertEqual(t["qty"], 200)
        self.assertEqual(t["legCount"], 3)
        self.assertEqual(t["entryDate"], "2026-01-01")
        self.assertEqual(t["exitDate"], "2026-02-01")
        self.assertAlmostEqual(t["pnl"], 50 * 2 + 50 * 3 + 100 * 2)
        self.assertEqual(len(t["fills"]), 4)
        self.assertEqual(t["opened"]["fills"], 2)
        self.assertEqual(t["closed"]["fills"], 2)
        self.assertEqual(t["side"], "SELL")

    def test_partial_exit_is_a_closed_trade_with_stable_id(self):
        acts = [buy("b1", "AAA", 100, 10, "2026-01-01"), sell("s1", "AAA", 40, 12, "2026-01-10")]
        trades = self._trades(acts)
        self.assertEqual(len(trades), 1)
        self.assertEqual(trades[0]["status"], "closed")
        self.assertEqual(trades[0]["id"], "rt:b1")
        self.assertEqual(trades[0]["qty"], 40)
        acts.append(sell("s2", "AAA", 60, 15, "2026-02-01"))
        trades = self._trades(acts)
        self.assertEqual(len(trades), 1)
        self.assertEqual(trades[0]["status"], "closed")
        self.assertEqual(trades[0]["id"], "rt:b1")
        self.assertEqual(trades[0]["qty"], 100)

    def test_saved_group_overrides_round_trip(self):
        acts = [
            buy("b1", "AAA", 100, 10, "2026-01-01"),
            sell("s1", "AAA", 100, 12, "2026-01-10"),
            buy("b2", "AAA", 50, 11, "2026-02-01"),
            sell("s2", "AAA", 50, 9, "2026-02-10"),
        ]
        key1 = "b1|s1|%s" % ("%.8f" % 100)
        key2 = "b2|s2|%s" % ("%.8f" % 50)
        trades = self._trades(acts, groups=[{"id": "g_manual", "locked": True, "members": [key1, key2]}])
        self.assertEqual(len(trades), 1)
        self.assertEqual(trades[0]["id"], "g_manual")
        self.assertTrue(trades[0]["locked"])
        self.assertEqual(trades[0]["legCount"], 2)

    def test_position_notes_carry_over_to_the_closed_trade(self):
        snapshot = {
            "activities": [buy("b1", "AAA", 100, 10, "2026-01-01")],
            "accounts": [], "balances": [], "navHistory": [], "navByAccount": {}, "syncedAt": "", "tradeGroups": [], "notes": {}, "securities": [],
        }
        base = model.build_base(snapshot, {"fx": {}, "benchmark": {}}, {}, today="2026-02-01")
        pid = base["positions"][0]["id"]
        self.assertEqual(pid, "rt:b1")
        journal = {pid: {"thesis": "holding for the catalyst", "tags": ["core"], "grade": ""}}
        base = model.build_base(snapshot, {"fx": {}, "benchmark": {}}, journal, today="2026-02-01")
        self.assertEqual(base["positions"][0]["thesis"], "holding for the catalyst")
        snapshot["activities"].append(sell("s1", "AAA", 100, 12, "2026-03-01"))
        base = model.build_base(snapshot, {"fx": {}, "benchmark": {}}, journal, today="2026-04-01")
        self.assertEqual(base["positions"], [])
        self.assertEqual(base["trades"][0]["id"], "rt:b1")
        self.assertEqual(base["trades"][0]["thesis"], "holding for the catalyst")
        self.assertEqual(base["trades"][0]["tags"], ["core"])

    def test_journal_attaches_to_trade(self):
        trades = self._trades(
            [buy("b1", "AAA", 100, 10, "2026-01-01"), sell("s1", "AAA", 100, 12, "2026-01-10")],
            journal={"rt:b1": {"thesis": "breakout", "tags": ["momo"], "grade": "A"}},
        )
        self.assertEqual(trades[0]["grade"], "A")
        self.assertEqual(trades[0]["tags"], ["momo"])
        self.assertEqual(trades[0]["thesis"], "breakout")

    def test_fill_labels_reflect_what_the_fill_did(self):
        trades = self._trades([
            act(id="sto", category="trade", activityType="OPTIONS_SELL", activitySubType="SELLTOOPEN", rawType="OPTIONS_SELL",
                quantity=-2, unitPrice=3, netCashAmount=600, transactionDate="2026-01-01", symbol="ZZZ 21AUG26 10.00 CALL"),
            act(id="buy", category="trade", activityType="OPTIONS_BUY", activitySubType="BUYTOOPEN", rawType="OPTIONS_BUY",
                quantity=2, unitPrice=1, netCashAmount=-200, transactionDate="2026-02-01", symbol="ZZZ 21AUG26 10.00 CALL"),
        ])
        subs = {f["id"]: f["sub"] for f in trades[0]["fills"]}
        self.assertEqual(subs, {"sto": "SELL TO OPEN", "buy": "BUY TO CLOSE"})
        # Shares are bought and sold; the open/close order types are option language.
        trades = self._trades([buy("b1", "AAA", 100, 10, "2026-01-01"), sell("s1", "AAA", 100, 12, "2026-01-10")])
        self.assertEqual({f["id"]: f["sub"] for f in trades[0]["fills"]}, {"b1": "BUY", "s1": "SELL"})

    def test_short_round_trip_is_cover(self):
        trades = self._trades([
            act(id="sto", category="trade", activityType="OPTIONS_SELL", activitySubType="SELLTOOPEN", rawType="OPTIONS_SELL",
                quantity=-2, unitPrice=3, netCashAmount=600, transactionDate="2026-01-01", symbol="ZZZ 21AUG26 10.00 CALL"),
            act(id="btc", category="trade", activityType="OPTIONS_BUY", activitySubType="BUYTOCLOSE", rawType="OPTIONS_BUY",
                quantity=2, unitPrice=1, netCashAmount=-200, transactionDate="2026-02-01", symbol="ZZZ 21AUG26 10.00 CALL"),
        ])
        self.assertEqual(len(trades), 1)
        self.assertEqual(trades[0]["side"], "COVER")
        self.assertEqual(trades[0]["kind"], "Options")
        self.assertEqual(trades[0]["mult"], 100)
        self.assertAlmostEqual(trades[0]["pnl"], 400)
        self.assertAlmostEqual(trades[0]["pnlPct"], 400 / 600)


class ExpiryTest(unittest.TestCase):
    def test_option_expiry_parse(self):
        self.assertEqual(model.option_expiry("LUNR 29AUG25 11.50 CALL"), "2025-08-29")
        self.assertEqual(model.option_expiry("BBAI 02JAN26 5.50 PUT"), "2026-01-02")
        self.assertEqual(model.option_expiry("AAPL"), "")

    def test_open_option_past_expiry_is_closed_at_zero(self):
        snapshot = {
            "activities": [
                act(id="sto", category="trade", activityType="OPTIONS_SELL", activitySubType="SELLTOOPEN", rawType="OPTIONS_SELL",
                    quantity=-2, unitPrice=0.3, netCashAmount=60, transactionDate="2025-12-05", symbol="BBAI 02JAN26 5.50 PUT"),
                act(id="bto", category="trade", activityType="OPTIONS_BUY", activitySubType="BUYTOOPEN", rawType="OPTIONS_BUY",
                    quantity=1, unitPrice=1.0, netCashAmount=-100, transactionDate="2026-01-05", symbol="ZZZ 17JUL26 10.00 CALL"),
            ],
            "accounts": [], "balances": [], "navHistory": [], "navByAccount": {}, "syncedAt": "", "tradeGroups": [], "notes": {}, "securities": [],
        }
        base = model.build_base(snapshot, {"fx": {}, "benchmark": {}}, {}, today="2026-03-01")
        self.assertEqual(len(base["openLots"]), 1)
        self.assertEqual(base["openLots"][0]["symbol"], "ZZZ 17JUL26 10.00 CALL")
        self.assertEqual(len(base["trades"]), 1)
        t = base["trades"][0]
        self.assertEqual(t["exitDate"], "2026-01-02")
        self.assertEqual(t["exit"], 0)
        self.assertAlmostEqual(t["pnl"], 60)
        self.assertIn("assumed-expiry", t["flags"])
        self.assertEqual(t["status"], "closed")


class AssignmentTest(unittest.TestCase):
    def test_assigned_call_delivers_the_shares(self):
        snapshot = {
            "activities": [
                buy("b1", "ASTS", 300, 25.0, "2025-01-10", currency="USD", securityId="sec-s-asts"),
                act(id="sto", category="trade", activityType="OPTIONS_SELL", activitySubType="SELLTOOPEN", rawType="OPTIONS_SELL",
                    quantity=-3, unitPrice=1.5, netCashAmount=450, transactionDate="2025-02-10", symbol="ASTS 07MAR25 31.00 CALL", securityId="sec-o-asts"),
                act(id="asg", category="option_event", activityType="ASSIGN", activitySubType="BUYTOCLOSE", rawType="OPTIONS_ASSIGN",
                    quantity=3, unitPrice=0, netCashAmount=9300, transactionDate="2025-03-07", symbol="ASTS 07MAR25 31.00 CALL", securityId="sec-o-asts"),
            ],
            "accounts": [], "balances": [], "navHistory": [], "navByAccount": {}, "syncedAt": "", "tradeGroups": [], "notes": {},
            "securities": [{"id": "sec-o-asts", "symbol": "ASTS", "underlyingId": "sec-s-asts"}, {"id": "sec-s-asts", "symbol": "ASTS", "name": "AST SpaceMobile", "primaryExchange": "NASDAQ"}],
        }
        base = model.build_base(snapshot, {"fx": {}, "benchmark": {}}, {}, today="2026-09-06")
        self.assertEqual(base["openLots"], [])
        by_sym = {t["symbol"]: t for t in base["trades"]}
        shares = by_sym["ASTS"]
        self.assertEqual(shares["qty"], 300)
        self.assertEqual(shares["exit"], 31.0)
        self.assertEqual(shares["exitDate"], "2025-03-07")
        self.assertAlmostEqual(shares["pnl"], (31 - 25) * 300)
        self.assertIn("assignment", shares["flags"])
        self.assertEqual(shares["name"], "AST SpaceMobile")
        self.assertAlmostEqual(by_sym["ASTS 07MAR25 31.00 CALL"]["pnl"], 450)

    def test_assigned_put_buys_the_shares(self):
        snapshot = {
            "activities": [
                act(id="sto", category="trade", activityType="OPTIONS_SELL", activitySubType="SELLTOOPEN", rawType="OPTIONS_SELL",
                    quantity=-1, unitPrice=0.5, netCashAmount=50, transactionDate="2025-11-10", symbol="BBAI 05DEC25 5.00 PUT"),
                act(id="asg", category="option_event", activityType="ASSIGN", activitySubType="BUYTOCLOSE", rawType="OPTIONS_ASSIGN",
                    quantity=1, unitPrice=0, netCashAmount=-500, transactionDate="2025-12-05", symbol="BBAI 05DEC25 5.00 PUT"),
            ],
            "accounts": [], "balances": [], "navHistory": [], "navByAccount": {}, "syncedAt": "", "tradeGroups": [], "notes": {}, "securities": [],
        }
        base = model.build_base(snapshot, {"fx": {}, "benchmark": {}}, {}, today="2026-01-01")
        self.assertEqual([(l["symbol"], l["qty"], l["price"]) for l in base["openLots"]], [("BBAI", 100, 5.0)])
        self.assertIn("assignment", base["openLots"][0]["flags"])


class CryptoTest(unittest.TestCase):
    def test_a_transfer_out_leaves_at_cost_with_no_pnl(self):
        transfer = lambda id, qty, value, day, out: act(id=id, activityType="CRYPTO_TRANSFER", activitySubType="TRANSFER_OUT" if out else "TRANSFER_IN",
                                                          rawType="CRYPTO_TRANSFER", direction="DEBIT" if out else "CREDIT", quantity=qty, unitPrice=value / qty,
                                                          netCashAmount=-value if out else value, transactionDate=day, symbol="ETH", currency="CAD", accountType="Ponzi")
        acts = [
            act(id="cb", activityType="CRYPTO_BUY", activitySubType="MARKET_ORDER", rawType="CRYPTO_BUY",
                quantity=2, unitPrice=100, netCashAmount=200, transactionDate="2026-01-01", symbol="ETH", currency="CAD", accountType="Ponzi"),
            transfer("ti", 1, 120, "2026-01-05", False),
            transfer("to", 1, 200, "2026-01-10", True),   # would be +100 as a sale
            act(id="cs", activityType="CRYPTO_SELL", activitySubType="MARKET_ORDER", rawType="CRYPTO_SELL",
                quantity=2, unitPrice=150, netCashAmount=300, transactionDate="2026-02-01", symbol="ETH", currency="CAD", accountType="Ponzi"),
        ]
        fifo = model.match_fifo(acts)
        self.assertEqual(fifo["unmatched"], [])
        self.assertEqual(fifo["open"], [])
        self.assertEqual([(round(s["pnl"], 6), s["quantity"], s["entryPrice"]) for s in fifo["closed"]], [(50.0, 1.0, 100.0), (30.0, 1.0, 120.0)],
                         "the coin sent out came off the first lot at cost; the sale closed one at 100 and one at 120")
        trades = model.build_trades(fifo["closed"], fifo["open"], [], {a["id"]: model.normalize_activity(a) for a in acts}, model.Securities([]), {})
        self.assertEqual(len(trades), 1)
        self.assertEqual((round(trades[0]["pnl"], 6), trades[0]["qty"]), (80.0, 2.0))
        self.assertNotIn("to", {f["id"] for f in trades[0]["fills"]}, "the transfer out is not a fill of the trade")
        # nothing held: nothing to take off, nothing unmatched, no trade
        fifo = model.match_fifo([transfer("to2", 1, 200, "2026-01-10", True)])
        self.assertEqual((fifo["closed"], fifo["open"], fifo["unmatched"]), ([], [], []))

    def test_crypto_buy_sell_and_reward(self):
        acts = [
            act(id="cb", activityType="CRYPTO_BUY", activitySubType="MARKET_ORDER", rawType="CRYPTO_BUY",
                quantity=2, unitPrice=100, netCashAmount=200, transactionDate="2026-01-01", symbol="ETH", currency="CAD",
                accountType="Ponzi"),
            act(id="rw", activityType="CRYPTO_STAKING_REWARD", activitySubType="other", rawType="CRYPTO_STAKING_REWARD",
                quantity=1, unitPrice=0, netCashAmount=0, transactionDate="2026-01-05", symbol="ETH", currency="CAD",
                accountType="Ponzi"),
            act(id="cs", activityType="CRYPTO_SELL", activitySubType="MARKET_ORDER", rawType="CRYPTO_SELL",
                quantity=3, unitPrice=150, netCashAmount=450, transactionDate="2026-02-01", symbol="ETH", currency="CAD",
                accountType="Ponzi"),
        ]
        norm = model.normalize_activities(acts)
        self.assertEqual(norm[0]["kind"], "Crypto")
        self.assertLess(norm[0]["netCashAmount"], 0)
        self.assertIn("reward", norm[1]["flags"])
        fifo = model.match_fifo(norm)
        self.assertEqual(fifo["unmatched"], [])
        self.assertEqual(fifo["open"], [])
        self.assertEqual(len(fifo["closed"]), 2)
        self.assertAlmostEqual(sum(t["pnl"] for t in fifo["closed"]), (150 - 100) * 2 + 150 * 1)
        self.assertTrue(all(t["kind"] == "Crypto" for t in fifo["closed"]))

    def test_crypto_dust_sell_is_not_unmatched(self):
        acts = [
            act(id="cb", activityType="CRYPTO_BUY", rawType="CRYPTO_BUY", quantity=1.0, unitPrice=100,
                netCashAmount=100, transactionDate="2026-01-01", symbol="DOGE", currency="CAD"),
            act(id="cs", activityType="CRYPTO_SELL", rawType="CRYPTO_SELL", quantity=1.0000004, unitPrice=120,
                netCashAmount=120, transactionDate="2026-02-01", symbol="DOGE", currency="CAD"),
        ]
        fifo = model.match_fifo(model.normalize_activities(acts))
        self.assertEqual(fifo["unmatched"], [])
        self.assertEqual(len(fifo["closed"]), 1)

    def test_pending_distribution_notice_is_not_a_lot(self):
        acts = [
            buy("b", "RDDY", 100, 9, "2026-01-01"),
            act(id="stk", category="trade", activityType="STKDIS", activitySubType="BUY", rawType="DIVIDEND",
                quantity=100, unitPrice=0, netCashAmount=0, transactionDate="2026-02-01", symbol="RDDY", currency="CAD"),
        ]
        fifo = model.match_fifo(model.normalize_activities(acts))
        self.assertEqual(len(fifo["open"]), 1)
        self.assertEqual(fifo["open"][0]["qty"], 100)
        self.assertEqual(fifo["open"][0]["price"], 9)


class FxTest(unittest.TestCase):
    def test_usd_pnl_uses_rates_on_fill_dates(self):
        fx = {"2026-01-05": 1.40, "2026-02-05": 1.30}
        fifo = model.match_fifo([
            buy("b", "LUNR", 100, 10, "2026-01-05", currency="USD"),
            sell("s", "LUNR", 100, 12, "2026-02-05", currency="USD"),
        ])
        model.apply_fx(fifo["closed"], fx)
        t = fifo["closed"][0]
        self.assertAlmostEqual(t["pnl"], 200)
        self.assertAlmostEqual(t["pnlCad"], 1200 * 1.30 - 1000 * 1.40)

    def test_rate_walks_back_over_weekends_and_falls_back(self):
        fx = {"2026-01-02": 1.40}
        self.assertEqual(model.rate_on(fx, "2026-01-04"), 1.40)
        self.assertEqual(model.rate_on(fx, "2025-06-01"), model.FX_FALLBACK)
        self.assertEqual(model.to_cad(fx, 100, "CAD", "2026-01-04"), 100)


class ViewTest(unittest.TestCase):
    def setUp(self):
        self.snapshot = {
            "activities": [
                buy("b1", "AAA", 100, 10, "2025-03-01", accountType="Trading"),
                sell("s1", "AAA", 100, 12, "2025-03-10", accountType="Trading"),
                buy("b2", "BBB", 10, 100, "2026-01-05", accountType="Trading"),
                sell("s2", "BBB", 10, 90, "2026-01-20", accountType="Trading"),
                buy("b3", "CCC", 10, 5, "2026-02-01", accountType="Retirement"),
                sell("s3", "CCC", 10, 6, "2026-02-15", accountType="Retirement"),
                buy("b4", "DDD", 10, 5, "2026-03-01", accountType="Trading"),
                buy("b5", "LUNR", 10, 10, "2026-03-01", accountType="Trading", currency="USD"),
                sell("s5", "LUNR", 10, 11, "2026-03-05", accountType="Trading", currency="USD"),
            ],
            "accounts": [
                {"id": "acct-1", "nickname": "Trading", "unifiedAccountType": "TFSA", "currency": "CAD"},
                {"id": "acct-2", "nickname": "Retirement", "unifiedAccountType": "RRSP", "currency": "CAD"},
            ],
            "balances": [],
            "navHistory": [
                {"date": "2024-12-31", "equity": 1000, "netDeposits": 1000},
                {"date": "2025-06-30", "equity": 1500, "netDeposits": 1200},
                {"date": "2025-12-31", "equity": 1600, "netDeposits": 1200},
                {"date": "2026-03-31", "equity": 1400, "netDeposits": 1200},
            ],
            "navByAccount": {"Trading": [{"date": "2025-12-31", "equity": 800, "netDeposits": 500}, {"date": "2026-03-31", "equity": 700, "netDeposits": 500}]},
            "syncedAt": "2026-04-01T00:00:00Z",
            "tradeGroups": [],
            "notes": {},
            "securities": [],
        }
        self.market = {"fx": {"2026-03-01": 1.4, "2026-03-05": 1.3}, "benchmark": {"2024-12-31": 100, "2025-12-31": 120, "2026-03-31": 126}}
        self.journal = {"rt:b1": {"thesis": "yes", "tags": ["x"], "grade": "A"}, "rt:b2": {"thesis": "", "tags": [], "grade": "F"}}
        self.base = model.build_base(self.snapshot, self.market, self.journal, today="2026-04-01")

    def test_one_list_feeds_every_tile(self):
        v = model.build_view(self.base, None)
        k = v["kpi"]
        self.assertEqual(k["count"], 4)
        self.assertEqual(len(v["trades"]), 4)
        self.assertAlmostEqual(k["realized"], sum(t["pnlCad"] for t in v["trades"]))
        self.assertAlmostEqual(sum(m["value"] for m in v["monthly"]), k["realized"])
        self.assertAlmostEqual(sum(r["pnl"] for r in v["bySymbol"]), k["realized"])
        g = v["grades"]
        self.assertEqual(sum(b["n"] for b in g["buckets"]) + g["ungraded"], k["count"])
        self.assertEqual(sum(r["n"] for r in v["bySymbol"]), k["count"])
        self.assertEqual(k["wins"] + k["losses"] + k["breakeven"], k["count"])
        self.assertEqual(len(v["queue"]), 3)
        usd = next(t for t in v["trades"] if t["symbol"] == "LUNR")
        self.assertAlmostEqual(usd["pnl"], 10)
        self.assertAlmostEqual(usd["pnlCad"], 110 * 1.3 - 100 * 1.4)

    def test_positions_and_options(self):
        v = model.build_view(self.base, None)
        self.assertEqual(len(v["positions"]), 1)
        p = v["positions"][0]
        self.assertEqual(p["symbol"], "DDD")
        self.assertEqual(p["priceSource"], "fill")
        self.assertEqual(p["alloc"], 1.0)
        self.assertEqual(p["held"], 31)
        self.assertEqual(v["options"]["accounts"], ["Retirement", "Trading"])
        self.assertEqual(v["options"]["kinds"], ["Shares"])
        self.assertEqual(v["options"]["tags"], ["x"])
        listings = v["options"]["listings"]
        self.assertEqual(sorted(listings), v["options"]["symbols"], "one listing per symbol the ⌘K list can show")
        self.assertEqual(listings["DDD"]["exchange"], p["exchange"])
        self.assertEqual(listings["DDD"]["kind"], "Shares")
        self.assertIn("name", listings["DDD"])

    def test_account_filter_narrows_everything(self):
        v = model.build_view(self.base, {"lists": {"account": ["Retirement"]}})
        self.assertEqual(v["kpi"]["count"], 1)
        self.assertEqual(v["trades"][0]["symbol"], "CCC")
        self.assertEqual(v["positions"], [])
        self.assertEqual(v["equity"]["label"], "All accounts")
        v = model.build_view(self.base, {"lists": {"account": ["Trading"]}})
        self.assertEqual(v["equity"]["label"], "Trading")
        self.assertEqual(len(v["positions"]), 1)

    def test_date_filters(self):
        v = model.build_view(self.base, {"years": ["2025"]})
        self.assertEqual([t["symbol"] for t in v["trades"]], ["AAA"])
        v = model.build_view(self.base, {"preset": "ytd"})
        self.assertEqual({t["symbol"] for t in v["trades"]}, {"BBB", "CCC", "LUNR"})
        v = model.build_view(self.base, {"from": "2026-02-01", "to": "2026-02-28"})
        self.assertEqual([t["symbol"] for t in v["trades"]], ["CCC"])
        v = model.build_view(self.base, {"preset": "1m"})
        self.assertEqual([t["symbol"] for t in v["trades"]], ["LUNR"])

    def test_list_and_range_filters(self):
        v = model.build_view(self.base, {"lists": {"grade": ["A"]}})
        self.assertEqual([t["symbol"] for t in v["trades"]], ["AAA"])
        v = model.build_view(self.base, {"lists": {"grade": ["Ungraded"]}})
        self.assertEqual({t["symbol"] for t in v["trades"]}, {"CCC", "LUNR"})
        v = model.build_view(self.base, {"lists": {"result": ["Losers"]}})
        self.assertEqual([t["symbol"] for t in v["trades"]], ["BBB"])
        v = model.build_view(self.base, {"ranges": {"price": {"op": ">", "v": 50}}})
        self.assertEqual([t["symbol"] for t in v["trades"]], ["BBB"])
        v = model.build_view(self.base, {"search": "aa"})
        self.assertEqual([t["symbol"] for t in v["trades"]], ["AAA"])
        v = model.build_view(self.base, {"lists": {"tag": ["untagged"]}})
        self.assertEqual(v["kpi"]["count"], 3)

    def test_returns_and_drawdown(self):
        v = model.build_view(self.base, None)
        years = {y["year"]: y for y in v["years"]}
        self.assertAlmostEqual(years["2025"]["r"], (1500 - 1000 - 200) / 1000 * 1 + 0.0, places=6) if False else None
        # 2025: two steps, (1500-1000-200)/1000 then (1600-1500)/1500
        self.assertAlmostEqual(years["2025"]["r"], (1 + 0.3) * (1 + 100 / 1500) - 1)
        self.assertAlmostEqual(years["2025"]["spR"], 0.2)
        self.assertAlmostEqual(years["2025"]["flow"], 200)
        self.assertAlmostEqual(years["2026"]["r"], (1400 - 1600) / 1600)
        self.assertAlmostEqual(years["2026"]["spR"], 0.05)
        dd = v["equity"]["drawdown"]
        self.assertAlmostEqual(dd["pct"], (1400 - 1600) / 1600)
        self.assertEqual(dd["at"], "2026-03-31")
        self.assertIsNotNone(v["equity"]["annualized"]["rate"])

    def test_drawdown_ignores_withdrawals_and_deposits(self):
        series = model.equity_series([
            {"date": "2026-01-01", "equity": 100000, "netDeposits": 100000},
            {"date": "2026-01-02", "equity": 101000, "netDeposits": 100000},
            {"date": "2026-01-03", "equity": 21000, "netDeposits": 20000},   # withdrew 80,000; no loss
            {"date": "2026-01-04", "equity": 21210, "netDeposits": 20000},
            {"date": "2026-01-05", "equity": 41210, "netDeposits": 40000},   # deposited 20,000
            {"date": "2026-01-06", "equity": 37089, "netDeposits": 40000},   # a real 10% loss
        ])
        dd = model.drawdown(series)
        self.assertAlmostEqual(dd["pct"], -0.1, places=4)
        self.assertEqual(dd["at"], "2026-01-06")
        self.assertEqual(dd["peakAt"], "2026-01-05")
        self.assertAlmostEqual(dd["abs"], -4121, delta=1)

    def test_negligible_years_are_skipped(self):
        series = model.equity_series([
            {"date": "2020-12-22", "equity": 0, "netDeposits": 0},
            {"date": "2020-12-23", "equity": 100, "netDeposits": 100},
            {"date": "2020-12-31", "equity": 101, "netDeposits": 100},
            {"date": "2023-06-30", "equity": 45000, "netDeposits": 40000},
            {"date": "2023-12-31", "equity": 50000, "netDeposits": 40000},
            {"date": "2024-12-31", "equity": 60000, "netDeposits": 40000},
        ])
        years = model.yearly_returns(series, {}, "2025-01-01")
        self.assertEqual([y["year"] for y in years], ["2023", "2024"])
        self.assertEqual(years[0]["from"], "2023-06-30", "2023 is measured from the first funded point, not from the $101 of 2020")
        self.assertAlmostEqual(years[0]["r"], 50000 / 45000 - 1, places=6)

    def test_year_starts_where_the_account_was_really_funded(self):
        # a few dollars parked in September, the real money a week later: the year's
        # chain must start at the funded point, not at the $1,666 base that would
        # turn the deposit's timing into a -61% week
        series = model.equity_series([
            {"date": "2023-09-06", "equity": 0, "netDeposits": 15},
            {"date": "2023-09-13", "equity": 1666, "netDeposits": 1682},
            {"date": "2023-09-20", "equity": 111771, "netDeposits": 112806},
            {"date": "2023-10-04", "equity": 116832, "netDeposits": 119270},
            {"date": "2023-12-27", "equity": 135232, "netDeposits": 125411},
            {"date": "2024-06-30", "equity": 174611, "netDeposits": 125411},
            {"date": "2024-12-31", "equity": 170000, "netDeposits": 125411},
        ])
        yr = model.year_return(series, "2023", "2025-01-01")
        self.assertEqual(yr["from"], "2023-09-20")
        # from 111,771 with 12,605 more deposited to 135,232: about +9.5%
        self.assertAlmostEqual(yr["r"], (116832 - 111771 - 6464) / 111771 * 1 + 0, delta=0.2)
        self.assertGreater(yr["r"], 0.05)
        self.assertLess(yr["r"], 0.15)
        bench = {"2022-12-30": 3800.0, "2023-09-19": 4400.0, "2023-12-29": 4770.0, "2024-12-31": 5880.0}
        years = model.yearly_returns(series, bench, "2025-01-01")
        by = {y["year"]: y for y in years}
        self.assertAlmostEqual(by["2023"]["spR"], 4770 / 4400 - 1, places=6, msg="the index is measured over the same span as the account")
        self.assertAlmostEqual(by["2024"]["spR"], 5880 / 4770 - 1, places=6)

    def test_yearly_returns_compare_against_the_chosen_index(self):
        series = model.equity_series([
            {"date": "2023-12-31", "equity": 100000, "netDeposits": 100000},
            {"date": "2024-12-31", "equity": 120000, "netDeposits": 100000},
        ])
        market_data = {"fx": {}, "benchmark": {"2023-12-29": 100.0, "2024-12-31": 110.0}, "benchmarks": {"SP500": {"2023-12-29": 100.0, "2024-12-31": 110.0}, "TSX": {"2023-12-29": 200.0, "2024-12-31": 250.0}}}
        snapshot = {"activities": [], "accounts": [], "balances": [], "navHistory": [{"date": "2023-12-31", "equity": 100000, "netDeposits": 100000}, {"date": "2024-12-31", "equity": 120000, "netDeposits": 100000}], "navByAccount": {}, "syncedAt": "", "tradeGroups": [], "notes": {}, "securities": []}
        base = model.build_base(snapshot, market_data, {}, today="2025-01-01")
        self.assertEqual(model.clean_filters({"benchmark": "tsx"})["benchmark"], "TSX")
        self.assertEqual(model.clean_filters({"benchmark": "tsx60"})["benchmark"], "TSX60")
        self.assertEqual(model.clean_filters({"benchmark": "nope"})["benchmark"], "SP500")
        v = model.build_view(base, {})
        self.assertEqual(v["benchmark"], {"key": "SP500", "label": "S&P 500"})
        self.assertAlmostEqual(v["years"][-1]["spR"], 0.10, places=6)
        v = model.build_view(base, {"benchmark": "TSX"})
        self.assertEqual(v["benchmark"], {"key": "TSX", "label": "S&P/TSX"})
        self.assertAlmostEqual(v["years"][-1]["spR"], 0.25, places=6)
        market_data["benchmarks"]["TSX60"] = {"2023-12-29": 100.0, "2024-12-31": 115.0}
        base = model.build_base(snapshot, market_data, {}, today="2025-01-01")
        v = model.build_view(base, {"benchmark": "TSX60"})
        self.assertEqual(v["benchmark"], {"key": "TSX60", "label": "TSX 60"})
        self.assertAlmostEqual(v["years"][-1]["spR"], 0.15, places=6)
        self.assertAlmostEqual(v["years"][-1]["r"], 0.20, places=6, msg="the account's own return does not depend on the index")

    def test_ex_div_and_pay_day_next_declared_else_last_known(self):
        acts = [
            buy("b1", "RDDY", 100, 5, "2026-05-01", accountType="Cashflow"),
            act(id="d1", category="dividend", activityType="Dividend", rawType="DIVIDEND", quantity=100, unitPrice=0.2, netCashAmount=20, transactionDate="2026-08-06", symbol="RDDY", currency="CAD", accountType="Cashflow"),
            buy("b2", "HHIS", 100, 5, "2026-05-01", accountType="Cashflow"),
            act(id="d2", category="dividend", activityType="Dividend", rawType="DIVIDEND", quantity=100, unitPrice=0.2, netCashAmount=20, transactionDate="2026-08-06", symbol="HHIS", currency="CAD", accountType="Cashflow"),
            buy("b3", "HBIX", 100, 5, "2026-05-01", accountType="Cashflow"),
            act(id="d3", category="dividend", activityType="Dividend", rawType="DIVIDEND", quantity=100, unitPrice=0.2, netCashAmount=20, transactionDate="2026-08-06", symbol="HBIX", currency="CAD", accountType="Cashflow"),
            buy("b4", "EASY", 100, 20, "2026-05-01", accountType="Cashflow"),
            act(id="d4", category="dividend", activityType="Dividend", rawType="DIVIDEND", quantity=100, unitPrice=0.31, netCashAmount=31, transactionDate="2026-08-21", symbol="EASY", currency="CAD", accountType="Cashflow"),
        ]
        snapshot = {"activities": acts, "accounts": [], "balances": [], "navHistory": [], "navByAccount": {}, "syncedAt": "", "tradeGroups": [], "notes": {}, "securities": []}
        market_data = {"fx": {}, "benchmark": {},
                       "distributions": {"RDDY": [{"exDate": "2026-09-30", "payDate": "2026-10-06", "amount": 0.15, "currency": "CAD"}, {"exDate": "2026-08-31", "payDate": "2026-09-04", "amount": 0.15, "currency": "CAD"}]},
                       "quotes": {"HHIS": {"price": 11.0, "exDividendDate": "2026-09-29"}, "HBIX": {"price": 6.7, "exDividendDate": "2026-08-29"}}}
        market_data["distributions"]["HHIS"] = [{"exDate": "2026-08-31", "payDate": "2026-09-04", "amount": 0.27, "currency": "CAD"}]
        # EASY pays twice a month: gone ex on the 31st, paid on the 8th, ex again on the 15th.
        market_data["distributions"]["EASY"] = [{"exDate": "2026-08-31", "payDate": "2026-09-08", "amount": 0.255, "currency": "CAD"}, {"exDate": "2026-09-15", "payDate": "2026-09-22", "amount": 0.255, "currency": "CAD"}]
        base = model.build_base(snapshot, market_data, {}, today="2026-09-07")
        by = {h["symbol"]: (h["nextExDate"], h["nextPayDate"], h["exPast"], h["payPast"]) for h in model.build_view(base, {})["cashflow"]["holdings"]}
        self.assertEqual(by["RDDY"], ("2026-09-30", "2026-10-06", False, False), "the declared record's next distribution, with its pay date")
        self.assertEqual(by["EASY"], ("2026-08-31", "2026-09-08", True, False), "gone ex but not yet paid: that distribution, not the one after it")
        self.assertEqual(by["HHIS"], ("2026-08-31", "2026-09-04", True, True), "nothing left to pay: the last known one, both dates passed")
        self.assertEqual(by["HBIX"], ("2026-08-29", "2026-08-06", True, True), "no record: the quote's last ex-date and the last payment received")
        base = model.build_base(snapshot, market_data, {}, today="2026-09-08")
        by = {h["symbol"]: (h["nextExDate"], h["nextPayDate"], h["exPast"], h["payPast"]) for h in model.build_view(base, {})["cashflow"]["holdings"]}
        self.assertEqual(by["EASY"], ("2026-08-31", "2026-09-08", True, False), "pay day itself still counts as ahead")
        base = model.build_base(snapshot, market_data, {}, today="2026-09-09")
        by = {h["symbol"]: (h["nextExDate"], h["nextPayDate"], h["exPast"], h["payPast"]) for h in model.build_view(base, {})["cashflow"]["holdings"]}
        self.assertEqual(by["EASY"], ("2026-09-15", "2026-09-22", False, False), "once paid, the next one")

    def test_monthly_distributions_run_to_the_current_month(self):
        acts = [
            buy("b1", "RDDY", 100, 5, "2026-05-01", accountType="Cashflow"),
            act(id="d1", category="dividend", activityType="Dividend", rawType="DIVIDEND", quantity=100, unitPrice=0.2, netCashAmount=20, transactionDate="2026-06-06", symbol="RDDY", currency="CAD", accountType="Cashflow"),
            act(id="d2", category="dividend", activityType="Dividend", rawType="DIVIDEND", quantity=100, unitPrice=0.2, netCashAmount=20, transactionDate="2026-07-06", symbol="RDDY", currency="CAD", accountType="Cashflow"),
        ]
        snapshot = {"activities": acts, "accounts": [], "balances": [], "navHistory": [], "navByAccount": {}, "syncedAt": "", "tradeGroups": [], "notes": {}, "securities": []}
        base = model.build_base(snapshot, {"fx": {}, "benchmark": {}}, {}, today="2026-09-07")
        months = model.build_view(base, {})["cashflow"]["months"]
        self.assertEqual([(m["key"], m["count"]) for m in months], [("2026-06", 1), ("2026-07", 1), ("2026-08", 0), ("2026-09", 0)], "empty bars up to the current month")
        tiles = {t["label"]: t for t in model.build_view(base, {})["cashflow"]["tiles"] if "perMonth" in t}
        self.assertEqual(tiles["2026 YTD"]["perMonth"], 20, "the monthly average counts paying months only")
        months = model.build_view(base, {"to": "2026-08-15"})["cashflow"]["months"]
        self.assertEqual([m["key"] for m in months], ["2026-06", "2026-07", "2026-08"], "a date filter ends the chart at its bound")
        base25 = model.build_base(snapshot, {"fx": {}, "benchmark": {}}, {}, today="2027-03-01")
        months = model.build_view(base25, {"years": ["2026"]})["cashflow"]["months"]
        self.assertEqual(months[-1]["key"], "2026-12", "a year filter ends the chart at December")

    def test_cashflow_tiles_roll_over_with_the_calendar(self):
        acts = [
            buy("b1", "RDDY", 100, 5, "2025-06-01", accountType="Cashflow"),
            act(id="d1", category="dividend", activityType="Dividend", rawType="DIVIDEND", quantity=100, unitPrice=0.2, netCashAmount=20, transactionDate="2025-07-06", symbol="RDDY", currency="CAD", accountType="Cashflow"),
            act(id="d2", category="dividend", activityType="Dividend", rawType="DIVIDEND", quantity=100, unitPrice=0.2, netCashAmount=20, transactionDate="2026-07-06", symbol="RDDY", currency="CAD", accountType="Cashflow"),
        ]
        snapshot = {"activities": acts, "accounts": [], "balances": [], "navHistory": [], "navByAccount": {}, "syncedAt": "", "tradeGroups": [], "notes": {}, "securities": []}
        labels = lambda today: [t["label"] for t in model.build_view(model.build_base(snapshot, {"fx": {}, "benchmark": {}}, {}, today=today), {})["cashflow"]["tiles"]]
        self.assertEqual(labels("2026-09-07"), ["2024", "2025", "2026 YTD", "All time", "Last 12 months", "Yield on cost"], "no margin account: Last 12 months stands in")
        self.assertEqual(labels("2027-01-01"), ["2025", "2026", "2027 YTD", "All time", "Last 12 months", "Yield on cost"])
        totals = {t["label"]: round(t["total"]) for t in model.build_view(model.build_base(snapshot, {"fx": {}, "benchmark": {}}, {}, today="2027-01-01"), {})["cashflow"]["tiles"] if "total" in t}
        self.assertEqual((totals["2026"], totals["2027 YTD"], totals["All time"]), (20, 0, 40))

    def test_model_cache_rolls_over_at_midnight(self):
        with tempfile.TemporaryDirectory() as tmp:
            os.environ["BAGHOLDER_HOME"] = tmp
            store.set_home(tmp)
            store.ensure()
            try:
                with mock.patch.object(model, "today_local", return_value="2026-12-31"):
                    model.invalidate()
                    self.assertEqual(model.base_model()["today"], "2026-12-31")
                    self.assertIs(model.base_model(), model.base_model(), "same day: the cached base is reused")
                with mock.patch.object(model, "today_local", return_value="2027-01-01"):
                    self.assertEqual(model.base_model()["today"], "2027-01-01", "a new day rebuilds even though no data changed")
            finally:
                model.invalidate()
                os.environ.pop("BAGHOLDER_HOME", None)

    def test_filters_are_cleaned(self):
        f = model.clean_filters({"lists": {"account": ["A", 3, ""]}, "ranges": {"hold": {"op": "<", "v": "7"}}, "preset": "bogus", "years": [2025, "abcd"], "from": "2026-1-1", "to": "2026-02-01"})
        self.assertEqual(f["lists"]["account"], ["A", "3"])
        self.assertEqual(f["ranges"]["hold"], {"op": "<", "v": 7.0})
        self.assertEqual(f["preset"], "all")
        self.assertEqual(f["years"], ["2025"])
        self.assertEqual(f["from"], "")
        self.assertEqual(f["to"], "2026-02-01")


class CashflowTest(unittest.TestCase):
    def test_without_a_margin_account_cash_day_change_and_last_twelve_months_stand_in(self):
        snapshot = {
            "activities": [
                buy("b1", "AAA", 10, 10, "2025-01-05", accountType="Trading"),
                buy("b2", "BBB", 5, 20, "2025-01-06", accountType="Kids", accountId="acct-2", currency="USD"),
                act(id="d0", category="dividend", activityType="Dividend", activitySubType="dividend", rawType="DIVIDEND", quantity=10, unitPrice=1.0, netCashAmount=10, transactionDate="2024-12-01", symbol="AAA", currency="CAD", accountType="Trading"),
                act(id="d1", category="dividend", activityType="Dividend", activitySubType="dividend", rawType="DIVIDEND", quantity=10, unitPrice=1.0, netCashAmount=10, transactionDate="2025-06-01", symbol="AAA", currency="CAD", accountType="Trading"),
                act(id="d2", category="dividend", activityType="Dividend", activitySubType="dividend", rawType="DIVIDEND", quantity=10, unitPrice=1.5, netCashAmount=15, transactionDate="2025-12-01", symbol="AAA", currency="CAD", accountType="Trading"),
            ],
            "accounts": [
                {"id": "acct-1", "nickname": "Trading", "currency": "CAD", "netLiquidationValue": 1500.0, "unifiedAccountType": "SELF_DIRECTED_TFSA"},
                {"id": "acct-2", "nickname": "Kids", "currency": "CAD", "netLiquidationValue": 500.0, "unifiedAccountType": "SELF_DIRECTED_RESP"},
            ],
            "balances": [{"accountId": "acct-1", "securityId": "sec-c-cad", "quantity": 300.0}, {"accountId": "acct-2", "securityId": "sec-c-usd", "quantity": 10.0}],
            "navHistory": [], "navByAccount": {}, "syncedAt": "", "tradeGroups": [], "notes": {},
            "securities": [{"id": "sec-c-cad", "symbol": "CAD", "currency": "CAD"}, {"id": "sec-c-usd", "symbol": "USD", "currency": "USD"}],
        }
        market = {"fx": {"2026-02-01": 1.5}, "benchmark": {}, "quotes": {"AAA": {"price": 12.0, "priceChange": 0.5, "percentChange": 4.35}, "BBB": {"price": 30.0, "priceChange": -1.0, "percentChange": -3.2}}}
        base = model.build_base(snapshot, market, {}, today="2026-02-01")
        v = model.build_view(base, None)
        pf = v["portfolio"]
        self.assertFalse(pf["hasMargin"])
        self.assertAlmostEqual(pf["cash"], 300 + 10 * 1.5)
        self.assertAlmostEqual(pf["cashPct"], 315 / 2000)
        # AAA +$5 CAD, BBB −$5 USD = −$7.5 CAD; over the previous close of both, $120 + $225 − (−$2.5)
        self.assertAlmostEqual(pf["dayChange"], 5 - 7.5)
        self.assertAlmostEqual(pf["dayChangePct"], -2.5 / (120 + 225 + 2.5))
        tiles = {t["label"]: t for t in v["cashflow"]["tiles"]}
        self.assertNotIn("Margin used", tiles)
        self.assertAlmostEqual(tiles["Last 12 months"]["total"], 25, "the two payments since 2025-02-01")
        self.assertAlmostEqual(tiles["Last 12 months"]["perMonth"], 12.5)
        self.assertEqual(tiles["Last 12 months"]["count"], 2)
        # one account on, the other a margin account: the margin tiles come back for that scope
        snapshot["accounts"][1]["unifiedAccountType"] = "SELF_DIRECTED_NON_REGISTERED_MARGIN"
        base = model.build_base(snapshot, market, {}, today="2026-02-01")
        self.assertTrue(model.build_view(base, None)["portfolio"]["hasMargin"])
        self.assertFalse(model.build_view(base, {"lists": {"account": ["Trading"]}})["portfolio"]["hasMargin"])

    def test_margin_used_tile_averages_interest_charges_over_charged_months(self):
        charge = lambda i, day, amount, ccy: act(
            id="i%d" % i, activityType="INTEREST_CHARGE", activitySubType="MARGIN_INTEREST", rawType="INTEREST_CHARGE",
            category="other", netCashAmount=-amount, transactionDate=day, symbol="", currency=ccy, accountType="Trading",
        )
        snapshot = {
            "activities": [
                buy("b1", "AAA", 10, 10, "2026-01-05", accountType="Trading"),
                charge(1, "2026-07-01", 100, "CAD"),
                charge(2, "2026-08-01", 20, "USD"),
                charge(3, "2026-08-04", 10, "CAD"),
            ],
            "accounts": [{"id": "acct-1", "nickname": "Trading", "currency": "CAD", "netLiquidationValue": 1500.0, "unifiedAccountType": "SELF_DIRECTED_NON_REGISTERED_MARGIN"}],
            "balances": [{"accountId": "acct-1", "securityId": "sec-c-cad", "quantity": -300.0}],
            "navHistory": [], "navByAccount": {}, "syncedAt": "", "tradeGroups": [], "notes": {},
            "securities": [{"id": "sec-c-cad", "symbol": "CAD", "currency": "CAD"}, {"id": "sec-c-usd", "symbol": "USD", "currency": "USD"}],
        }
        base = model.build_base(snapshot, {"fx": {"2026-08-01": 1.5}, "benchmark": {}}, {}, today="2026-09-06")
        v = model.build_view(base, None)
        labels = [t["label"] for t in v["cashflow"]["tiles"]]
        self.assertEqual(labels[-3:], ["All time", "Margin used", "Yield on cost"])
        tile = v["cashflow"]["tiles"][-2]
        self.assertAlmostEqual(tile["marginUsed"], v["portfolio"]["marginUsed"])
        self.assertAlmostEqual(tile["marginUsed"], 300.0)
        # $100 + $20 × 1.5 + $10 over the two months that carried a charge
        self.assertEqual(tile["interestMonths"], 2)
        self.assertAlmostEqual(tile["interestPerMonth"], (100 + 30 + 10) / 2)
        # a scope without a margin account has no Margin used tile: Last 12 months stands in
        v = model.build_view(base, {"lists": {"account": ["Cashflow"]}})
        self.assertEqual([t["label"] for t in v["cashflow"]["tiles"]][-2:], ["Last 12 months", "Yield on cost"])

    def test_yield_on_cost_from_declared_rate(self):
        div = lambda i, day, qty, per: act(
            id="d%d" % i, category="dividend", activityType="Dividend", activitySubType="dividend", rawType="DIVIDEND",
            quantity=qty, unitPrice=per, netCashAmount=qty * per, transactionDate=day, symbol="RDDY", currency="CAD",
            accountType="Cashflow",
        )
        snapshot = {
            "activities": [
                buy("b1", "RDDY", 20000, 7.13, "2026-01-05", accountType="Cashflow"),
                div(1, "2026-07-06", 19000, 0.2),
                div(2, "2026-08-06", 19000, 0.2),
                act(id="int", category="interest", activityType="Interest", rawType="INTEREST", netCashAmount=4.5,
                    transactionDate="2026-08-01", symbol="", accountType="Cash"),
                act(id="wht", activityType="WITHHOLDING_TAX", rawType="WITHHOLDING_TAX", netCashAmount=-40,
                    transactionDate="2026-08-07", symbol="", accountType="Cashflow"),
            ],
            "accounts": [], "balances": [], "navHistory": [], "navByAccount": {}, "syncedAt": "", "tradeGroups": [], "notes": {}, "securities": [],
        }
        base = model.build_base(snapshot, {"fx": {}, "benchmark": {}}, {}, today="2026-09-06")
        self.assertEqual(len(base["cashflow"]), 4)
        v = model.build_view(base, None)
        cf = v["cashflow"]
        self.assertEqual(cf["count"], 2)
        self.assertEqual([r["kind"] for r in cf["rows"]], ["Dividend", "Dividend"])
        self.assertEqual({r["kind"] for r in cf["other"]}, {"Interest", "Withholding tax"})
        self.assertAlmostEqual(cf["total"], 7600)
        self.assertEqual([m["key"] for m in cf["months"]], ["2026-07", "2026-08", "2026-09"], "runs to the current month")
        h = cf["holdings"][0]
        self.assertEqual(h["symbol"], "RDDY")
        self.assertEqual(h["freq"], 12)
        self.assertAlmostEqual(h["yoc"], 2.4 / 7.13)
        self.assertAlmostEqual(h["yob"], 0.2 * 20000)
        self.assertAlmostEqual(h["ytd"], 7600)
        tiles = {t["label"]: t for t in cf["tiles"]}
        self.assertAlmostEqual(tiles["2026 YTD"]["total"], 7600)
        self.assertAlmostEqual(tiles["2026 YTD"]["perMonth"], 3800)
        self.assertAlmostEqual(tiles["Yield on cost"]["yield"], (2.4 * 20000) / (20000 * 7.13))
        self.assertAlmostEqual(tiles["Yield on cost"]["projected"], 2.4 * 20000 / 12)
        v = model.build_view(base, {"lists": {"grade": ["A"]}})
        self.assertIn("grade", v["cashflow"]["skippedFilters"])
        self.assertEqual(v["cashflow"]["count"], 2)


class PaymentFrequencyTest(unittest.TestCase):
    def test_frequency_is_verified_from_dates(self):
        self.assertEqual(model.payments_per_year(["2026-07-06", "2026-08-06"]), 12)
        self.assertEqual(model.payments_per_year(["2026-08-06", "2026-07-06", "2026-06-05", "2026-05-06"]), 12)
        self.assertEqual(model.payments_per_year(["2025-01-07", "2026-01-07"]), 1)
        self.assertEqual(model.payments_per_year(["2025-03-20", "2025-06-20", "2025-09-22", "2025-12-19"]), 4)
        self.assertEqual(model.payments_per_year(["2026-01-02", "2026-01-09", "2026-01-16"]), 52)
        # a monthly payer that switched to weekly is read from its recent payments
        self.assertEqual(model.payments_per_year(["2026-01-06", "2026-02-06", "2026-03-06", "2026-04-06", "2026-05-06", "2026-05-13", "2026-05-20", "2026-05-27"]), 52)
        self.assertIsNone(model.payments_per_year(["2026-08-06"]))
        self.assertIsNone(model.payments_per_year(["2026-08-06", "2026-08-06"]))

    def test_frequency_uses_payment_rows_without_per_unit_values(self):
        div = lambda i, day, qty, per, amount: act(
            id="v%d" % i, category="dividend", activityType="Dividend", activitySubType="dividend", rawType="DIVIDEND",
            quantity=qty, unitPrice=per, netCashAmount=amount, transactionDate=day, symbol="VEQT", currency="CAD", accountType="Kids",
        )
        snapshot = {
            "activities": [buy("b1", "VEQT", 300, 49.76, "2024-06-01", accountType="Kids"), div(1, "2025-01-07", 0, 0, 91.56), div(2, "2026-01-07", 300, 0.76, 228)],
            "accounts": [], "balances": [], "navHistory": [], "navByAccount": {}, "syncedAt": "", "tradeGroups": [], "notes": {}, "securities": [],
        }
        base = model.build_base(snapshot, {"fx": {}, "benchmark": {}}, {}, today="2026-09-06")
        h = model.build_view(base, None)["cashflow"]["holdings"][0]
        self.assertEqual(h["freq"], 1)
        self.assertTrue(h["freqVerified"])

    def test_single_payment_shows_no_yield_and_annual_payer_is_not_x12(self):
        div = lambda i, sym, day, qty, per, acct="Kids": act(
            id="d%s%d" % (sym, i), category="dividend", activityType="Dividend", activitySubType="dividend", rawType="DIVIDEND",
            quantity=qty, unitPrice=per, netCashAmount=qty * per, transactionDate=day, symbol=sym, currency="CAD", accountType=acct,
        )
        snapshot = {
            "activities": [
                buy("b1", "VEQT", 300, 49.76, "2024-06-01", accountType="Kids"),
                div(1, "VEQT", "2025-01-07", 300, 0.76),
                div(2, "VEQT", "2026-01-07", 300, 0.76),
                buy("b2", "NEWM", 1000, 10.0, "2026-07-01", accountType="Kids"),
                div(1, "NEWM", "2026-08-06", 1000, 0.1),
            ],
            "accounts": [], "balances": [], "navHistory": [], "navByAccount": {}, "syncedAt": "", "tradeGroups": [], "notes": {}, "securities": [],
        }
        base = model.build_base(snapshot, {"fx": {}, "benchmark": {}}, {}, today="2026-09-06")
        h = {x["symbol"]: x for x in model.build_view(base, None)["cashflow"]["holdings"]}
        self.assertEqual(h["VEQT"]["freq"], 1)
        self.assertTrue(h["VEQT"]["freqVerified"])
        self.assertAlmostEqual(h["VEQT"]["yoc"], 0.76 / 49.76)
        self.assertEqual(h["NEWM"]["freq"], 12)
        self.assertFalse(h["NEWM"]["freqVerified"])
        self.assertAlmostEqual(h["NEWM"]["yoc"], 0.1 * 12 / 10.0)
        self.assertEqual(h["NEWM"]["per"], 0.1)


class QuoteTest(unittest.TestCase):
    def test_tmx_quote_symbol_mapping(self):
        self.assertEqual(market.tmx_quote_symbol("CCHI", "TSX", "CAD"), "CCHI")
        self.assertEqual(market.tmx_quote_symbol("CH", "TSX-V", "CAD"), "CH")
        self.assertEqual(market.tmx_quote_symbol("LUNR", "NASDAQ", "USD"), "LUNR:US")
        self.assertEqual(market.tmx_quote_symbol("ASTS", "", "USD"), "ASTS:US")
        self.assertEqual(market.tmx_quote_symbol("HBIX", "Cboe Canada", "CAD"), "HBIX:AQL")
        self.assertEqual(market.tmx_quote_symbol("QIMC", "CSE", "CAD"), "QIMC:CNX", "TMX names CSE listings with :CNX")
        self.assertIsNone(market.tmx_quote_symbol("VOD", "LSE", "GBP"), "a currency TMX does not carry")
        self.assertEqual(market.tmx_quote_symbol("ONE", "ALPHA EXCHANGE", "CAD"), "ONE", "an ATS venue: the currency's usual form, settled by tmx_lookup")
        self.assertIsNone(market.tmx_quote_symbol("QNC 20NOV26 3.00 CALL", "", "USD"))

    def test_positions_use_the_quote_when_present(self):
        snapshot = {
            "activities": [buy("b1", "VEQT", 100, 49.76, "2026-01-05", accountType="Kids"), buy("b2", "HBIX", 100, 7.0, "2026-01-05", accountType="Kids")],
            "accounts": [], "balances": [], "navHistory": [], "navByAccount": {}, "syncedAt": "", "tradeGroups": [], "notes": {}, "securities": [],
        }
        quotes = {"VEQT": {"price": 62.4, "priceChange": 0.08, "percentChange": 0.128, "fetchedAt": "2026-09-06T14:00:00Z"}}
        base = model.build_base(snapshot, {"fx": {}, "benchmark": {}, "quotes": quotes}, {}, today="2026-09-06")
        p = {x["symbol"]: x for x in base["positions"]}
        self.assertEqual(p["VEQT"]["last"], 62.4)
        self.assertEqual(p["VEQT"]["priceSource"], "quote")
        self.assertAlmostEqual(p["VEQT"]["unreal"], (62.4 - 49.76) * 100)
        self.assertEqual(p["VEQT"]["priceChange"], 0.08)
        self.assertEqual(p["HBIX"]["priceSource"], "fill")
        self.assertEqual(p["HBIX"]["last"], 7.0)
        self.assertEqual(model.held_symbols(base), [{"symbol": "VEQT", "exchange": "", "currency": "CAD", "kind": "Shares"}, {"symbol": "HBIX", "exchange": "", "currency": "CAD", "kind": "Shares"}])

    def test_quote_sources_cover_every_held_kind(self):
        src = market.quote_source
        self.assertEqual(src({"symbol": "VEQT", "exchange": "TSX", "currency": "CAD", "kind": "Shares"}), ("tmx", "VEQT"))
        self.assertEqual(src({"symbol": "LUNR", "exchange": "NASDAQ", "currency": "USD", "kind": "Shares"}), ("tmx", "LUNR:US"))
        self.assertEqual(src({"symbol": "HBIX", "exchange": "Cboe Canada", "currency": "CAD", "kind": "Shares"}), ("cboe_ca", "HBIX"))
        self.assertEqual(src({"symbol": "BTC", "exchange": "Crypto", "currency": "CAD", "kind": "Crypto"}), ("coinbase", "BTC-CAD"))
        self.assertEqual(src({"symbol": "BTC", "exchange": "Crypto", "currency": "USD", "kind": "Crypto"}), ("coinbase", "BTC-USD"))
        self.assertEqual(src({"symbol": "QNC 20NOV26 3.00 CALL", "exchange": "NYSE", "currency": "USD", "kind": "Options"}), ("cboe_options", "QNC261120C00003000"))
        self.assertIsNone(src({"symbol": "SHOP 17OCT25 100.00 PUT", "exchange": "TSX", "currency": "CAD", "kind": "Options"}))
        self.assertEqual(market.occ_code("LUNR 29AUG25 11.50 CALL"), "LUNR250829C00011500")
        self.assertEqual(market.occ_code("SPY 251219P00450000"), "SPY251219P00450000")
        self.assertEqual(market.occ_code("VEQT"), "")
        self.assertEqual(market.occ_root("QNC261120C00003000"), "QNC")

    def test_public_quote_parsers(self):
        closed = json.dumps({"data": {"last": "0.0", "prev_close": "6.7600", "change": "0.0", "change_pct": "0.0", "company_name": "HARVEST BITCOIN ENHANCED INCOME ETF"}})
        self.assertEqual(market.parse_cboe_ca_quote(closed)["price"], 6.76)
        open_ = json.dumps({"data": {"last": "6.81", "prev_close": "6.7600", "change": "0.05", "change_pct": "0.74"}})
        q = market.parse_cboe_ca_quote(open_)
        self.assertEqual((q["price"], q["prevClose"], q["priceChange"]), (6.81, 6.76, 0.05))
        self.assertIsNone(market.parse_cboe_ca_quote(json.dumps({"data": {"last": "0", "prev_close": "0"}})))
        self.assertEqual(market.parse_coinbase(json.dumps({"data": {"amount": "109300.3", "base": "BTC", "currency": "CAD"}}), "BTC-CAD"), {"price": 109300.3, "currency": "CAD"})
        self.assertIsNone(market.parse_coinbase(json.dumps({"errors": [{"id": "not_found"}]}), "XYZ-CAD"))
        chain = json.dumps({"data": {"options": [{"option": "QNC261120C00003000", "bid": 0.0, "ask": 0.25, "last_trade_price": 0.15, "prev_day_close": 0.15}, {"option": "QNC261120C00005000", "bid": 0.1, "ask": 0.2, "last_trade_price": 0.05, "prev_day_close": 0.12}]}})
        rows = market.parse_cboe_options(chain)
        self.assertEqual(market.option_mark(rows["QNC261120C00003000"])["price"], 0.15)
        self.assertAlmostEqual(market.option_mark(rows["QNC261120C00005000"])["price"], 0.15)
        self.assertIsNone(market.option_mark(rows.get("QNC261120C00009000")))
        self.assertIsNone(market.option_mark({"bid": 0, "ask": 0, "last_trade_price": 0, "prev_day_close": 0}))

    def test_refresh_quotes_prices_crypto_and_options(self):
        with tempfile.TemporaryDirectory() as tmp:
            os.environ["BAGHOLDER_HOME"] = tmp
            store.set_home(tmp)
            store.ensure()
            try:
                syms = [
                    {"symbol": "BTC", "exchange": "Crypto", "currency": "CAD", "kind": "Crypto"},
                    {"symbol": "QNC 20NOV26 3.00 CALL", "exchange": "NYSE", "currency": "USD", "kind": "Options"},
                    {"symbol": "QNC 19FEB27 3.00 CALL", "exchange": "NYSE", "currency": "USD", "kind": "Options"},
                    {"symbol": "SHOP 17OCT25 100.00 PUT", "exchange": "TSX", "currency": "CAD", "kind": "Options"},
                ]
                chain = {"QNC261120C00003000": {"bid": 0.1, "ask": 0.2, "prev_day_close": 0.15}, "QNC270219C00003000": {"bid": 0, "ask": 0.5, "last_trade_price": 0.3, "prev_day_close": 0.3}}
                with mock.patch.object(market, "fetch_coinbase_spot", return_value={"price": 109300.3, "currency": "CAD"}) as cb, mock.patch.object(market, "fetch_cboe_option_chain", return_value=chain) as oc:
                    self.assertEqual(market.refresh_quotes(syms), 3)
                self.assertEqual([x.args[0] for x in cb.call_args_list], ["BTC-CAD"])
                self.assertEqual(oc.call_count, 1, "one chain fetch serves every contract on the underlying")
                q = store.quotes()
                self.assertEqual(q["BTC"]["price"], 109300.3)
                self.assertAlmostEqual(q["QNC 20NOV26 3.00 CALL"]["price"], 0.15)
                self.assertEqual(q["QNC 19FEB27 3.00 CALL"]["price"], 0.3)
                self.assertNotIn("SHOP 17OCT25 100.00 PUT", q)
            finally:
                os.environ.pop("BAGHOLDER_HOME", None)

    def test_positions_price_crypto_and_options_from_quotes(self):
        acts = [
            act(id="c1", category="trade", activityType="BUY", rawType="CRYPTO_BUY", quantity=0.5, unitPrice=100000, netCashAmount=-50000, transactionDate="2026-01-05", symbol="BTC", currency="CAD", accountType="Crypto", securityId="sec-z-btc"),
            act(id="o1", category="trade", activityType="BUY", rawType="OPTIONS_BUY", quantity=2, unitPrice=0.10, netCashAmount=-20, transactionDate="2026-02-05", symbol="QNC 20NOV26 3.00 CALL", currency="USD", accountType="TFSA", securityId="sec-o-1"),
        ]
        snapshot = {"activities": acts, "accounts": [], "balances": [], "navHistory": [], "navByAccount": {}, "syncedAt": "", "tradeGroups": [], "notes": {}, "securities": []}
        quotes = {"BTC": {"price": 120000.0}, "QNC 20NOV26 3.00 CALL": {"price": 0.15}}
        base = model.build_base(snapshot, {"fx": {}, "benchmark": {}, "quotes": quotes}, {}, today="2026-09-06")
        by = {p["symbol"]: p for p in base["positions"]}
        self.assertEqual((by["BTC"]["kind"], by["BTC"]["priceSource"], by["BTC"]["last"], by["BTC"]["mv"]), ("Crypto", "quote", 120000.0, 60000.0))
        self.assertEqual((by["QNC 20NOV26 3.00 CALL"]["kind"], by["QNC 20NOV26 3.00 CALL"]["priceSource"], by["QNC 20NOV26 3.00 CALL"]["last"], by["QNC 20NOV26 3.00 CALL"]["mv"]), ("Options", "quote", 0.15, 30.0))
        held = model.held_symbols(base)
        self.assertEqual(sorted((h["symbol"], h["kind"]) for h in held), [("BTC", "Crypto"), ("QNC 20NOV26 3.00 CALL", "Options")])

    def test_a_coins_price_never_prices_a_share_with_the_same_symbol(self):
        # a share or warrant called BTC beside the coin BTC: one quote row per symbol, and only the coin may take Coinbase's price
        acts = [
            act(id="c1", category="trade", activityType="BUY", rawType="CRYPTO_BUY", quantity=0.5, unitPrice=100000, netCashAmount=-50000, transactionDate="2026-01-05", symbol="BTC", currency="CAD", accountType="Crypto", securityId="sec-z-btc-1"),
            act(id="s1", category="trade", activityType="BUY", rawType="DIY_BUY", quantity=4653, unitPrice=1.75, netCashAmount=-8142.75, transactionDate="2026-02-05", symbol="BTC", currency="CAD", accountType="TFSA", securityId="sec-s-btc-warrant"),
        ]
        snapshot = {"activities": acts, "accounts": [], "balances": [], "navHistory": [], "navByAccount": {}, "syncedAt": "", "tradeGroups": [], "notes": {}, "securities": []}
        quotes = {"BTC": {"price": 109998.0, "source": "coinbase"}}
        base = model.build_base(snapshot, {"fx": {}, "benchmark": {}, "quotes": quotes}, {}, today="2026-09-06")
        by = {(p["symbol"], p["kind"]): p for p in base["positions"]}
        self.assertEqual((by[("BTC", "Crypto")]["priceSource"], by[("BTC", "Crypto")]["last"]), ("quote", 109998.0))
        self.assertEqual((by[("BTC", "Shares")]["priceSource"], by[("BTC", "Shares")]["last"], round(by[("BTC", "Shares")]["mv"], 2)), ("fill", 1.75, 8142.75), "the share keeps its fill price rather than the coin's")
        self.assertTrue(model.quote_fits({"price": 1.0}, "Shares"), "a quote with no source stated is the kind's own")
        self.assertFalse(model.quote_fits({"price": 1.0, "source": "tmx"}, "Crypto"))

    def test_refresh_quotes_respects_the_interval(self):
        with tempfile.TemporaryDirectory() as tmp:
            os.environ["BAGHOLDER_HOME"] = tmp
            store.set_home(tmp)
            store.ensure()
            try:
                from datetime import datetime, timezone
                syms = [{"symbol": "VEQT", "exchange": "TSX", "currency": "CAD"}, {"symbol": "LUNR", "exchange": "NASDAQ", "currency": "USD"}, {"symbol": "HBIX", "exchange": "Cboe Canada", "currency": "CAD"}]
                cboe = {"price": 6.76, "prevClose": 6.76, "fetchedAt": "2026-09-06T14:00:00Z"}
                with mock.patch.object(market, "fetch_tmx_quote", return_value={"price": 10.0, "priceChange": 0.1, "percentChange": 1.0, "prevClose": 9.9, "fetchedAt": "2026-09-06T14:00:00Z"}) as f, mock.patch.object(market, "fetch_cboe_ca_quote", return_value=cboe) as c:
                    self.assertEqual(market.refresh_quotes(syms, now=datetime(2026, 9, 6, 14, 0, tzinfo=timezone.utc)), 3)
                    self.assertEqual([x.args[0] for x in f.call_args_list], ["VEQT", "LUNR:US"])
                    self.assertEqual([x.args[0] for x in c.call_args_list], ["HBIX"])
                    self.assertEqual(market.refresh_quotes(syms, now=datetime(2026, 9, 6, 14, 0, 30, tzinfo=timezone.utc)), 0)
                    self.assertEqual(market.refresh_quotes(syms, now=datetime(2026, 9, 6, 14, 2, tzinfo=timezone.utc)), 3)
                self.assertEqual(store.quotes()["HBIX"]["price"], 6.76)
                q = store.quotes()["LUNR"]
                self.assertEqual(q["price"], 10.0)
                self.assertEqual(q["prevClose"], 9.9)
                store.upsert_quote("LUNR", {"price": 11.0, "fetchedAt": "2026-09-06T15:00:00Z", "dividendAmount": None})
                self.assertEqual(store.quotes()["LUNR"]["price"], 11.0)
            finally:
                store.set_home(None)
                os.environ.pop("BAGHOLDER_HOME", None)


class DeclaredDistributionsTest(unittest.TestCase):
    def test_tmx_parsers(self):
        q = {"data": {"getQuoteBySymbol": {"symbol": "CCHI", "name": "Ninepoint Cameco HighShares ETF", "price": 10.95, "dividendFrequency": None, "dividendYield": 27.5, "dividendAmount": 0.135, "exDividendDate": "2026-09-15 00:00:00.0"}}}
        rec = market.parse_tmx_quote(q)
        self.assertEqual(rec["price"], 10.95)
        self.assertEqual(rec["exDividendDate"], "2026-09-15")
        d = {"data": {"dividends": {"dividends": [{"exDate": "2026-09-15", "payableDate": "2026-09-21", "amount": 0.135, "currency": "CAD"}, {"exDate": "bad", "amount": 1}, {"exDate": "2026-08-31", "payableDate": "2026-09-04", "amount": "0.135"}]}}}
        rows = market.parse_tmx_dividends(d)
        self.assertEqual([r["exDate"] for r in rows], ["2026-09-15", "2026-08-31"])
        self.assertEqual(market.tmx_symbol("cchi.to"), "CCHI")
        self.assertTrue(market.is_canadian_listing("TSX", "CAD"))
        self.assertFalse(market.is_canadian_listing("NASDAQ", "USD"))
        self.assertTrue(market.is_canadian_listing("", "CAD"))

    def test_declared_record_beats_own_history_and_tracks_schedule_change(self):
        div = lambda i, day, qty, per: act(
            id="c%d" % i, category="dividend", activityType="Dividend", activitySubType="dividend", rawType="DIVIDEND",
            quantity=qty, unitPrice=per, netCashAmount=qty * per, transactionDate=day, symbol="CCHI", currency="CAD", accountType="Cashflow",
        )
        snapshot = {
            "activities": [buy("b1", "CCHI", 4000, 11.64, "2026-08-25", accountType="Cashflow"), div(1, "2026-09-04", 4000, 0.135)],
            "accounts": [], "balances": [], "navHistory": [], "navByAccount": {}, "syncedAt": "", "tradeGroups": [], "notes": {}, "securities": [],
        }
        public = {"CCHI": [
            {"exDate": "2026-09-15", "payDate": "2026-09-21", "amount": 0.135, "currency": "CAD"},
            {"exDate": "2026-08-31", "payDate": "2026-09-04", "amount": 0.135, "currency": "CAD"},
            {"exDate": "2026-08-14", "payDate": "2026-08-20", "amount": 0.135, "currency": "CAD"},
            {"exDate": "2026-07-31", "payDate": "2026-08-10", "amount": 0.27, "currency": "CAD"},
            {"exDate": "2026-06-30", "payDate": "2026-07-08", "amount": 0.27, "currency": "CAD"},
            {"exDate": "2026-05-29", "payDate": "2026-06-05", "amount": 0.27, "currency": "CAD"},
        ]}
        quotes = {"CCHI": {"price": 10.95, "dividendAmount": 0.135, "dividendFrequency": "", "exDividendDate": "2026-09-15", "fetchedAt": "2026-09-06T00:00:00Z"}}
        base = model.build_base(snapshot, {"fx": {}, "benchmark": {}, "distributions": public, "quotes": quotes}, {}, today="2026-09-06")
        h = model.build_view(base, None)["cashflow"]["holdings"][0]
        self.assertEqual(h["per"], 0.135)
        self.assertEqual(h["freq"], 24)
        self.assertTrue(h["freqVerified"])
        self.assertEqual(h["rateSource"], "declared")
        self.assertAlmostEqual(h["yoc"], 0.135 * 24 / 11.64)
        self.assertEqual(h["last"], 10.95)
        self.assertEqual(h["priceSource"], "close")
        self.assertAlmostEqual(h["currentYield"], 0.135 * 24 / 10.95)
        # without the public record it falls back to the single own payment
        base = model.build_base(snapshot, {"fx": {}, "benchmark": {}}, {}, today="2026-09-06")
        h = model.build_view(base, None)["cashflow"]["holdings"][0]
        self.assertEqual(h["rateSource"], "payments")
        self.assertFalse(h["freqVerified"])
        self.assertEqual(h["priceSource"], "fill")

    def test_payer_symbols_are_held_dividend_payers(self):
        snapshot = {
            "activities": [
                buy("b1", "RDDY", 100, 7, "2026-01-05", accountType="Cashflow"),
                act(id="d1", category="dividend", activityType="Dividend", rawType="DIVIDEND", quantity=100, unitPrice=0.2, netCashAmount=20, transactionDate="2026-02-06", symbol="RDDY", currency="CAD", accountType="Cashflow"),
                buy("b2", "TD", 10, 80, "2025-01-05", accountType="Cashflow"),
                act(id="d2", category="dividend", activityType="Dividend", rawType="DIVIDEND", quantity=10, unitPrice=1, netCashAmount=10, transactionDate="2025-02-06", symbol="TD", currency="CAD", accountType="Cashflow"),
                sell("s2", "TD", 10, 90, "2025-03-01", accountType="Cashflow"),
                buy("b3", "AAA", 10, 5, "2026-01-05", accountType="Cashflow"),
            ],
            "accounts": [], "balances": [], "navHistory": [], "navByAccount": {}, "syncedAt": "", "tradeGroups": [], "notes": {}, "securities": [],
        }
        base = model.build_base(snapshot, {"fx": {}, "benchmark": {}}, {}, today="2026-09-06")
        self.assertEqual(model.payer_symbols(base), [{"symbol": "RDDY", "exchange": "", "currency": "CAD"}])

    def test_store_roundtrip_and_stale_detection(self):
        with tempfile.TemporaryDirectory() as tmp:
            os.environ["BAGHOLDER_HOME"] = tmp
            store.set_home(tmp)
            store.ensure()
            try:
                self.assertEqual(store.upsert_distributions("cchi", [{"exDate": "2026-08-31", "payDate": "2026-09-04", "amount": 0.135, "currency": "CAD"}, {"exDate": "x", "amount": 1}]), 1)
                self.assertEqual(store.distributions()["CCHI"][0]["amount"], 0.135)
                store.upsert_quote("CCHI", {"price": 10.95, "dividendAmount": 0.135, "fetchedAt": "2026-09-06T00:00:00Z"})
                self.assertEqual(store.quotes()["CCHI"]["price"], 10.95)
                syms = [{"symbol": "CCHI", "exchange": "TSX", "currency": "CAD"}, {"symbol": "LUNR", "exchange": "NASDAQ", "currency": "USD"}, {"symbol": "NEW", "exchange": "", "currency": "CAD"}]
                from datetime import datetime, timezone
                fresh = datetime(2026, 9, 6, 5, 0, tzinfo=timezone.utc)
                # A fresh quote says nothing about the declared record: until the
                # record itself has been fetched, the symbol is stale.
                self.assertEqual(market.stale_symbols(syms, now=fresh), ["CCHI", "NEW"])
                store.mark_distributions_fetched("CCHI", "2026-09-06T00:00:00Z")
                self.assertEqual(market.stale_symbols(syms, now=fresh), ["NEW"])
                old = datetime(2026, 9, 8, 5, 0, tzinfo=timezone.utc)
                self.assertEqual(market.stale_symbols(syms, now=old), ["CCHI", "NEW"])
                # The quote loop refreshing the quote does not make the record fresh.
                store.upsert_quote("CCHI", {"price": 11.0, "fetchedAt": "2026-09-08T04:55:00Z"})
                self.assertEqual(market.stale_symbols(syms, now=old), ["CCHI", "NEW"])
                with mock.patch.object(market, "fetch_tmx", return_value=({"price": 1.0, "dividendAmount": 0.1, "dividendFrequency": "Monthly", "exDividendDate": "2026-09-01"}, [{"exDate": "2026-09-01", "payDate": "2026-09-05", "amount": 0.1, "currency": "CAD"}])) as f:
                    self.assertEqual(market.refresh_distributions(syms), 2)
                self.assertEqual(sorted(store.quotes()), ["CCHI", "NEW"])
                self.assertEqual(sorted(store.distributions_fetched_at()), ["CCHI", "NEW"])
                self.assertEqual(f.call_count, 2)
                self.assertEqual(market.stale_symbols(syms), [])
                # A Cboe Canada listing has a record on TMX only under its :AQL
                # form; it is stored under the bare symbol, and TMX's delayed
                # quote does not replace the price Cboe's own feed keeps fresh.
                self.assertEqual(market.tmx_record_symbol("HBIX", "CBOE CANADA"), "HBIX:AQL")
                self.assertEqual(market.tmx_record_symbol("HBIX", "NEO"), "HBIX:AQL")
                self.assertEqual(market.tmx_record_symbol("CCHI", "TSX"), "CCHI")
                cboe = [{"symbol": "HBIX", "exchange": "CBOE CANADA", "currency": "CAD"}]
                store.upsert_quote("HBIX", {"price": 6.76}, source="cboe_ca")
                asked = []
                def post(url, body, *a, **k):
                    asked.append((body["operationName"], body["variables"]["symbol"]))
                    if body["operationName"] == "getQuoteBySymbol":
                        return {"data": {"getQuoteBySymbol": {"symbol": "HBIX:AQL", "price": 6.70, "exDividendDate": "2026-08-31 00:00:00.0", "dividendFrequency": "Monthly", "dividendAmount": 0.12}}}
                    return {"data": {"dividends": {"dividends": [{"exDate": "2026-08-31", "payableDate": "2026-09-04", "amount": 0.12, "currency": "CAD"}]}}}
                with mock.patch.object(market, "_post_json", side_effect=post):
                    self.assertEqual(market.refresh_distributions(cboe), 1)
                self.assertEqual(sorted(set(s for _, s in asked)), ["HBIX:AQL"])
                self.assertEqual([d["exDate"] for d in store.distributions().get("HBIX", [])], ["2026-08-31"])
                self.assertEqual(store.quotes()["HBIX"]["price"], 6.76)
                self.assertIn("HBIX", store.distributions_fetched_at())
            finally:
                store.set_home(None)
                os.environ.pop("BAGHOLDER_HOME", None)


class LegacyNotesTest(unittest.TestCase):
    def test_group_id_matches_ledger_html(self):
        # ledger.html: FNV-1a over "\n".join(sorted keys), "g_" + hex + "_" + n
        self.assertEqual(model.group_id_for_keys(["b|s|100.00000000"]), model.group_id_for_keys(["b|s|100.00000000"]))
        self.assertTrue(model.group_id_for_keys(["a", "b"]).endswith("_2"))
        self.assertEqual(model.group_id_for_keys(["a", "b"]), model.group_id_for_keys(["b", "a"]))

    def test_legacy_note_lands_on_round_trip(self):
        acts = model.normalize_activities([buy("b1", "AAA", 100, 10, "2026-01-01"), sell("s1", "AAA", 100, 12, "2026-01-10")])
        fifo = model.match_fifo(acts)
        key = model.slice_member_key(fifo["closed"][0])
        legacy_id = model.group_id_for_keys([key])
        journal = model.migrate_legacy_notes(fifo["closed"], [], {legacy_id: {"thesis": "why", "tag": "a, b", "grade": "C"}})
        self.assertEqual(journal, {"rt:b1": {"thesis": "why", "tags": ["a", "b"], "grade": "C"}})


class StoreTablesTest(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        os.environ["BAGHOLDER_HOME"] = self.tmp.name
        store.set_home(self.tmp.name)
        bagholder.set_home(self.tmp.name)
        store.ensure()
        model.invalidate()

    def tearDown(self):
        self.tmp.cleanup()
        os.environ.pop("BAGHOLDER_HOME", None)

    def test_fx_and_benchmark_roundtrip(self):
        self.assertEqual(store.fx_last_date(), "")
        self.assertEqual(store.upsert_fx_rates({"2026-01-02": "1.4", "bad": 1, "2026-01-03": 0}), 1)
        # A day's rate is written once and never rewritten: a later fetch cannot change it.
        store.upsert_fx_rates({"2026-01-02": 9.9})
        self.assertEqual(store.fx_rates()["2026-01-02"], 1.4)
        self.assertEqual(store.fx_rates(), {"2026-01-02": 1.4})
        self.assertEqual(store.fx_last_date(), "2026-01-02")
        store.upsert_benchmark_prices({"2026-01-02": 5000, "2026-01-05": 5100})
        self.assertEqual(store.benchmark_last_date(), "2026-01-05")
        self.assertEqual(store.market_data()["benchmark"]["2026-01-05"], 5100)

    def test_legacy_spy_meta_migrates_into_table(self):
        store.set_meta("spy_by_date", json.dumps({"2020-01-02": 3200.5, "junk": "x"}))
        with store._lock:
            conn = store._connect()
            try:
                conn.execute("DELETE FROM benchmark_prices")
                conn.commit()
                store._migrate_spy_meta(conn)
                conn.commit()
            finally:
                conn.close()
        self.assertEqual(store.benchmark_prices(), {"2020-01-02": 3200.5})

    def test_journal_roundtrip_and_version(self):
        v0 = store.data_version()
        store.save_journal_entry("rt:x", {"thesis": "t", "tags": ["a", "a", " b "], "grade": "z"})
        self.assertEqual(store.journal(), {"rt:x": {"thesis": "t", "tags": ["a", "b"], "grade": ""}})
        self.assertNotEqual(v0, store.data_version())
        store.save_journal_entry("rt:x", {"thesis": "", "tags": [], "grade": ""})
        self.assertEqual(store.journal(), {})

    def test_clear_synced_data_keeps_journal_and_market_by_default(self):
        store.merge_local_rows([
            buy("b1", "AAA", 10, 1, "2026-01-01", source="csv"),
            sell("s1", "AAA", 10, 2, "2026-01-05", source="csv"),
        ])
        store.replace_accounts([{"id": "acct-1", "nickname": "Trading"}])
        store.upsert_nav([{"date": "2026-01-05", "equity": 20, "netDeposits": 10}])
        store.set_meta("synced_at", "2026-01-05T00:00:00Z")
        store.upsert_fx_rates({"2026-01-05": 1.4})
        store.save_journal_entry("rt:b1", {"grade": "A"})
        before = store.data_summary()
        self.assertEqual(before["activities"], 2)
        self.assertEqual(before["accounts"], 1)
        self.assertEqual(before["navDays"], 1)
        self.assertEqual(before["journal"], 1)
        after = store.clear_synced_data()
        self.assertEqual(after["activities"], 0)
        self.assertEqual(after["accounts"], 0)
        self.assertEqual(after["navDays"], 0)
        self.assertEqual(after["syncedAt"], "")
        self.assertEqual(after["journal"], 1)
        self.assertEqual(after["fxDays"], 1)
        self.assertEqual(store.activity_count(), 0)
        self.assertEqual(bagholder.activity_sync_bounds(), {"start_date": None, "full_history": True})
        store.upsert_price_history("AAA", [{"date": "2026-01-05", "close": 2}], source="yahoo")
        store.upsert_price_bars("AAA", "1h", [{"time": 1767600000, "close": 2}], source="yahoo")
        store.set_meta("market_attempt_at", "2026-01-05T00:00:00Z")
        store.set_meta("tmx_form:AAA", "AAA")
        store.set_meta("yahoo_miss:AAA.V", "1")
        after = store.clear_synced_data(keep_journal=False, keep_market=False)
        self.assertEqual(after["journal"], 0)
        self.assertEqual(after["fxDays"], 0)
        self.assertEqual(store.price_history("AAA"), [])
        self.assertEqual(store.price_bars("AAA", "1h"), [])
        self.assertEqual(store.get_meta("market_attempt_at"), "")
        self.assertEqual(store.get_meta("tmx_form:AAA"), "")
        self.assertEqual(store.get_meta("yahoo_miss:AAA.V"), "")
        self.assertEqual(store.get_meta("schema_version"), str(store.SCHEMA_VERSION))

    def test_portfolio_tiles_sum_wealthsimple_figures_over_the_accounts_in_scope(self):
        acts = [
            buy("b1", "AAA", 10, 10, "2026-01-05"),                       # Trading, CAD: cost 100
            buy("b2", "BBB", 5, 20, "2026-01-06", accountType="Kids", accountId="acct-2", currency="USD"),  # cost 100 USD
        ]
        snap = {
            "activities": acts,
            "accounts": [
                {"id": "acct-1", "nickname": "Trading", "currency": "CAD", "netLiquidationValue": 1500.0, "unifiedAccountType": "SELF_DIRECTED_NON_REGISTERED_MARGIN"},
                {"id": "acct-2", "nickname": "Kids", "currency": "CAD", "netLiquidationValue": 400.0, "unifiedAccountType": "SELF_DIRECTED_JOINT_NON_REGISTERED_MARGIN"},
                {"id": "acct-3", "nickname": "Cash", "currency": "CAD", "netLiquidationValue": 25.0, "unifiedAccountType": "CASH"},
                {"id": "acct-4", "nickname": "Old", "currency": "CAD", "netLiquidationValue": 999.0, "status": "closed", "unifiedAccountType": "SELF_DIRECTED_NON_REGISTERED_MARGIN"},
                {"id": "acct-5", "nickname": "TFSA", "currency": "CAD", "netLiquidationValue": 0.0, "unifiedAccountType": "SELF_DIRECTED_TFSA"},
            ],
            "balances": [
                {"accountId": "acct-1", "securityId": "sec-c-cad", "quantity": -300.0},
                {"accountId": "acct-1", "securityId": "sec-c-usd", "quantity": -10.0},
                {"accountId": "acct-2", "securityId": "sec-c-cad", "quantity": 50.0},
                {"accountId": "acct-1", "securityId": "sec-s-aaa", "quantity": 10.0},
                {"accountId": "acct-4", "securityId": "sec-c-cad", "quantity": -5000.0},
            ],
            "securities": [
                {"id": "sec-c-cad", "symbol": "CAD", "currency": "CAD"},
                {"id": "sec-c-usd", "symbol": "USD", "currency": "USD"},
                {"id": "sec-s-aaa", "symbol": "AAA", "currency": "CAD"},
            ],
            "margin": [
                {"accountId": "acct-1", "buyingPower": 700.0, "currency": "CAD", "unavailable": ""},
                {"accountId": "acct-2", "buyingPower": None, "currency": "CAD", "unavailable": "UnavailableSecurities (1 securities)"},
                {"accountId": "acct-5", "buyingPower": 5638.24, "currency": "CAD", "unavailable": ""},   # a TFSA's buying power is its cash, not margin
            ],
        }
        market = {"fx": {"2026-02-01": 1.5}, "benchmark": {}, "quotes": {"AAA": {"price": 12.0, "priceChange": 0.5, "percentChange": 4.35}, "BBB": {"price": 30.0}}}
        base = model.build_base(snap, market, {"rt:b1": {"grade": "B", "thesis": "hold", "tags": ["core"]}}, today="2026-02-01")
        v = model.build_view(base, {})
        pf = v["portfolio"]
        self.assertAlmostEqual(pf["marketValue"], 120 + 150 * 1.5)          # AAA 10 × 12 CAD; BBB 5 × 30 USD at 1.5
        self.assertAlmostEqual(pf["costBasis"], 100 + 100 * 1.5)
        self.assertAlmostEqual(pf["unrealized"], 20 + 50 * 1.5)
        self.assertEqual((pf["positionCount"], pf["accountCount"]), (2, 2))
        self.assertAlmostEqual(pf["nav"], 1500 + 400 + 25, msg="every open account counts, cash accounts included, closed ones not")
        self.assertAlmostEqual(pf["marginUsed"], 300 + 10 * 1.5, msg="negative cash per currency, in CAD")
        self.assertEqual(pf["marginUsedBy"], {"CAD": 300.0, "USD": 10.0}, "the closed account's cash is not margin used")
        self.assertAlmostEqual(pf["availableMargin"], 700.0, "the TFSA's buying power is not counted")
        self.assertEqual(pf["availableMarginUnavailable"], ["Kids"])
        aaa = next(p for p in v["positions"] if p["symbol"] == "AAA")
        self.assertAlmostEqual(aaa["dayChange"], 10 * 0.5)
        self.assertEqual(aaa["grade"], "B")
        self.assertEqual([f["side"] for f in aaa["fills"]], ["BUY"])
        bbb = next(p for p in v["positions"] if p["symbol"] == "BBB")
        self.assertIsNone(bbb["dayChange"], "no change on the quote, no day change")
        kids = model.build_view(base, {"lists": {"account": ["Kids"]}})["portfolio"]
        self.assertAlmostEqual(kids["nav"], 400.0)
        self.assertAlmostEqual(kids["marginUsed"], 0.0)
        self.assertIsNone(kids["availableMargin"])
        self.assertEqual(kids["availableMarginUnavailable"], ["Kids"])
        self.assertAlmostEqual(kids["marketValue"], 150 * 1.5)
        empty = model.build_view(model.build_base({"activities": acts}, market, {}, today="2026-02-01"), {})["portfolio"]
        self.assertIsNone(empty["nav"])
        self.assertIsNone(empty["availableMargin"])
        self.assertEqual(empty["marginUsed"], 0.0)

    def test_model_view_from_store_and_cache(self):
        store.merge_local_rows([
            buy("b1", "AAA", 10, 1, "2026-01-01", source="csv"),
            sell("s1", "AAA", 10, 2, "2026-01-05", source="csv"),
        ])
        v = model.view(None)
        self.assertEqual(v["kpi"]["count"], 1)
        self.assertAlmostEqual(v["kpi"]["realized"], 10)
        base1 = model.base_model()
        self.assertIs(base1, model.base_model())
        tid = v["trades"][0]["id"]
        store.save_journal_entry(tid, {"grade": "B"})
        v2 = model.view(None)
        self.assertEqual(v2["trades"][0]["grade"], "B")
        self.assertEqual(v2["grades"]["buckets"][1]["n"], 1)


class SourceHealthTest(unittest.TestCase):
    def test_every_request_records_its_outcome_and_an_empty_chart_says_why(self):
        from unittest import mock
        from urllib.error import HTTPError, URLError
        market._health.clear()
        market._chart_notes.clear()
        with tempfile.TemporaryDirectory() as tmp:
            os.environ["BAGHOLDER_HOME"] = tmp
            store.set_home(tmp)
            store.ensure()
            store.save_tiles([])   # the tile row asks Yahoo too; this test counts the chart's own requests
            try:
                rec = {"symbol": "CH", "exchange": "TSX-V", "currency": "CAD", "kind": "Shares"}
                # TMX unreachable, Yahoo throttled: the chart's reason names both, the menu shows both
                def post(url, *a, **k):
                    raise URLError("no route")
                def get(url, *a, **k):
                    raise HTTPError(url, 429, "Too Many Requests", {}, None)
                # the POST path still speaks urlopen; reads go over a kept connection
                with mock.patch.object(market, "urlopen", side_effect=lambda req, **k: (_ for _ in ()).throw(URLError("no route") if b"graphql" in (req.data or b"") or "tmx" in req.full_url else HTTPError(req.full_url, 429, "Too Many Requests", {}, None))), \
                     mock.patch.object(market, "_fetch_once", side_effect=lambda url, *a, **k: (_ for _ in ()).throw(URLError("no route") if "tmx" in url else HTTPError(url, 429, "Too Many Requests", {}, None))), \
                     mock.patch.object(market, "YAHOO_MIN_INTERVAL_SEC", 0):
                    market._yahoo_backoff_until = 0.0
                    bars, src = market.fetch_history(rec, "2026-02-01", "2026-02-10")
                self.assertEqual(bars, [])
                reason = market.chart_reason(rec, "1d")
                self.assertIn("TMX Money could not be reached", reason)
                self.assertIn("Yahoo Finance refused the request (too many)", reason)
                health = {h["key"]: h for h in market.source_health()}
                self.assertFalse(health["tmx"]["ok"]); self.assertEqual(health["tmx"]["error"], "could not be reached")
                self.assertFalse(health["yahoo"]["ok"]); self.assertTrue(health["yahoo"]["error"].startswith("refused the request"), health["yahoo"]["error"])
                market._yahoo_backoff_until = 0.0
                # every source answered, none had bars: the reason names the sources asked
                with mock.patch.object(market, "_post_json", return_value={"data": {"getTimeSeriesData": [], "getQuoteBySymbol": None}}), \
                     mock.patch.object(market, "_get_text", return_value=json.dumps({"chart": {"result": []}})), mock.patch.object(market, "YAHOO_MIN_INTERVAL_SEC", 0):
                    bars, src = market.fetch_history(rec, "2026-02-01", "2026-02-10")
                self.assertEqual(market.chart_reason(rec, "1d"), "No bars for this span from TMX Money or Yahoo Finance.")
                # a 404 is the symbol, not the source
                market._health.clear()
                with mock.patch.object(market, "urlopen", side_effect=lambda req, **k: (_ for _ in ()).throw(HTTPError(req.full_url, 404, "Not Found", {}, None))), \
                     mock.patch.object(market, "_fetch_once", side_effect=lambda url, *a, **k: (_ for _ in ()).throw(HTTPError(url, 404, "Not Found", {}, None))):
                    with self.assertRaises(HTTPError):
                        market._get_text("https://query1.finance.yahoo.com/v8/finance/chart/GONE.CN")
                self.assertEqual(market.source_health(), [], "a missing symbol leaves the source's health alone")
                market.note_source("tmx", True)
                self.assertEqual([(h["name"], h["ok"]) for h in market.source_health()], [("TMX Money", True)])
            finally:
                os.environ.pop("BAGHOLDER_HOME", None)
                market._health.clear()
                market._chart_notes.clear()


class MarketParseTest(unittest.TestCase):
    def test_parsers(self):
        boc = json.dumps({"observations": [{"d": "2026-08-28", "FXUSDCAD": {"v": "1.3888"}}, {"d": "x"}]})
        self.assertEqual(market.parse_boc_json(boc), {"2026-08-28": 1.3888})
        fred = "observation_date,SP500\n2026-08-28,6500.12\n2026-08-29,.\nbad\n"
        self.assertEqual(market.parse_fred_csv(fred), {"2026-08-28": 6500.12})
        stooq = "Date,Open,High,Low,Close,Volume\n2026-08-28,1,2,0,6501.5,0\n"
        self.assertEqual(market.parse_stooq_csv(stooq), {"2026-08-28": 6501.5})

    def test_periodic_refresh_paces_fx_and_benchmark_and_refetches_stale_records(self):
        from datetime import datetime, timedelta, timezone
        with tempfile.TemporaryDirectory() as tmp:
            os.environ["BAGHOLDER_HOME"] = tmp
            store.set_home(tmp)
            store.ensure()
            try:
                boc = json.dumps({"observations": [{"d": "2026-09-04", "FXUSDCAD": {"v": "1.38"}}]})
                fred = "observation_date,SP500\n2026-09-04,7000\n"
                syms = [{"symbol": "CCHI", "exchange": "TSX", "currency": "CAD"}]
                divs = ({"price": 1.0}, [{"exDate": "2026-09-01", "payDate": "2026-09-05", "amount": 0.1, "currency": "CAD"}])
                t0 = datetime(2026, 9, 6, 12, 0, tzinfo=timezone.utc)
                with mock.patch.object(market, "_post_json", side_effect=OSError("offline")), mock.patch.object(market, "_get_text", side_effect=[boc, fred]) as g, mock.patch.object(market, "fetch_tmx", return_value=divs) as f:
                    out = market.refresh_periodic(symbols=syms, now=t0)
                self.assertEqual((out["fx"], out["benchmark"], out["distributions"]), (1, 1, 1))
                self.assertEqual((g.call_count, f.call_count), (2, 1))
                # Ten minutes later the TMX indices are still missing (offline above), so the
                # indices are attempted again regardless of the six-hour clock.
                with mock.patch.object(market, "_post_json", side_effect=OSError("offline")), mock.patch.object(market, "_get_text", side_effect=[boc, fred]) as g, mock.patch.object(market, "fetch_tmx", return_value=divs) as f:
                    out = market.refresh_periodic(symbols=syms, now=t0 + timedelta(minutes=5))
                self.assertEqual(g.call_count, 2, "a missing index does not wait for the clock")
                store.upsert_benchmark_prices({"2026-09-04": 1500.0}, symbol="TSX")
                store.upsert_benchmark_prices({"2026-09-04": 1500.0}, symbol="TSX60")
                # Ten minutes later: FX and the benchmark wait for their six hours; the record is fresh.
                with mock.patch.object(market, "_post_json", side_effect=OSError("offline")), mock.patch.object(market, "_get_text", side_effect=[boc, fred]) as g, mock.patch.object(market, "fetch_tmx", return_value=divs) as f:
                    out = market.refresh_periodic(symbols=syms, now=t0 + timedelta(minutes=10))
                self.assertEqual((out["fx"], out["benchmark"], out["distributions"]), (0, 0, 0))
                self.assertEqual((g.call_count, f.call_count), (0, 0))
                # Seven hours later FX and the benchmark are attempted again; the record is still within 20 hours.
                with mock.patch.object(market, "_post_json", side_effect=OSError("offline")), mock.patch.object(market, "_get_text", side_effect=[boc, fred]) as g, mock.patch.object(market, "fetch_tmx", return_value=divs) as f:
                    out = market.refresh_periodic(symbols=syms, now=t0 + timedelta(hours=7))
                self.assertEqual((g.call_count, f.call_count), (2, 0))
                # A day later the declared record is refetched.
                with mock.patch.object(market, "_post_json", side_effect=OSError("offline")), mock.patch.object(market, "_get_text", side_effect=[boc, fred]), mock.patch.object(market, "fetch_tmx", return_value=divs) as f:
                    out = market.refresh_periodic(symbols=syms, now=t0 + timedelta(hours=25))
                self.assertEqual(f.call_count, 1)
                # Tuesday 2026-09-08 at 16:00 Eastern: the Bank has not published, and the
                # attempt is fresh, so nothing is fetched; at 16:45 Eastern today's rate is
                # missing from the table and is fetched at once.
                store.set_meta("market_attempt_at", "2026-09-08T19:50:00Z")
                with mock.patch.object(market, "_post_json", side_effect=OSError("offline")), mock.patch.object(market, "_get_text", side_effect=[boc, fred]) as g, mock.patch.object(market, "fetch_tmx", return_value=divs):
                    market.refresh_periodic(symbols=syms, now=datetime(2026, 9, 8, 20, 0, tzinfo=timezone.utc))
                self.assertEqual(g.call_count, 0)
                with mock.patch.object(market, "_post_json", side_effect=OSError("offline")), mock.patch.object(market, "_get_text", side_effect=[boc, fred]) as g, mock.patch.object(market, "fetch_tmx", return_value=divs):
                    market.refresh_periodic(symbols=syms, now=datetime(2026, 9, 8, 20, 45, tzinfo=timezone.utc))
                self.assertEqual(g.call_count, 2)
                self.assertFalse(market.fx_day_published_but_missing(datetime(2026, 9, 12, 21, 0, tzinfo=timezone.utc)), "Saturday: nothing to publish")
            finally:
                os.environ.pop("BAGHOLDER_HOME", None)

    def test_history_parsers_and_sources(self):
        tmx = {"data": {"getTimeSeriesData": [{"dateTime": "2026-09-04T16:00:00-04:00", "open": 4.8, "high": 4.8, "low": 4.68, "close": 4.75, "volume": 50972}, {"dateTime": "2026-09-03T16:00:00-04:00", "open": 4.83, "high": 4.95, "low": 4.73, "close": 4.75, "volume": 115702}]}}
        bars = market.parse_tmx_history(tmx)
        self.assertEqual([b["date"] for b in bars], ["2026-09-03", "2026-09-04"])
        self.assertEqual(bars[1]["close"], 4.75)
        cboe = json.dumps({"data": [{"date": "2026-09-04", "open": "6.59", "close": "6.70", "high": 6.7, "low": 6.58, "volume": 53193.0}, {"date": "2026-09-03", "open": "6.56", "close": "6.76", "high": 6.76, "low": 6.54, "volume": 35377.0}]})
        bars = market.parse_cboe_ca_history(cboe)
        self.assertEqual([(b["date"], b["close"]) for b in bars], [("2026-09-03", "6.76"), ("2026-09-04", "6.70")])
        # Coinbase Exchange rows are [time, low, high, open, close, volume]
        candles = json.dumps([[1787097600, 63000.5, 65341.83, 64848.68, 63911.88, 6197.03], [1787011200, 62000, 64000, 63000, 63500, 100], ["bad"]])
        bars = market.parse_coinbase_candles(candles)
        self.assertEqual([(b["time"], b["open"], b["high"], b["low"], b["close"], b["volume"]) for b in bars], [(1787011200, 63000, 64000, 62000, 63500, 100), (1787097600, 64848.68, 65341.83, 63000.5, 63911.88, 6197.03)])
        src = market.history_source
        self.assertEqual(src({"symbol": "RDDY", "exchange": "TSX", "currency": "CAD", "kind": "Shares"}), ("tmx", "RDDY"))
        self.assertEqual(src({"symbol": "LUNR", "exchange": "NASDAQ", "currency": "USD", "kind": "Shares"}), ("tmx", "LUNR:US"))
        self.assertEqual(src({"symbol": "HBIX", "exchange": "Cboe Canada", "currency": "CAD", "kind": "Shares"}), ("tmx", "HBIX:AQL"), "history from TMX even where the quote comes from Cboe")
        self.assertEqual(src({"symbol": "ONE", "exchange": "Alpha Exchange", "currency": "CAD", "kind": "Shares"}), ("tmx", "ONE"), "an unknown venue starts from the currency's usual form")
        self.assertEqual(src({"symbol": "ASTS", "exchange": "", "currency": "USD", "kind": "Shares"}), ("tmx", "ASTS:US"))
        self.assertEqual(src({"symbol": "BTC", "exchange": "Crypto", "currency": "CAD", "kind": "Crypto"}), ("coinbase", "BTC-CAD"))
        self.assertIsNone(src({"symbol": "QNC 20NOV26 3.00 CALL", "exchange": "NYSE", "currency": "USD", "kind": "Options"}))

    def test_timeframes_aggregate_and_report_availability(self):
        from datetime import datetime, timezone
        daily = [
            {"date": "2026-08-31", "open": 1, "high": 3, "low": 0.5, "close": 2, "volume": 10},   # Monday
            {"date": "2026-09-01", "open": 2, "high": 4, "low": 1.5, "close": 3, "volume": 10},
            {"date": "2026-09-04", "open": 3, "high": 3.5, "low": 2, "close": 2.5, "volume": 10},  # Friday
            {"date": "2026-09-08", "open": 2.5, "high": 5, "low": 2, "close": 4.5, "volume": 10},  # next week
        ]
        weeks = market.aggregate_daily(daily, "1w")
        self.assertEqual([(w["date"], w["open"], w["high"], w["low"], w["close"], w["volume"]) for w in weeks], [("2026-08-31", 1, 4, 0.5, 2.5, 30), ("2026-09-07", 2.5, 5, 2, 4.5, 10)])
        months = market.aggregate_daily(daily, "1M")
        self.assertEqual([(m["date"], m["open"], m["close"]) for m in months], [("2026-08-01", 1, 2), ("2026-09-01", 2, 4.5)])
        hourly = [{"time": 3600 * h, "open": h, "high": h + 0.5, "low": h - 0.5, "close": h, "volume": 1} for h in range(1, 10)]
        four = market.aggregate_hourly(hourly, 14400)
        self.assertEqual([(b["time"], b["open"], b["high"], b["low"], b["close"], b["volume"]) for b in four], [(0, 1, 3.5, 0.5, 3, 3), (14400, 4, 7.5, 3.5, 7, 4), (28800, 8, 9.5, 7.5, 9, 2)])
        now = datetime(2026, 9, 7, 12, 0, tzinfo=timezone.utc)
        share = {"symbol": "RDDY", "exchange": "TSX", "currency": "CAD", "kind": "Shares"}
        coin = {"symbol": "BTC", "exchange": "Crypto", "currency": "CAD", "kind": "Crypto"}
        opt = {"symbol": "QNC 20NOV26 3.00 CALL", "exchange": "NYSE", "currency": "USD", "kind": "Options"}
        self.assertEqual(market.available_timeframes(share, "2024-01-01", now), ["1d", "1w", "1M"])
        self.assertEqual(market.available_timeframes(coin, "2026-08-01", now), ["1h", "4h", "1d", "1w", "1M"])
        self.assertEqual(market.available_timeframes(coin, "2019-01-01", now), ["1h", "4h", "1d", "1w", "1M"], "Coinbase keeps hourly candles for good")
        self.assertEqual(market.available_timeframes(opt, "2026-08-01", now), [])

    def test_tmx_minutes_become_session_aligned_hourly_and_four_hour_bars(self):
        from datetime import datetime, timezone
        def row(hhmm, o, h, l, c, v=1, day="2025-11-10", off="-05:00"):
            return {"dateTime": "%sT%s:00%s" % (day, hhmm, off), "open": o, "high": h, "low": l, "close": c, "volume": v}
        data = {"data": {"intraday": [
            row("09:30", 10, 11, 9, 10.5), row("09:31", 10.5, 12, 10, 11), row("10:29", 11, 11.5, 10.8, 11.2),
            row("10:30", 11.2, 11.3, 11.1, 11.25), row("13:29", 11.25, 11.4, 11.0, 11.3),
            row("13:30", 11.3, 11.6, 11.2, 11.5), row("15:59", 11.5, 11.7, 11.4, 11.6),
            row("09:30", 20, 21, 19, 20.5, day="2025-11-11"),
        ]}}
        minutes = market.parse_tmx_minutes(data)
        self.assertEqual(len(minutes), 8)
        self.assertEqual(minutes[0]["minute"], 9 * 60 + 30)
        self.assertEqual(minutes[0]["time"], int(datetime(2025, 11, 10, 14, 30, tzinfo=timezone.utc).timestamp()), "09:30 Eastern is 14:30 UTC")
        hourly = market.aggregate_session(minutes, 60)
        # 9:30-10:29, 10:30-11:29, 13:30-14:29 (13:29 falls in the 12:30 bar), 15:30-15:59, then the next day
        starts = [datetime.fromtimestamp(b["time"], tz=timezone.utc).strftime("%m-%d %H:%M") for b in hourly]
        self.assertEqual(starts, ["11-10 14:30", "11-10 15:30", "11-10 17:30", "11-10 18:30", "11-10 20:30", "11-11 14:30"])
        first = hourly[0]
        self.assertEqual((first["open"], first["high"], first["low"], first["close"], first["volume"]), (10, 12, 9, 11.2, 3))
        four = market.aggregate_session(minutes, 240)
        starts4 = [datetime.fromtimestamp(b["time"], tz=timezone.utc).strftime("%m-%d %H:%M") for b in four]
        self.assertEqual(starts4, ["11-10 14:30", "11-10 18:30", "11-11 14:30"], "9:30-13:29 and 13:30-16:00")
        self.assertEqual((four[0]["open"], four[0]["high"], four[0]["low"], four[0]["close"]), (10, 12, 9, 11.3))
        self.assertEqual((four[1]["open"], four[1]["close"]), (11.3, 11.6))

    def test_option_trades_are_charted_on_their_underlying(self):
        from datetime import datetime, timezone
        opt = {"symbol": "QNC 20NOV26 3.00 CALL", "exchange": "NYSE", "currency": "USD", "kind": "Options"}
        inst = market.chart_instrument(opt)
        self.assertEqual(inst, {"symbol": "QNC", "exchange": "NYSE", "currency": "USD", "kind": "Shares"})
        self.assertEqual(market.history_source(inst), ("tmx", "QNC:US"))
        now = datetime(2026, 9, 7, 12, 0, tzinfo=timezone.utc)
        self.assertEqual(market.available_timeframes(inst, "2026-06-01", now), ["1h", "4h", "1d", "1w", "1M"])
        share = {"symbol": "RDDY", "exchange": "TSX", "currency": "CAD", "kind": "Shares"}
        self.assertEqual(market.chart_instrument(share), share)

    def test_intraday_available_for_tmx_listings_within_a_year(self):
        from datetime import datetime, timezone
        now = datetime(2026, 9, 7, 12, 0, tzinfo=timezone.utc)
        tsla = {"symbol": "TSLA", "exchange": "NASDAQ", "currency": "USD", "kind": "Shares"}
        self.assertEqual(market.available_timeframes(tsla, "2025-11-01", now), ["1h", "4h", "1d", "1w", "1M"])
        self.assertEqual(market.available_timeframes(tsla, "2024-08-01", now), ["1d", "1w", "1M"], "beyond every source's intraday reach")
        self.assertEqual(market.available_timeframes(tsla, "2025-08-01", now), ["1h", "4h", "1d", "1w", "1M"], "past TMX's year, within Yahoo's two")
        self.assertEqual(market.intraday_reach(tsla, now), "2024-09-08")
        hbix = {"symbol": "HBIX", "exchange": "Cboe Canada", "currency": "CAD", "kind": "Shares"}
        self.assertEqual(market.available_timeframes(hbix, "2026-08-01", now), ["1h", "4h", "1d", "1w", "1M"], "Cboe Canada listings have TMX's minute bars under :AQL")

    def test_intraday_bars_are_cached_per_timeframe(self):
        from datetime import datetime, timedelta, timezone
        with tempfile.TemporaryDirectory() as tmp:
            os.environ["BAGHOLDER_HOME"] = tmp
            store.set_home(tmp)
            store.ensure()
            try:
                rec = {"symbol": "TSLA", "exchange": "NASDAQ", "currency": "USD", "kind": "Shares"}
                t0 = int(datetime(2025, 11, 10, 14, 30, tzinfo=timezone.utc).timestamp())
                by_tf = {"1h": [{"time": t0, "open": 1, "high": 2, "low": 0.5, "close": 1.5, "volume": 3}, {"time": t0 + 3600, "open": 1.5, "high": 2, "low": 1, "close": 1.8, "volume": 4}],
                         "4h": [{"time": t0, "open": 1, "high": 2, "low": 0.5, "close": 1.8, "volume": 7}]}
                now = datetime(2025, 12, 1, 12, 0, tzinfo=timezone.utc)
                with mock.patch.object(market, "fetch_intraday", return_value=(by_tf, "tmx")) as f:
                    h = market.ensure_intraday(rec, "1h", "2025-11-05", "2025-11-20", now=now)
                self.assertEqual([b["close"] for b in h], [1.5, 1.8])
                self.assertEqual(h[0]["open"], 1)
                with mock.patch.object(market, "fetch_intraday", return_value=(by_tf, "tmx")) as f:
                    four = market.ensure_intraday(rec, "4h", "2025-11-05", "2025-11-20", now=now)
                self.assertEqual(f.call_count, 0, "one minute fetch fills both timeframes")
                self.assertEqual([b["close"] for b in four], [1.8])
            finally:
                os.environ.pop("BAGHOLDER_HOME", None)

    def test_intraday_archive_covers_recent_trades_and_holdings(self):
        acts = [
            buy("b1", "OLD", 10, 5, "2024-01-05", accountType="TFSA"), sell("s1", "OLD", 10, 6, "2024-02-05", accountType="TFSA"),
            buy("b2", "NEW", 10, 5, "2026-03-01", accountType="TFSA"), sell("s2", "NEW", 10, 6, "2026-04-01", accountType="TFSA"),
            buy("b3", "NEW", 10, 5, "2026-06-01", accountType="TFSA"), sell("s3", "NEW", 10, 6, "2026-07-01", accountType="TFSA"),
            buy("b4", "HELD", 10, 5, "2025-05-01", accountType="TFSA"),
        ]
        snapshot = {"activities": acts, "accounts": [], "balances": [], "navHistory": [], "navByAccount": {}, "syncedAt": "", "tradeGroups": [], "notes": {}, "securities": []}
        base = model.build_base(snapshot, {"fx": {}, "benchmark": {}}, {}, today="2026-09-07")
        recs = model.intraday_archive_symbols(base)
        by = {r["symbol"]: r for r in recs}
        self.assertNotIn("OLD", by, "closed long before the window")
        self.assertEqual(by["NEW"]["start"], "2026-03-01", "earliest entry within the window")
        self.assertEqual(by["HELD"]["start"], "2025-09-07", "an old holding is wanted from the window start")
        opt_acts = acts + [
            act(id="o1", category="trade", activityType="BUY", rawType="OPTIONS_BUY", quantity=2, unitPrice=0.10, netCashAmount=-20, transactionDate="2026-05-05", symbol="LUNR 15JAN27 10.00 CALL", currency="USD", accountType="TFSA", securityId="sec-o-1"),
        ]
        base = model.build_base(dict(snapshot, activities=opt_acts), {"fx": {}, "benchmark": {}}, {}, today="2026-09-07")
        by = {r["symbol"]: r for r in model.intraday_archive_symbols(base)}
        self.assertIn("LUNR", by, "an option position is archived as its underlying")
        self.assertEqual((by["LUNR"]["kind"], by["LUNR"]["start"]), ("Shares", "2026-05-05"))

    def test_archive_sweep_is_paced_and_tops_up_incrementally(self):
        from datetime import datetime, timedelta, timezone
        with tempfile.TemporaryDirectory() as tmp:
            os.environ["BAGHOLDER_HOME"] = tmp
            store.set_home(tmp)
            store.ensure()
            try:
                now = datetime(2026, 9, 7, 12, 0, tzinfo=timezone.utc)
                recs = [{"symbol": s, "exchange": "TSX", "currency": "CAD", "kind": "Shares", "start": "2026-08-01"} for s in ("AAA", "BBB", "CCC")]
                recs.append({"symbol": "QNC 20NOV26 3.00 CALL", "exchange": "NYSE", "currency": "USD", "kind": "Options", "start": "2026-08-01"})
                t0 = int(datetime(2026, 8, 3, 13, 30, tzinfo=timezone.utc).timestamp())
                bars = {"1h": [{"time": t0, "open": 1, "high": 1, "low": 1, "close": 1, "volume": 1}], "4h": [{"time": t0, "open": 1, "high": 1, "low": 1, "close": 1, "volume": 1}]}
                with mock.patch.object(market, "fetch_intraday", return_value=(bars, "tmx")) as f:
                    self.assertEqual(market.archive_intraday(recs, now=now, limit=2), ["AAA", "BBB"])
                    self.assertEqual(f.call_count, 2)
                    self.assertEqual(market.archive_intraday(recs, now=now, limit=2), ["CCC"], "never-fetched first, options skipped")
                    self.assertEqual(market.archive_intraday(recs, now=now + timedelta(hours=2), limit=2), [], "fresh copies are left alone")
                # A day later each instrument is topped up from its last stored bar, not refetched from the start.
                with mock.patch.object(market, "fetch_intraday", return_value=(bars, "tmx")) as f:
                    self.assertEqual(market.archive_intraday(recs, now=now + timedelta(hours=25), limit=8), ["AAA", "BBB", "CCC"])
                    self.assertEqual(f.call_args.args[1], t0 - 2 * 86400)
                self.assertEqual(store.bar_fetch("AAA", "1h")["startTs"], int(datetime(2026, 8, 1, tzinfo=timezone.utc).timestamp()), "the archived span still starts where it began")
            finally:
                os.environ.pop("BAGHOLDER_HOME", None)

    def test_daily_archive_keeps_bars_only_for_sources_that_forget_them(self):
        from datetime import datetime, timezone
        with tempfile.TemporaryDirectory() as tmp:
            os.environ["BAGHOLDER_HOME"] = tmp
            store.set_home(tmp)
            store.ensure()
            try:
                now = datetime(2026, 9, 7, 12, 0, tzinfo=timezone.utc)
                recs = [
                    {"symbol": "HBIX", "exchange": "Cboe Canada", "currency": "CAD", "kind": "Shares", "start": "2026-06-01"},
                    {"symbol": "BTC", "exchange": "Crypto", "currency": "CAD", "kind": "Crypto", "start": "2026-06-01"},
                    {"symbol": "RDDY", "exchange": "TSX", "currency": "CAD", "kind": "Shares", "start": "2026-06-01"},
                ]
                bars = [{"date": "2026-06-02", "open": 1, "high": 1, "low": 1, "close": 1, "volume": 1}]
                with mock.patch.object(market, "fetch_history", return_value=(bars, "tmx")) as f:
                    self.assertEqual(market.archive_daily(recs, now=now), [], "every history source keeps full history itself; nothing to archive daily")
                    self.assertEqual(f.call_count, 0)
            finally:
                os.environ.pop("BAGHOLDER_HOME", None)

    def test_tmx_symbol_form_is_resolved_by_venue_and_remembered(self):
        from datetime import datetime, timezone
        with tempfile.TemporaryDirectory() as tmp:
            os.environ["BAGHOLDER_HOME"] = tmp
            store.set_home(tmp)
            store.ensure()
            try:
                venues = {"QIMC:CNX": "Canadian Securities Exchange", "HBIX:AQL": "NEO-L (Cboe Canada Listed)", "HG:US": "New York Stock Exchange"}
                asked = []
                def post(url, body, *a, **k):
                    sym = body["variables"]["symbol"]
                    asked.append((body["operationName"], sym))
                    if body["operationName"] == "getQuoteBySymbol":
                        return {"data": {"getQuoteBySymbol": {"symbol": sym, "exchangeName": venues[sym], "price": 1.0} if sym in venues else None}}
                    return {"data": {"getTimeSeriesData": [{"dateTime": "2026-02-02T16:00:00-05:00", "open": 1, "high": 1, "low": 1, "close": 1, "volume": 1}] if sym in venues else []}}
                rec = {"symbol": "QIMC", "exchange": "", "currency": "CAD", "kind": "Shares"}
                with mock.patch.object(market, "_post_json", side_effect=post), mock.patch.object(market, "_get_text", side_effect=OSError("offline")):
                    # A record with no venue asks for the bare form, gets nothing, and
                    # resolves the form whose quote names a venue: remembered from then on.
                    bars, _ = market.fetch_history(rec, "2026-02-01", "2026-02-03")
                self.assertEqual(len(bars), 1)
                self.assertEqual([s for op, s in asked if op == "getTimeSeriesData"], ["QIMC", "QIMC:CNX"])
                self.assertEqual([s for op, s in asked if op == "getQuoteBySymbol"], ["QIMC", "QIMC:CNX"], "the bare form is probed first, the CSE form answers")
                self.assertEqual(market.tmx_remembered("QIMC"), "QIMC:CNX")
                asked.clear()
                with mock.patch.object(market, "_post_json", side_effect=post), mock.patch.object(market, "_get_text", side_effect=OSError("offline")):
                    market.fetch_history(rec, "2026-02-01", "2026-02-03")
                self.assertEqual(asked, [("getTimeSeriesData", "QIMC:CNX")], "remembered: no probing, straight to the right form")
                # A Canadian record never resolves to a US form, and a miss is remembered for a day.
                asked.clear()
                hg = {"symbol": "HG", "exchange": "CSE", "currency": "CAD", "kind": "Shares"}
                with mock.patch.object(market, "_post_json", side_effect=post), mock.patch.object(market, "_get_text", side_effect=OSError("offline")):
                    self.assertEqual(market.fetch_history(hg, "2026-02-01", "2026-02-03")[0], [])
                    self.assertEqual(market.fetch_history(hg, "2026-02-01", "2026-02-03")[0], [])
                probes = [s for op, s in asked if op == "getQuoteBySymbol"]
                self.assertEqual(probes, ["HG:CNX", "HG", "HG:AQL"], "only the forms for the record's currency, once")
                self.assertEqual(market.tmx_remembered("HG"), "HG")
                # The quote path and the record path resolve the same way.
                asked.clear()
                with mock.patch.object(market, "_post_json", side_effect=post), mock.patch.object(market, "_get_text", side_effect=OSError("offline")):
                    self.assertEqual(market.fetch_tmx_quote("QIMC")["exchange"], "Canadian Securities Exchange")
                self.assertEqual(asked, [("getQuoteBySymbol", "QIMC:CNX")])
            finally:
                os.environ.pop("BAGHOLDER_HOME", None)

    def test_crypto_candles_come_from_coinbase_in_the_position_currency(self):
        from datetime import datetime, timezone
        with tempfile.TemporaryDirectory() as tmp:
            os.environ["BAGHOLDER_HOME"] = tmp
            store.set_home(tmp)
            store.ensure()
            try:
                store.upsert_fx_rates({"2026-02-05": 1.40, "2026-02-06": 1.50})   # Thursday, Friday
                fetched = []
                def get(url, *a, **k):
                    fetched.append(url)
                    if "finance.yahoo.com" in url:
                        raise OSError("404")   # Yahoo has no CAD pair for these
                    if "/products/PEPE-CAD" in url or "/products/NOPE-" in url:
                        raise OSError("404")
                    if url.endswith("/products/PEPE-USD"):
                        return json.dumps({"id": "PEPE-USD", "status": "online"})
                    if url.endswith("/products/USDC-CAD"):
                        return json.dumps({"id": "USDC-CAD", "status": "online"})
                    if "/candles?" in url:
                        # Friday 2026-02-06 and Sunday 2026-02-08 (Friday's rate), then a day with no rate at all
                        return json.dumps([[1770336000, 1.0, 3.0, 2.0, 2.5, 10], [1770508800, 1.0, 3.0, 2.0, 2.5, 10], [1769040000, 1.0, 3.0, 2.0, 2.5, 10]])
                    raise OSError("unexpected " + url)
                pepe = {"symbol": "PEPE", "exchange": "Crypto", "currency": "CAD", "kind": "Crypto"}
                self.assertEqual(market.history_candidates(pepe), [("coinbase", "PEPE-CAD"), ("yahoo", "PEPE-CAD"), ("coinbase", "PEPE-USD"), ("yahoo", "PEPE-USD")])
                with mock.patch.object(market, "_get_text", side_effect=get):
                    self.assertEqual(market.coinbase_market("PEPE-CAD"), "")
                    self.assertEqual(market.coinbase_market("USDC-CAD"), "USDC-CAD")
                    n = len(fetched)
                    self.assertEqual(market.coinbase_market("PEPE-CAD"), "")
                    self.assertEqual(market.coinbase_market("USDC-CAD"), "USDC-CAD")
                    self.assertEqual(len(fetched), n, "markets are remembered, misses for a day")
                    bars, source = market.fetch_history(pepe, "2026-01-20", "2026-02-09")
                self.assertEqual(source, "coinbase")
                self.assertEqual(store.get_meta("bars_source:PEPE"), "coinbase|PEPE-USD", "the candidate that answered is remembered")
                self.assertEqual([(b["date"], b["open"], b["high"], b["low"], b["close"], b["volume"]) for b in bars],
                                 [("2026-02-06", 3.0, 4.5, 1.5, 3.75, 10), ("2026-02-08", 3.0, 4.5, 1.5, 3.75, 10)],
                                 "USD candles at the Bank of Canada rate of the day (Sunday takes Friday's); the day with no rate within a week is dropped")
                self.assertEqual(market.in_position_currency([{"time": 1770336000, "open": 1, "high": 1, "low": 1, "close": 1, "volume": 0}], "CAD", "CAD")[0]["close"], 1, "a CAD market is used as is")
                self.assertEqual(market.in_position_currency([{"time": 1770336000, "open": 1, "high": 1, "low": 1, "close": 1, "volume": 0}], "EUR", "CAD"), [], "nothing else is converted")
                self.assertEqual(market.intraday_reach(pepe), market.COINBASE_EXCHANGE_START)
            finally:
                os.environ.pop("BAGHOLDER_HOME", None)

    def test_yahoo_is_asked_gently(self):
        from urllib.error import HTTPError
        with tempfile.TemporaryDirectory() as tmp:
            os.environ["BAGHOLDER_HOME"] = tmp
            store.set_home(tmp)
            store.ensure()
            try:
                market._yahoo_backoff_until = 0.0
                market._yahoo_next_at = 0.0
                calls = []
                kw = {}
                def get(url, *a, **k):
                    calls.append(url)
                    kw.update(k)
                    if "/GONE.CN?" in url:
                        raise HTTPError(url, 404, "Not Found", {}, None)
                    raise HTTPError(url, 429, "Too Many Requests", {}, None)
                with mock.patch.object(market, "_get_text", side_effect=get), mock.patch.object(market, "YAHOO_MIN_INTERVAL_SEC", 0):
                    self.assertEqual(market.fetch_yahoo("GONE.CN", 0, 1, "1d"), [])
                    self.assertEqual(kw.get("headers"), market.YAHOO_HEADERS, "Yahoo is asked with its own headers, not the app's usual ones")
                    self.assertEqual(market.fetch_yahoo("GONE.CN", 0, 1, "1d"), [])
                    self.assertEqual(len(calls), 1, "a symbol Yahoo does not carry is not asked again today")
                    with self.assertRaises(HTTPError):
                        market.fetch_yahoo("BUSY.TO", 0, 1, "1d")   # a real failure reaches the chain, which records it
                    with self.assertRaises(RuntimeError):
                        market.fetch_yahoo("BUSY.TO", 0, 1, "1d")
                    with self.assertRaises(RuntimeError):
                        market.fetch_yahoo("OTHER.TO", 0, 1, "60m")
                    self.assertEqual(len(calls), 2, "after a 429 nothing is asked for a while")
                market._yahoo_backoff_until = 0.0
            finally:
                os.environ.pop("BAGHOLDER_HOME", None)

    def test_background_sweep_never_asks_yahoo(self):
        from datetime import datetime, timezone
        with tempfile.TemporaryDirectory() as tmp:
            os.environ["BAGHOLDER_HOME"] = tmp
            store.set_home(tmp)
            store.ensure()
            try:
                now = datetime(2026, 9, 7, 12, 0, tzinfo=timezone.utc)
                old = {"symbol": "TSLA", "exchange": "NASDAQ", "currency": "USD", "kind": "Shares", "start": "2025-01-15"}   # past TMX's year, within Yahoo's two
                calls = []
                with mock.patch.object(market, "_get_text", side_effect=lambda url, *a, **k: calls.append(url) or (_ for _ in ()).throw(OSError("no"))), mock.patch.object(market, "_post_json", side_effect=OSError("no")):
                    self.assertEqual(market.archive_intraday([old], now=now), ["TSLA"])
                self.assertEqual([u for u in calls if "yahoo" in u], [], "the sweep leaves the rate-limited source alone")
                with mock.patch.object(market, "_get_text", side_effect=lambda url, *a, **k: calls.append(url) or (_ for _ in ()).throw(OSError("no"))), mock.patch.object(market, "_post_json", side_effect=OSError("no")):
                    market.ensure_intraday(old, "1h", "2025-01-15", "2025-02-01", now=now)
                self.assertEqual(len([u for u in calls if "yahoo" in u]), 1, "a chart someone opens does ask it")
            finally:
                os.environ.pop("BAGHOLDER_HOME", None)

    def test_a_timeframe_a_fetch_could_not_supply_is_not_asked_for_again_for_a_while(self):
        from datetime import datetime, timedelta, timezone
        with tempfile.TemporaryDirectory() as tmp:
            os.environ["BAGHOLDER_HOME"] = tmp
            store.set_home(tmp)
            store.ensure()
            try:
                now = datetime(2026, 9, 7, 12, 0, tzinfo=timezone.utc)
                rec = {"symbol": "TSLA", "exchange": "NASDAQ", "currency": "USD", "kind": "Shares"}
                self.assertFalse(market.intraday_ready(rec, "1h", "2025-01-15", now))
                with mock.patch.object(market, "fetch_intraday", return_value=({}, "")) as f:
                    self.assertEqual(market.ensure_intraday(rec, "1h", "2025-01-15", "2025-02-01", now=now), [])
                    self.assertEqual(f.call_count, 1)
                self.assertTrue(market.intraday_ready(rec, "1h", "2025-01-15", now), "nothing to wait for after a miss")
                self.assertEqual(market.offered_timeframes(rec, "2025-01-15", now), ["1d", "1w", "1M"], "the chart falls back to daily instead of an empty hourly view")
                later = now + timedelta(minutes=market.INTRADAY_RETRY_MINUTES + 1)
                self.assertFalse(market.intraday_ready(rec, "1h", "2025-01-15", later), "tried again after the retry window")
                self.assertEqual(market.offered_timeframes(rec, "2025-01-15", later), ["1h", "4h", "1d", "1w", "1M"])
            finally:
                os.environ.pop("BAGHOLDER_HOME", None)

    def test_history_chain_falls_through_to_yahoo_and_remembers_the_winner(self):
        with tempfile.TemporaryDirectory() as tmp:
            os.environ["BAGHOLDER_HOME"] = tmp
            store.set_home(tmp)
            store.ensure()
            try:
                yahoo = {"chart": {"result": [{"meta": {"exchangeTimezoneName": "America/Toronto", "gmtoffset": -14400}, "timestamp": [1770042600, 1770129000],
                         "indicators": {"quote": [{"open": [1.0, 1.1], "high": [1.2, 1.3], "low": [0.9, 1.0], "close": [1.1, 1.2], "volume": [10, 20]}]}}]}}
                asked = []
                def get(url, *a, **k):
                    asked.append(url)
                    if "/QMET.CN?" in url:
                        return json.dumps(yahoo)
                    raise OSError("404")
                posts = []
                def post(url, body, *a, **k):
                    posts.append(body["variables"]["symbol"])
                    return {"data": {"getTimeSeriesData": [], "getQuoteBySymbol": None}}
                rec = {"symbol": "QMET", "exchange": "CSE", "currency": "CAD", "kind": "Shares"}
                self.assertEqual(market.history_candidates(rec), [("tmx", "QMET:CNX"), ("yahoo", "QMET.CN"), ("yahoo", "QMET.TO"), ("yahoo", "QMET.V"), ("yahoo", "QMET.NE")], "TMX first, then Yahoo with the venue's suffix first")
                with mock.patch.object(market, "_get_text", side_effect=get), mock.patch.object(market, "_post_json", side_effect=post):
                    bars, source = market.fetch_history(rec, "2026-02-01", "2026-02-05")
                self.assertEqual(source, "yahoo")
                self.assertEqual([(b["date"], b["open"], b["close"]) for b in bars], [("2026-02-02", 1.0, 1.1), ("2026-02-03", 1.1, 1.2)], "Yahoo's daily stamps fall on the exchange's local day (February: standard time, not the offset Yahoo reports today)")
                self.assertTrue(posts, "TMX was asked first")
                self.assertEqual(store.get_meta("bars_source:QMET"), "yahoo|QMET.CN")
                posts.clear(); asked.clear()
                with mock.patch.object(market, "_get_text", side_effect=get), mock.patch.object(market, "_post_json", side_effect=post):
                    market.fetch_history(rec, "2026-02-01", "2026-02-05")
                self.assertEqual(posts, [], "the remembered winner is tried first; TMX is not asked again")
                self.assertEqual(len(asked), 1)
                # A crypto pair Coinbase has no candles for falls through to Yahoo's own CAD pair.
                usdc = {"symbol": "USDC", "exchange": "Crypto", "currency": "CAD", "kind": "Crypto"}
                def get2(url, *a, **k):
                    if url.endswith("/products/USDC-CAD"):
                        return json.dumps({"id": "USDC-CAD"})
                    if "/candles?" in url:
                        return "[]"
                    if "/USDC-CAD?" in url:
                        return json.dumps(yahoo)
                    raise OSError("404")
                with mock.patch.object(market, "_get_text", side_effect=get2):
                    bars, source = market.fetch_history(usdc, "2026-02-01", "2026-02-05")
                self.assertEqual((source, len(bars)), ("yahoo", 2))
                self.assertEqual(store.get_meta("bars_source:USDC"), "yahoo|USDC-CAD")
                # A source whose bars start well after the span does not win with a partial
                # answer when the next source has the earlier days; when no source reaches
                # the start, the one reaching furthest back wins.
                store.set_meta("bars_source:USDC", "")
                late = [[1770508800, 1.0, 1.0, 1.0, 1.4, 1]]   # Coinbase: one candle on 2026-02-08 only
                def get3(url, *a, **k):
                    if url.endswith("/products/USDC-CAD"):
                        return json.dumps({"id": "USDC-CAD"})
                    if "/candles?" in url:
                        return json.dumps(late)
                    if "/USDC-CAD?" in url:
                        return json.dumps(yahoo)   # 2026-02-02 and 2026-02-03
                    raise OSError("404")
                with mock.patch.object(market, "_get_text", side_effect=get3):
                    bars, source = market.fetch_history(usdc, "2026-01-20", "2026-02-09")
                self.assertEqual((source, bars[0]["date"]), ("yahoo", "2026-02-02"), "Yahoo reaches further back than Coinbase for this span")
                with mock.patch.object(market, "_get_text", side_effect=get3):
                    bars, source = market.fetch_history(usdc, "2026-02-01", "2026-02-09")
                self.assertEqual(source, "yahoo", "remembered, and it covers the span")
                store.set_meta("bars_source:USDC", "")
                with mock.patch.object(market, "_get_text", side_effect=get3):
                    bars, source = market.fetch_history(usdc, "2026-02-06", "2026-02-09")
                self.assertEqual(source, "coinbase", "a span Coinbase covers (within the slack) is answered by the first link")
            finally:
                os.environ.pop("BAGHOLDER_HOME", None)

    def test_partial_history_does_not_claim_the_earlier_days(self):
        from datetime import datetime, timezone
        with tempfile.TemporaryDirectory() as tmp:
            os.environ["BAGHOLDER_HOME"] = tmp
            store.set_home(tmp)
            store.ensure()
            try:
                rec = {"symbol": "USDC", "exchange": "Crypto", "currency": "CAD", "kind": "Crypto"}
                now = datetime(2026, 9, 7, 12, 0, tzinfo=timezone.utc)
                late = [{"date": "2026-02-25", "open": 1, "high": 1, "low": 1, "close": 1, "volume": 1}]
                with mock.patch.object(market, "fetch_history", return_value=(late, "coinbase")) as f:
                    market.ensure_history(rec, "2026-01-10", "2026-01-30", now=now)
                    self.assertEqual(store.history_fetch("USDC")["start"], "2026-02-25", "covered from the first bar, not from the day asked")
                    market.ensure_history(rec, "2026-01-10", "2026-01-30", now=now)
                    self.assertEqual(f.call_count, 2, "the earlier span is asked for again")
                    market.ensure_history(rec, "2026-03-01", "2026-03-10", now=now)
                    self.assertEqual(f.call_count, 2, "a span the bars do cover is served from the store")
            finally:
                os.environ.pop("BAGHOLDER_HOME", None)

    def test_history_is_cached_and_closed_days_never_rewritten(self):
        from datetime import datetime, timedelta, timezone
        with tempfile.TemporaryDirectory() as tmp:
            os.environ["BAGHOLDER_HOME"] = tmp
            store.set_home(tmp)
            store.ensure()
            try:
                rec = {"symbol": "RDDY", "exchange": "TSX", "currency": "CAD", "kind": "Shares"}
                bars = [{"date": "2026-09-03", "open": 4.83, "high": 4.95, "low": 4.73, "close": 4.75, "volume": 1}, {"date": "2026-09-04", "open": 4.8, "high": 4.8, "low": 4.68, "close": 4.75, "volume": 1}]
                now = datetime(2026, 9, 5, 12, 0, tzinfo=timezone.utc)
                with mock.patch.object(market, "fetch_history", return_value=(bars, "tmx")) as f:
                    out = market.ensure_history(rec, "2026-08-28", "2026-09-05", now=now)
                self.assertEqual([b["date"] for b in out], ["2026-09-03", "2026-09-04"])
                self.assertEqual(f.call_args.args[1:3], ("2026-08-28", "2026-09-05"))
                # Same span, minutes later: served from the store, no fetch.
                with mock.patch.object(market, "fetch_history", return_value=(bars, "tmx")) as f:
                    market.ensure_history(rec, "2026-08-28", "2026-09-05", now=now + timedelta(minutes=5))
                self.assertEqual(f.call_count, 0)
                # An older span was never fetched: fetched from that start.
                with mock.patch.object(market, "fetch_history", return_value=(bars, "tmx")) as f:
                    market.ensure_history(rec, "2026-06-01", "2026-06-30", now=now + timedelta(minutes=5))
                self.assertEqual(f.call_args.args[1], "2026-06-01")
                # A span reaching the present is refetched once the copy is a day old; a
                # closed day keeps its bar, the newest day may be replaced.
                changed = [{"date": "2026-09-03", "open": 1, "high": 1, "low": 1, "close": 1, "volume": 1}, {"date": "2026-09-04", "open": 4.8, "high": 4.9, "low": 4.68, "close": 4.85, "volume": 2}]
                with mock.patch.object(market, "fetch_history", return_value=(changed, "tmx")) as f:
                    out = market.ensure_history(rec, "2026-08-25", "2026-09-05", now=now + timedelta(hours=25))
                self.assertEqual(f.call_count, 1)
                self.assertEqual([(b["date"], b["close"]) for b in out], [("2026-09-03", 4.75), ("2026-09-04", 4.85)])
            finally:
                os.environ.pop("BAGHOLDER_HOME", None)

    def test_tsx_composite_is_fetched_from_tmx_and_stored_by_symbol(self):
        with tempfile.TemporaryDirectory() as tmp:
            os.environ["BAGHOLDER_HOME"] = tmp
            store.set_home(tmp)
            store.ensure()
            try:
                tmx = {"data": {"getTimeSeriesData": [{"dateTime": "2026-09-04T16:00:00-04:00", "open": 1, "high": 1, "low": 1, "close": 36513.8, "volume": 0}, {"dateTime": "2026-09-03T16:00:00-04:00", "open": 1, "high": 1, "low": 1, "close": 36633.12, "volume": 0}]}}
                with mock.patch.object(market, "_post_json", return_value=tmx) as p:
                    self.assertEqual(market.refresh_tsx(), 4, "two days for each of the two indices")
                asked = [c.args[1]["variables"] for c in p.call_args_list]
                self.assertEqual([a["symbol"] for a in asked], ["^TSX", "^TX60"], "the Composite and the 60")
                self.assertEqual({a["start"] for a in asked}, {"2016-01-01"}, "first fetch goes back to 2016")
                self.assertEqual(store.benchmark_prices("TSX")["2026-09-04"], 36513.8)
                self.assertEqual(store.benchmark_prices("TSX60")["2026-09-04"], 36513.8)
                self.assertEqual(store.benchmark_prices("SP500"), {}, "kept apart from the S&P 500")
                self.assertEqual(sorted(store.market_data()["benchmarks"]), ["SP500", "TSX", "TSX60"])
                from datetime import date as _d
                self.assertTrue(market.benchmark_stale(_d(2026, 9, 5)), "the S&P 500 has no closes yet")
                store.upsert_benchmark_prices({"2026-09-04": 6500.0}, symbol="SP500")
                self.assertFalse(market.benchmark_stale(_d(2026, 9, 5)))
                self.assertTrue(market.benchmark_stale(_d(2026, 10, 5)), "closes older than the stale window")
                with mock.patch.object(market, "_post_json", return_value=tmx) as p:
                    market.refresh_tsx()
                self.assertEqual({c.args[1]["variables"]["start"] for c in p.call_args_list}, {"2026-08-28"}, "later fetches start a week before the newest stored day")
            finally:
                os.environ.pop("BAGHOLDER_HOME", None)

    def test_refresh_uses_store_and_survives_errors(self):
        with tempfile.TemporaryDirectory() as tmp:
            os.environ["BAGHOLDER_HOME"] = tmp
            store.set_home(tmp)
            store.ensure()
            try:
                with mock.patch.object(market, "_post_json", side_effect=OSError("offline")), mock.patch.object(market, "_get_text", side_effect=OSError("offline")):
                    self.assertEqual(market.refresh_all(), {"fx": 0, "benchmark": 0, "distributions": 0, "skipped": False})
                self.assertTrue(market.is_stale())
                boc = json.dumps({"observations": [{"d": "2026-09-04", "FXUSDCAD": {"v": "1.38"}}]})
                fred = "observation_date,SP500\n2026-09-04,7000\n"
                with mock.patch.object(market, "_post_json", side_effect=OSError("offline")), mock.patch.object(market, "_get_text", side_effect=[boc, fred]):
                    out = market.refresh_all()
                self.assertEqual(out["fx"], 1)
                self.assertEqual(out["benchmark"], 1)
                self.assertEqual(store.fx_rates(), {"2026-09-04": 1.38})
            finally:
                store.set_home(None)
                os.environ.pop("BAGHOLDER_HOME", None)


import csvimport


CANONICAL_CSV = """transaction_date,activity_type,activity_sub_type,symbol,quantity,unit_price,net_cash_amount,currency,account_id
2026-01-05,Trade,BUY,AAA,10,5.00,-50.00,CAD,acct-1
2026-02-05,Trade,SELL,AAA,-10,6.00,60.00,CAD,acct-1
2026-02-06,Dividend,DIVIDEND,AAA,,,1.50,CAD,acct-1
"""

STATEMENT_CSV = """date,transaction,description,amount,balance,currency
2026-01-06,BUY,"AAA - Alpha Inc: Bought 10 shares (executed at 2026-01-05) at $5.00 per share",-50.00,950.00,CAD
2026-02-06,SELL,"AAA - Alpha Inc: Sold 10 shares (executed at 2026-02-05) at $6.00 per share",60.00,1010.00,CAD
2026-02-10,SELL,"LUNR 15JAN27 12.00 CALL: Sold 2 contracts (executed at 2026-02-10)",1200.00,2210.00,USD
2026-03-01,DIV,"AAA - Alpha Inc: Dividend",1.50,2211.50,CAD
As of 2026-03-02
"""

LEGACY_CSV = """Date,Action,Symbol,Quantity,Price,Amount,Currency
2026-01-05,Buy,AAA,10,5.00,-50.00,CAD
2026-02-05,Sell,AAA,10,6.00,60.00,CAD
"""


class CsvImportTest(unittest.TestCase):
    def test_helpers(self):
        self.assertEqual(csvimport.parse_number("($1,234.50)"), -1234.5)
        self.assertEqual(csvimport.parse_number("CAD 12"), 12.0)
        self.assertEqual(csvimport.parse_number("n/a"), 0.0)
        self.assertEqual(csvimport.parse_date("2026-01-05T14:00:00Z"), "2026-01-05")
        self.assertEqual(csvimport.parse_date("05/01/2026"), "2026-05-01")
        self.assertEqual(csvimport.parse_date("25/01/2026"), "2026-01-25")
        self.assertEqual(csvimport.parse_date("5-Jan-2026"), "2026-01-05")
        self.assertEqual(csvimport.parse_date("Jan 5, 2026"), "2026-01-05")
        self.assertEqual(csvimport.parse_date("46027"), "2026-01-05")
        self.assertEqual(csvimport.detect_format(["transaction_date", "activity_type", "symbol"]), "canonical")
        self.assertEqual(csvimport.detect_format(["Date", "Transaction", "Description", "Amount"]), "statement")
        self.assertEqual(csvimport.detect_format(["Date", "Action", "Symbol", "Quantity", "Price", "Amount"]), "legacy")
        self.assertEqual(csvimport.detect_format(["foo", "bar"]), "unknown")
        self.assertEqual(csvimport.book_id_from_file_name("monthly-statement-ABC12345CAD-2026-01-31.csv"), "ABC12345CAD")

    def test_canonical(self):
        r = csvimport.parse_csv(CANONICAL_CSV, "activities.csv")
        self.assertEqual(r["format"], "canonical")
        self.assertEqual(len(r["activities"]), 3)
        buy, sell, div = r["activities"]
        self.assertEqual((buy["category"], buy["activitySubType"], buy["quantity"], buy["unitPrice"]), ("trade", "BUY", 10.0, 5.0))
        self.assertEqual((sell["category"], sell["quantity"], sell["netCashAmount"]), ("trade", -10.0, 60.0))
        self.assertEqual(div["category"], "dividend")
        self.assertEqual(r["countsByType"], {"Trade": 2, "Dividend": 1})

    def test_statement_reads_fills_from_descriptions(self):
        r = csvimport.parse_csv(STATEMENT_CSV, "monthly-statement-ABC12345CAD-2026-03-31.csv")
        self.assertEqual(r["format"], "statement")
        self.assertTrue(r["footerStripped"])
        self.assertEqual(r["skipped"], [])
        buy, sell, opt, div = r["activities"]
        self.assertEqual((buy["symbol"], buy["name"], buy["quantity"], buy["unitPrice"], buy["transactionDate"], buy["settlementDate"]), ("AAA", "Alpha Inc", 10.0, 5.0, "2026-01-05", "2026-01-06"))
        self.assertEqual((sell["activitySubType"], sell["quantity"], sell["netCashAmount"]), ("SELL", -10.0, 60.0))
        # options: statement amount is contract cash, so per-share price is amount / (contracts x 100)
        self.assertEqual((opt["symbol"], opt["quantity"], opt["currency"]), ("LUNR 15JAN27 12.00 CALL", -2.0, "USD"))
        self.assertAlmostEqual(opt["unitPrice"], 6.0)
        self.assertEqual(div["category"], "dividend")
        self.assertEqual(buy["bookId"], "ABC12345CAD")

    def test_legacy_and_unknown(self):
        r = csvimport.parse_csv(LEGACY_CSV, "old.csv")
        self.assertEqual(r["format"], "legacy")
        self.assertEqual([a["activitySubType"] for a in r["activities"]], ["BUY", "SELL"])
        self.assertEqual(r["activities"][1]["quantity"], -10.0)
        r = csvimport.parse_csv("foo,bar\n1,2\n", "x.csv")
        self.assertEqual(r["format"], "unknown")
        self.assertEqual(len(r["skipped"]), 1)


class ImportStoreTest(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        os.environ["BAGHOLDER_HOME"] = self.tmp.name
        store.set_home(self.tmp.name)
        bagholder.set_home(self.tmp.name)
        store.ensure()
        model.invalidate()

    def tearDown(self):
        self.tmp.cleanup()
        os.environ.pop("BAGHOLDER_HOME", None)

    def test_import_text_merges_and_dedups(self):
        r = csvimport.import_text("activities.csv", CANONICAL_CSV)
        self.assertEqual((r["format"], r["added"], r["duplicates"]), ("canonical", 3, 0))
        r = csvimport.import_text("activities.csv", CANONICAL_CSV)
        self.assertEqual((r["added"], r["duplicates"]), (0, 3))
        v = model.view(None)
        self.assertEqual(v["kpi"]["count"], 1)
        self.assertAlmostEqual(v["kpi"]["realized"], 10)

    def test_folder_scan_skips_junk_and_unchanged_files(self):
        folder = os.path.join(self.tmp.name, "csv")
        os.makedirs(os.path.join(folder, "nested"))
        with open(os.path.join(folder, "a.csv"), "w", encoding="utf-8") as fh:
            fh.write(LEGACY_CSV)
        with open(os.path.join(folder, "._a.csv"), "w", encoding="utf-8") as fh:
            fh.write(LEGACY_CSV)
        with open(os.path.join(folder, "notes.txt"), "w", encoding="utf-8") as fh:
            fh.write("hi")
        with open(os.path.join(folder, "nested", "b.csv"), "w", encoding="utf-8") as fh:
            fh.write(CANONICAL_CSV)
        self.assertFalse(csvimport.set_watch_folder(os.path.join(folder, "missing"))["ok"])
        self.assertTrue(csvimport.set_watch_folder(folder)["ok"])
        r = csvimport.scan_folder()
        self.assertEqual([f["file"] for f in r["files"]], ["a.csv"])
        self.assertEqual(r["added"], 2)
        r = csvimport.scan_folder()
        self.assertTrue(r["files"][0]["unchanged"])
        self.assertEqual(r["added"], 0)
        with open(os.path.join(folder, "c.csv"), "w", encoding="utf-8") as fh:
            fh.write(CANONICAL_CSV)
        r = csvimport.scan_folder()
        self.assertEqual({f["file"]: f.get("unchanged") for f in r["files"]}, {"a.csv": True, "c.csv": False})
        self.assertEqual(r["added"], 3)  # a different account id, so nothing is a duplicate
        r = csvimport.scan_folder(force=True)
        self.assertEqual((r["added"], r["duplicates"]), (0, 5))
        st = csvimport.status()
        self.assertTrue(st["watching"])
        self.assertEqual(len(st["files"]), 2)
        csvimport.clear_watch_folder()
        self.assertFalse(csvimport.status()["watching"])


class DetailTest(unittest.TestCase):
    """Legs and fills are most of the model's bytes and the lists never read them:
    they travel only for the trade or holding open on the page."""

    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        os.environ["BAGHOLDER_HOME"] = self.tmp.name
        store.set_home(self.tmp.name)
        bagholder.set_home(self.tmp.name)
        store.ensure()
        model.invalidate()
        store.upsert_fx_rates({"2099-01-01": 1.0})
        store.upsert_benchmark_prices({"2099-01-01": 1.0})
        store.merge_local_rows([
            buy("b1", "AAA", 10, 1, "2026-01-01", source="csv"),
            sell("s1", "AAA", 10, 2, "2026-01-05", source="csv"),
            buy("b2", "BBB", 5, 3, "2026-01-02", source="csv"),
        ])

    def tearDown(self):
        model.invalidate()
        self.tmp.cleanup()
        os.environ.pop("BAGHOLDER_HOME", None)

    def test_the_view_carries_legs_and_fills_for_the_open_trade_only(self):
        base = model.base_model()
        trade = base["trades"][0]
        holding = base["positions"][0]
        self.assertEqual((len(trade["fills"]), len(holding["fills"])), (2, 1), "the base model keeps every fill")
        v = model.view()
        self.assertEqual([k for k in v["trades"][0] if k in ("legs", "fills")], [])
        self.assertEqual([k for k in v["positions"][0] if k in ("legs", "fills")], [])
        self.assertEqual(v["trades"][0]["legCount"], 1, "the counts stay on the row")
        v = model.view(None, trade["id"])
        self.assertEqual(len(v["trades"][0]["fills"]), 2)
        self.assertNotIn("fills", v["positions"][0])
        v = model.view(None, holding["id"])
        self.assertEqual(len(v["positions"][0]["fills"]), 1)
        self.assertNotIn("fills", v["trades"][0])
        self.assertEqual(len(base["trades"][0]["fills"]), 2, "slimming the view never touches the base model")

    def test_trade_detail_finds_a_trade_or_a_holding_by_id(self):
        base = model.base_model()
        d = model.trade_detail(base["trades"][0]["id"])
        self.assertEqual((d["id"], len(d["legs"]), [f["id"] for f in d["fills"]]), (base["trades"][0]["id"], 1, ["s1", "b1"]))
        d = model.trade_detail(base["positions"][0]["id"])
        self.assertEqual((d["legs"], [f["id"] for f in d["fills"]]), ([], ["b2"]))
        self.assertIsNone(model.trade_detail("nope"))
        self.assertIsNone(model.trade_detail(None))

    def test_a_quote_tick_reuses_the_matched_book(self):
        base = model.base_model()
        with mock.patch.object(model, "build_book", wraps=model.build_book) as bb:
            store.upsert_quote("AAA", {"price": 9.0}, source="tmx")   # a new data version, the same book
            ticked = model.base_model()
            self.assertIsNot(ticked, base, "a quote changes the data version, so the base is rebuilt")
            self.assertFalse(bb.called, "but the activity rows are not matched again")
            self.assertEqual([t["id"] for t in ticked["trades"]], [t["id"] for t in base["trades"]])
            self.assertEqual(ticked["positions"][0]["fills"], base["positions"][0]["fills"])
            store.merge_local_rows([buy("b3", "CCC", 1, 4, "2026-01-03", source="csv")])
            grown = model.base_model()
            self.assertTrue(bb.called, "a new activity row is matched")
            self.assertEqual(sorted(p["symbol"] for p in grown["positions"]), ["BBB", "CCC"])
            self.assertEqual(grown["activityCount"], 4)


class ServerTest(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        os.environ["BAGHOLDER_HOME"] = self.tmp.name
        store.set_home(self.tmp.name)
        bagholder.set_home(self.tmp.name)
        store.ensure()
        model.invalidate()
        store.save_tiles([])   # an empty tile row: a model request must not start a quote read over the network
        store.upsert_fx_rates({"2099-01-01": 1.0})
        store.upsert_benchmark_prices({"2099-01-01": 1.0})
        self.httpd = ThreadingHTTPServer(("127.0.0.1", 0), bagholder.Handler)
        self.port = self.httpd.server_address[1]
        threading.Thread(target=self.httpd.serve_forever, daemon=True).start()

    def tearDown(self):
        self.httpd.shutdown()
        self.httpd.server_close()
        self.tmp.cleanup()
        os.environ.pop("BAGHOLDER_HOME", None)

    def _get(self, path):
        req = Request("http://127.0.0.1:%d%s" % (self.port, path))
        with urlopen(req, timeout=10) as r:
            return r.status, r.read()

    def _post(self, path, body):
        req = Request(
            "http://127.0.0.1:%d%s" % (self.port, path),
            data=json.dumps(body).encode("utf-8"),
            headers={"Content-Type": "application/json", "X-Bagholder": "1"},
            method="POST",
        )
        with urlopen(req, timeout=10) as r:
            return r.status, json.loads(r.read().decode("utf-8"))

    def test_v2_page_and_model_route(self):
        status, body = self._get("/v2")
        self.assertEqual(status, 200)
        html = body.decode("utf-8")
        self.assertIn('<link rel="icon" type="image/png" href="favicon.png"', html)
        self.assertIn("/api/model", html)
        self.assertIn("/api/journal", html)
        store.merge_local_rows([
            buy("b1", "AAA", 10, 1, "2026-01-01", source="csv"),
            sell("s1", "AAA", 10, 2, "2026-01-05", source="csv"),
        ])
        status, body = self._get("/api/model")
        self.assertEqual(status, 200)
        data = json.loads(body.decode("utf-8"))
        self.assertTrue(data["ok"])
        self.assertEqual(data["kpi"]["count"], 1)
        self.assertIn("status", data)
        from urllib.parse import quote
        status, body = self._get("/api/model?filters=" + quote(json.dumps({"lists": {"symbol": ["ZZZ"]}})))
        data = json.loads(body.decode("utf-8"))
        self.assertEqual(data["kpi"]["count"], 0)
        self.assertEqual(data["tradeTotal"], 1)

    def test_the_model_route_takes_the_open_trade_and_the_trade_route_serves_its_detail(self):
        store.merge_local_rows([
            buy("b1", "AAA", 10, 1, "2026-01-01", source="csv"),
            sell("s1", "AAA", 10, 2, "2026-01-05", source="csv"),
        ])
        _, body = self._get("/api/model")
        row = json.loads(body.decode("utf-8"))["trades"][0]
        self.assertNotIn("fills", row)
        from urllib.parse import quote
        _, body = self._get("/api/model?trade=" + quote(row["id"]))
        self.assertEqual(len(json.loads(body.decode("utf-8"))["trades"][0]["fills"]), 2)
        status, body = self._get("/api/trade?id=" + quote(row["id"]))
        data = json.loads(body.decode("utf-8"))
        self.assertEqual((status, data["ok"], data["id"], len(data["legs"]), [f["id"] for f in data["fills"]]), (200, True, row["id"], 1, ["s1", "b1"]))
        from urllib.error import HTTPError
        with self.assertRaises(HTTPError) as cm:
            self._get("/api/trade?id=nope")
        self.assertEqual(cm.exception.code, 404)

    def test_journal_post_persists(self):
        store.merge_local_rows([
            buy("b1", "AAA", 10, 1, "2026-01-01", source="csv"),
            sell("s1", "AAA", 10, 2, "2026-01-05", source="csv"),
        ])
        _, data = self._get("/api/model")
        tid = json.loads(data.decode("utf-8"))["trades"][0]["id"]
        status, out = self._post("/api/journal", {"id": tid, "thesis": "why", "tags": ["a"], "grade": "A"})
        self.assertEqual(status, 200)
        self.assertEqual(out["journal"][tid]["grade"], "A")
        self.assertEqual(store.journal()[tid]["thesis"], "why")
        _, data = self._get("/api/model")
        self.assertEqual(json.loads(data.decode("utf-8"))["trades"][0]["tags"], ["a"])

    def test_data_routes_clear_and_disconnect(self):
        store.merge_local_rows([
            buy("b1", "AAA", 10, 1, "2026-01-01", source="csv"),
            sell("s1", "AAA", 10, 2, "2026-01-05", source="csv"),
        ])
        bagholder.save_session({"access_token": "x", "refresh_token": "y"})
        status, body = self._get("/api/data")
        data = json.loads(body.decode("utf-8"))
        self.assertEqual(data["activities"], 2)
        self.assertTrue(data["sessionPresent"])
        self.assertIn("bagholder.db", data["path"])
        _, data = self._get("/api/model")
        self.assertEqual(json.loads(data.decode("utf-8"))["kpi"]["count"], 1)
        status, out = self._post("/api/data/clear", {"session": True})
        self.assertEqual(status, 200)
        self.assertEqual(out["activities"], 0)
        self.assertFalse(out["sessionPresent"])
        self.assertIsNone(bagholder.load_session())
        _, data = self._get("/api/model")
        payload = json.loads(data.decode("utf-8"))
        self.assertEqual(payload["kpi"]["count"], 0)
        self.assertEqual(payload["activityCount"], 0)
        html = bagholder.ledger_path().read_text(encoding="utf-8")
        self.assertIn("/api/data/clear", html)
        self.assertIn("Clear data", html)

    def test_clear_data_from_the_page_keeps_the_login(self):
        store.merge_local_rows([
            buy("b1", "AAA", 10, 1, "2026-01-01", source="csv"),
            sell("s1", "AAA", 10, 2, "2026-01-05", source="csv"),
        ])
        store.upsert_fx_rates({"2026-01-05": 1.4})
        store.save_journal_entry("rt:b1", {"grade": "A"})
        bagholder.save_session({"access_token": "x", "refresh_token": "y"})
        status, out = self._post("/api/data/clear", {"journal": True, "market": True})
        self.assertEqual(status, 200)
        self.assertEqual(out["activities"], 0)
        self.assertEqual(out["journal"], 0)
        self.assertEqual(out["fxDays"], 0)
        self.assertTrue(out["sessionPresent"])
        self.assertIsNotNone(bagholder.load_session())
        html = bagholder.ledger_path().read_text(encoding="utf-8")
        self.assertIn('{ journal: true, market: true }', html)
        self.assertNotIn("session: true", html)
        self.assertIn("Your Wealthsimple login stays.", html)

    def _stub_http(self, post_response, calls, delay=0.0):
        def fake(method, url, body=None, headers=None, timeout=60):
            calls.append((method, url.rsplit("/", 1)[-1], dict(body or {})))
            if method == "GET" and url.endswith("/token/info"):
                return {"identity_canonical_id": "identity-1", "email": "who@example.com", "client_id": "client-1"}
            if delay:
                time.sleep(delay)
            return dict(post_response)
        return fake

    def test_refresh_runs_one_at_a_time_and_rotates_once(self):
        bagholder.save_session({"access_token": "a1", "refresh_token": "r1", "client_id": "client-1"})
        calls = []
        fake = self._stub_http({"access_token": "a2", "refresh_token": "r2", "expires_in": 1800}, calls, delay=0.2)
        results = []
        with mock.patch.object(bagholder, "_http_json", fake):
            threads = [threading.Thread(target=lambda: results.append(bagholder.refresh_session(bagholder.load_session()))) for _ in range(2)]
            for t in threads:
                t.start()
            for t in threads:
                t.join()
        self.assertEqual(results, [True, True])
        self.assertEqual([c[:2] for c in calls], [("POST", "token")], "the second thread adopts the rotated login instead of posting the used token")
        self.assertEqual(bagholder.load_session()["refresh_token"], "r2")

    def test_capture_makes_the_login_its_own_by_refreshing(self):
        calls = []
        fake = self._stub_http({"access_token": "a2", "refresh_token": "r2", "expires_in": 1800}, calls)
        with mock.patch.object(bagholder, "_http_json", fake), mock.patch.object(bagholder, "run_sync", lambda *a, **k: True):
            result = bagholder.capture_tokens({"access_token": "a1", "refresh_token": "r1", "client_id": "client-1", "wssdi": "device-1"})
        self.assertTrue(result["ok"])
        sess = bagholder.load_session()
        self.assertEqual((sess["access_token"], sess["refresh_token"]), ("a2", "r2"), "the browser's copy is stale, the app's is current")
        self.assertEqual([c[:2] for c in calls if c[0] == "POST"], [("POST", "token")])
        self.assertEqual(calls[-1][2]["refresh_token"], "r1")
        self.assertTrue(bagholder.status_payload()["connected"])

    def test_capture_refused_on_refresh_is_not_a_connection(self):
        calls = []
        fake = self._stub_http({"_http_status": 401, "error": "invalid_grant"}, calls)
        with mock.patch.object(bagholder, "_http_json", fake), mock.patch.object(bagholder, "run_sync", lambda *a, **k: True):
            result = bagholder.capture_tokens({"access_token": "a1", "refresh_token": "r-dead", "client_id": "client-1"})
        self.assertFalse(result["ok"])
        self.assertEqual(result["error"], bagholder.REFUSED_LOGIN_MESSAGE)
        self.assertIsNone(bagholder.load_session(), "a refused capture is not saved")
        self.assertFalse(bagholder.status_payload()["connected"])

    def test_a_refused_token_is_never_posted_again(self):
        bagholder.save_session({"access_token": "a1", "refresh_token": "r-dead-2", "client_id": "client-1"})
        calls = []
        fake = self._stub_http({"_http_status": 401, "error": "invalid_grant"}, calls)
        with mock.patch.object(bagholder, "_http_json", fake):
            first = bagholder.refresh_session(bagholder.load_session())
            second = bagholder.refresh_session(bagholder.load_session())
        self.assertEqual((first, second), (False, False))
        self.assertEqual(len([c for c in calls if c[0] == "POST"]), 1, "the loop's next tries do not post the dead token")
        self.assertEqual(bagholder.status_payload()["error"], bagholder.REFUSED_LOGIN_MESSAGE)
        self.assertIsNotNone(bagholder.load_session(), "the file stays; Disconnect or Connect replaces it")

    def test_import_watch_and_manual_trade_routes(self):
        status, out = self._post("/api/import", {"name": "activities.csv", "text": CANONICAL_CSV})
        self.assertEqual(status, 200)
        self.assertEqual((out["format"], out["added"]), ("canonical", 3))
        _, data = self._get("/api/model")
        self.assertEqual(json.loads(data.decode("utf-8"))["kpi"]["count"], 1)
        status, out = self._post("/api/book/append", {"date": "2026-03-01", "symbol": "bbb", "side": "BUY", "qty": 5, "price": 2, "currency": "CAD", "commission": 1, "accountId": "manual", "accountType": "Manual"})
        self.assertEqual(status, 200)
        self.assertEqual(out["added"], 1)
        _, data = self._get("/api/model")
        payload = json.loads(data.decode("utf-8"))
        self.assertEqual([p["symbol"] for p in payload["positions"]], ["BBB"])
        self.assertEqual(payload["positions"][0]["fees"], 1.0)
        folder = os.path.join(self.tmp.name, "csv")
        os.makedirs(folder)
        with open(os.path.join(folder, "old.csv"), "w", encoding="utf-8") as fh:
            fh.write(LEGACY_CSV)
        status, out = self._post("/api/watch", {"path": folder})
        self.assertEqual(status, 200)
        self.assertEqual(out["added"], 2)
        status, out = self._post("/api/watch/scan", {})
        self.assertEqual((out["added"], out["duplicates"]), (0, 2))
        self.assertTrue(out["status"]["watching"])
        status, body = self._get("/api/watch")
        self.assertTrue(json.loads(body.decode("utf-8"))["watching"])
        status, out = self._post("/api/watch/clear", {})
        self.assertFalse(out["watching"])
        html = bagholder.ledger_path().read_text(encoding="utf-8")
        for needle in ("/api/import", "/api/watch", "Add trade", "Load folder"):
            self.assertIn(needle, html)

    def test_root_serves_the_page(self):
        status, body = self._get("/")
        self.assertEqual(status, 200)
        self.assertIn(b"/api/model", body)
        status, body = self._get("/v2")
        self.assertEqual(status, 200)
        self.assertIn(b"/api/model", body)


if __name__ == "__main__":
    unittest.main()


class PriceTickTest(unittest.TestCase):
    """A quote moving must not match the book again, and must give the same model."""

    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        os.environ["BAGHOLDER_HOME"] = self.tmp.name
        store.set_home(self.tmp.name)
        bagholder.set_home(self.tmp.name)
        store.ensure()
        model.invalidate(book=True)
        store.save_tiles([])
        for row in (
            buy("b1", "AAA", 100, 10, "2026-01-05", accountType="Trading"),
            sell("s1", "AAA", 100, 12, "2026-02-05", accountType="Trading"),
            buy("b2", "BBB", 20, 50, "2026-03-05", accountType="Trading"),
        ):
            store.insert_local(row)
        store.replace_accounts([{"id": "acct-1", "nickname": "Trading", "unifiedAccountType": "TFSA", "currency": "CAD"}])
        store.upsert_quote("BBB", {"price": 60.0, "currency": "CAD"}, source="tmx")
        model.invalidate(book=True)

    def tearDown(self):
        store.close_all()
        self.tmp.cleanup()
        store.set_home(None)
        os.environ.pop("BAGHOLDER_HOME", None)
        model.invalidate(book=True)

    def test_a_price_tick_marks_the_same_model_without_rematching(self):
        first = model.base_model()
        self.assertTrue(first["positions"], "the open BBB lot is a position")
        self.assertTrue(first["trades"], "and the closed AAA round trip is a trade")
        built = []
        original = model.build_book
        model.build_book = lambda *a, **k: (built.append(1), original(*a, **k))[1]
        try:
            store.upsert_quote("BBB", {"price": 70.0, "currency": "CAD"}, source="tmx")
            marked = model.base_model()
            self.assertEqual(built, [], "a price tick does not match the book again")
            model.invalidate(book=True)
            full = model.base_model(force=True)
        finally:
            model.build_book = original
        self.assertEqual(marked["positions"], full["positions"], "marked positions equal a full rebuild")
        self.assertEqual(marked["trades"], full["trades"], "and so do the closed trades")
        self.assertEqual(marked["cashflow"], full["cashflow"])
        self.assertEqual(marked["equity"], full["equity"])
        self.assertEqual(
            model.slim(model.build_view(marked, None)),
            model.slim(model.build_view(full, None)),
            "the page is served exactly what a full rebuild would have produced",
        )

    def test_a_new_row_does_match_the_book_again(self):
        model.base_model()
        built = []
        original = model.build_book
        model.build_book = lambda *a, **k: (built.append(1), original(*a, **k))[1]
        try:
            store.insert_local(buy("b3", "CCC", 5, 20, "2026-04-05", accountType="Trading"))
            model.base_model()
        finally:
            model.build_book = original
        self.assertEqual(len(built), 1, "an activity row is a new book")

    def test_invalidate_keeps_the_match_and_book_true_drops_it(self):
        model.base_model()
        model.invalidate()
        self.assertIsNotNone(model._book["book"], "a quote or a headline does not throw the match away")
        model.invalidate(book=True)
        self.assertIsNone(model._book["book"], "new rows do")

    def test_a_grade_written_survives_the_next_price_tick(self):
        base = model.base_model()
        position = base["positions"][0]
        store.save_journal_entry(position["id"], {"grade": "A", "thesis": "held", "tags": ["core"]})
        model.apply_journal(store.journal())
        store.upsert_quote("BBB", {"price": 80.0, "currency": "CAD"}, source="tmx")
        marked = model.base_model()
        again = [p for p in marked["positions"] if p["id"] == position["id"]]
        self.assertEqual(again[0]["grade"], "A", "the grade is marked in, not dropped with the old journal")
        self.assertEqual(again[0]["tags"], ["core"])


class ModelRequestRefreshTest(unittest.TestCase):
    """The request path must not start a refresh per request."""

    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        os.environ["BAGHOLDER_HOME"] = self.tmp.name
        store.set_home(self.tmp.name)
        bagholder.set_home(self.tmp.name)
        store.ensure()
        model.invalidate(book=True)
        store.save_tiles([])
        bagholder._jobs.clear()
        self.httpd = ThreadingHTTPServer(("127.0.0.1", 0), bagholder.Handler)
        self.port = self.httpd.server_address[1]
        threading.Thread(target=self.httpd.serve_forever, daemon=True).start()

    def tearDown(self):
        self.httpd.shutdown()
        self.httpd.server_close()
        store.close_all()
        self.tmp.cleanup()
        store.set_home(None)
        os.environ.pop("BAGHOLDER_HOME", None)
        bagholder._jobs.clear()

    def _get(self, path):
        req = Request("http://127.0.0.1:%d%s" % (self.port, path), headers={"X-Bagholder": "1"})
        with urlopen(req, timeout=5) as r:
            return json.loads(r.read().decode("utf-8"))

    def test_twenty_page_loads_start_one_quote_read(self):
        gate = threading.Event()
        started = []

        def slow_quotes():
            started.append(1)
            gate.wait(3)
            return 0

        with mock.patch.object(bagholder, "refresh_quotes", bagholder.single_flight("quotes")(slow_quotes)), \
             mock.patch.object(market, "is_stale", lambda **k: False), \
             mock.patch.object(market, "quote_symbols_needing_refresh", lambda *a, **k: [("AAA", "tmx", "AAA")]):
            for _ in range(20):
                self.assertTrue(self._get("/api/model?filters=%7B%7D")["ok"])
            gate.set()
            time.sleep(0.3)
        self.assertEqual(len(started), 1, "one quote read, not one per request")

    def test_the_status_payload_does_not_read_the_activity_table(self):
        reads = []
        original = store.snapshot
        store.snapshot = lambda *a, **k: (reads.append(1), original(*a, **k))[1]
        try:
            self.assertTrue(self._get("/api/status")["ok"])
        finally:
            store.snapshot = original
        self.assertEqual(reads, [], "the header's counts come from the database, not from every row in it")


class SingleFlightTest(unittest.TestCase):
    """One refresh of a kind at a time, and a request path that cannot pile them up."""

    def setUp(self):
        bagholder._jobs.clear()

    def tearDown(self):
        bagholder._jobs.clear()

    def test_a_second_call_returns_at_once_instead_of_running(self):
        started = threading.Event()
        release = threading.Event()
        ran = []

        @bagholder.single_flight("test-job")
        def work():
            ran.append(1)
            started.set()
            release.wait(2)
            return "done"

        t = threading.Thread(target=work, daemon=True)
        t.start()
        self.assertTrue(started.wait(2), "the first call is running")
        self.assertEqual(work(), 0, "the second is told the kind is busy and does nothing")
        self.assertEqual(len(ran), 1)
        release.set()
        t.join(2)
        self.assertEqual(work(), "done", "once it is free the next call runs")
        self.assertEqual(len(ran), 2)

    def test_a_kick_starts_one_thread_and_then_holds_for_the_cooldown(self):
        calls = []
        gate = threading.Event()

        @bagholder.single_flight("test-kick")
        def work():
            calls.append(1)
            gate.wait(2)

        bagholder._COOLDOWN["test-kick"] = 30.0
        try:
            self.assertTrue(bagholder.kick("test-kick", work), "the first request starts it")
            for _ in range(20):
                bagholder.kick("test-kick", work)   # a page reload, a filter, a poll
            gate.set()
            time.sleep(0.2)
            self.assertEqual(len(calls), 1, "twenty more requests start nothing")
            self.assertFalse(bagholder.kick("test-kick", work), "and the cooldown holds after it finishes")
        finally:
            bagholder._COOLDOWN.pop("test-kick", None)

    def test_a_busy_refresh_returns_what_its_caller_expects(self):
        # the archive loop measures what it did; a busy answer must not be None
        gate = threading.Event()

        @bagholder.single_flight("test-shape", busy=())
        def archive():
            gate.wait(2)
            return ["AAA", "BBB"]

        t = threading.Thread(target=archive, daemon=True)
        t.start()
        time.sleep(0.1)
        busy = archive()
        self.assertEqual(len(busy), 0, "a busy archive pass reports no work, and can still be measured")
        gate.set()
        t.join(2)

        @bagholder.single_flight("test-shape-2", busy={})
        def market():
            gate.wait(2)
            return {"fx": 1}

        gate.clear()
        t2 = threading.Thread(target=market, daemon=True)
        t2.start()
        time.sleep(0.1)
        self.assertEqual(market().get("quotes"), None, "and a busy market read answers a dict, as its caller reads one")
        gate.set()
        t2.join(2)

    def test_the_cooldown_expires(self):
        calls = []

        @bagholder.single_flight("test-cool")
        def work():
            calls.append(1)

        bagholder._COOLDOWN["test-cool"] = 0.05
        try:
            bagholder.kick("test-cool", work)
            time.sleep(0.3)
            self.assertTrue(bagholder.kick("test-cool", work), "a source is tried again once its cooldown passes")
            time.sleep(0.2)
            self.assertEqual(len(calls), 2)
        finally:
            bagholder._COOLDOWN.pop("test-cool", None)
