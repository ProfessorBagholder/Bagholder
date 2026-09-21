"""The published fear and greed indexes: what each publisher's answer becomes, and how a
reading is kept so the meter is drawn with the page."""
from __future__ import annotations

import json
import os
import sys
import tempfile
import unittest
from unittest import mock

sys.path.insert(0, os.path.dirname(os.path.dirname(os.path.abspath(__file__))))

import bagholder  # noqa: E402
import fear  # noqa: E402
import store  # noqa: E402

CNN = {
    "fear_and_greed": {"score": 28.6571428571429, "rating": "fear", "timestamp": "2026-09-15T23:59:51+00:00",
                       "previous_close": 31.0571428571429, "previous_1_week": 39.142857142857146,
                       "previous_1_month": 64.31428571428572, "previous_1_year": 64.45714285714287},
    "fear_and_greed_historical": {"data": [{"x": 1789516791000.0, "y": 28.6571428571429, "rating": "fear"},
                                           {"x": 1757980800000.0, "y": 64.37142857142858, "rating": "greed"}]},
    "market_momentum_sp125": {"score": 22.8, "rating": "extreme fear", "data": []},
    "market_momentum_sp500": {"score": 99.0, "rating": "extreme greed", "data": []},
    "stock_price_strength": {"score": 1, "rating": "extreme fear", "data": []},
    "stock_price_breadth": {"score": 5, "rating": "extreme fear", "data": []},
    "put_call_options": {"score": 32.2, "rating": "fear", "data": []},
    "market_volatility_vix_50": {"score": 50, "rating": "neutral", "data": []},
    "market_volatility_vix": {"score": 12, "rating": "extreme fear", "data": []},
    "junk_bond_demand": {"score": 58.6, "rating": "greed", "data": []},
    "safe_haven_demand": {"score": 31, "rating": "fear", "data": []},
}
CRYPTO = {"data": [{"value": "51", "value_classification": "Neutral", "timestamp": "1789516800"},
                   {"value": "69", "value_classification": "Greed", "timestamp": "1789430400"}]}


class ScaleTest(unittest.TestCase):
    def test_a_score_is_named_on_the_publishers_own_scale(self):
        self.assertEqual([fear.band(v) for v in (0, 24.9, 25, 44.9, 45, 55, 56, 75.9, 76, 100)],
                         ["Extreme fear", "Extreme fear", "Fear", "Fear", "Neutral", "Neutral",
                          "Greed", "Greed", "Extreme greed", "Extreme greed"])

    def test_the_publishers_own_word_is_kept_where_it_gives_one(self):
        self.assertEqual(fear.rating("extreme fear", 90), "Extreme fear", "the publisher's word, not the scale's")
        self.assertEqual(fear.rating("", 90), "Extreme greed", "and the scale's where it gives none")


class StocksTest(unittest.TestCase):
    def test_the_reading_its_comparisons_its_seven_indicators_and_its_history(self):
        rec = fear.parse_stocks(CNN)
        self.assertEqual((rec["index"], rec["source"], rec["score"], rec["rating"]), ("stocks", "CNN", 28.7, "Fear"))
        self.assertEqual(rec["asOf"], "2026-09-15T23:59:51Z")
        self.assertEqual([(r["label"], r["score"], r["rating"]) for r in rec["previous"]],
                         [("Previous close", 31.1, "Fear"), ("A week ago", 39.1, "Fear"),
                          ("A month ago", 64.3, "Greed"), ("A year ago", 64.5, "Greed")])
        self.assertEqual([p["name"] for p in rec["parts"]],
                         ["Market momentum", "Stock price strength", "Stock price breadth", "Put and call options",
                          "Market volatility", "Junk bond demand", "Safe haven demand"])
        self.assertEqual([p["score"] for p in rec["parts"][:1]], [22.8], "the 125-day momentum CNN's own page names")
        self.assertEqual(rec["parts"][4]["score"], 50, "and the VIX's 50-day average, not the other form in the answer")
        self.assertEqual([p["date"] for p in rec["series"]], ["2025-09-16", "2026-09-15"], "oldest first")

    def test_an_answer_with_no_score_is_no_reading(self):
        self.assertEqual(fear.parse_stocks({"fear_and_greed": {}}), {})
        self.assertEqual(fear.parse_stocks(None), {})


class CryptoTest(unittest.TestCase):
    def test_the_days_own_reading_and_the_days_behind_it(self):
        rec = fear.parse_crypto(CRYPTO)
        self.assertEqual((rec["index"], rec["source"], rec["score"], rec["rating"]), ("crypto", "Alternative.me", 51.0, "Neutral"))
        self.assertEqual(rec["asOf"], "2026-09-16T00:00:00Z")
        self.assertEqual([(r["label"], r["score"]) for r in rec["previous"]], [("Yesterday", 69.0)],
                         "only the days the publisher gave")
        self.assertEqual(rec["parts"], [], "it publishes no indicators under the index")
        self.assertEqual([p["date"] for p in rec["series"]], ["2026-09-15", "2026-09-16"])

    def test_an_empty_answer_is_no_reading(self):
        self.assertEqual(fear.parse_crypto({"data": []}), {})


class KeptTest(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        os.environ["BAGHOLDER_HOME"] = self.tmp.name
        store.set_home(self.tmp.name)
        bagholder.set_home(self.tmp.name)
        store.ensure()

    def tearDown(self):
        self.tmp.cleanup()
        os.environ.pop("BAGHOLDER_HOME", None)

    def test_a_reading_is_stored_whole_and_read_back_whole(self):
        rec = fear.parse_stocks(CNN)
        store.save_gauge("stocks", rec, version=bagholder.FEAR_VERSION)
        back = store.gauge("stocks")
        self.assertEqual((back["score"], back["rating"], back["source"], back["asOf"]), (28.7, "Fear", "CNN", rec["asOf"]))
        self.assertEqual(len(back["parts"]), 7)
        self.assertEqual(back["series"], rec["series"])
        self.assertEqual(back["readVersion"], bagholder.FEAR_VERSION)

    def test_the_meter_is_answered_from_the_store_and_refreshed_behind_the_page(self):
        store.save_gauge("stocks", fear.parse_stocks(CNN), version=bagholder.FEAR_VERSION)
        with mock.patch.object(fear, "read", side_effect=AssertionError("read anyway")):
            out = bagholder.fear_payload("stocks")
        self.assertEqual(out["gauge"]["score"], 28.7)
        # a reading past its minutes is answered at once and read again behind the page
        store.save_gauge("stocks", fear.parse_stocks(CNN), now="2026-09-15T00:00:00Z", version=bagholder.FEAR_VERSION)
        with mock.patch.object(bagholder, "kick") as kicked:
            self.assertEqual(bagholder.fear_payload("stocks")["gauge"]["score"], 28.7)
        self.assertEqual(kicked.call_count, 1)

    def test_an_index_nobody_publishes_is_refused_and_a_silent_publisher_is_said(self):
        self.assertFalse(bagholder.fear_payload("vibes")["ok"])
        with mock.patch.object(fear, "read", return_value={}):
            self.assertFalse(bagholder.fear_payload("crypto")["ok"])

    def test_the_sweep_reads_only_what_is_stale(self):
        store.save_gauge("stocks", fear.parse_stocks(CNN), version=bagholder.FEAR_VERSION)
        with mock.patch.object(fear, "read", return_value=fear.parse_crypto(CRYPTO)) as read:
            self.assertEqual(bagholder.sweep_fear(), 1, "the crypto one only")
        self.assertEqual([c.args[0] for c in read.call_args_list], ["crypto"])


if __name__ == "__main__":
    unittest.main()
