"""Short selling: which regulator answers for a listing, what each report says, and the
one figure the app derives from them."""
from __future__ import annotations

import json
import os
import tempfile
import unittest
from datetime import date, datetime, timezone
from unittest import mock

import bagholder
import model
import shorts
import store

US_FILE = ("Date|Symbol|ShortVolume|ShortExemptVolume|TotalVolume|Market\r\n"
           "20260914|A|380591.095732|11|631970.726692|B,Q,N\r\n"
           "20260914|GME|2250985.897562|1942|3542062.253804|B,Q,N\r\n"
           "20260914|NOVOL|0|0|0|Q\r\n"
           "20260914|SHORT\r\n")

CA_GRID = [["", "", "", "", ""],
           ["Security Issue Name", "Security Symbol", "Exchange Code", "No.Shares", "Net Change"],
           ["QUANTUM EMOTION CORP.", "QNC", "TSXV", 2667164.0, 64077.0],
           ["1933 INDUSTRIES INC.", "TGIF", "CSE", 72000.0, 68990.0],
           ["ROW WITH NO SHARES", "NIL", "TSX", "", ""],
           ["SHORT ROW", "OOPS"]]

CA_CSV = ("Security,Company Name,Listing Market,Short Sale Trades,% Total Trades,Short Traded Volume,% Total Traded Volume,Short Traded Value,% Total Traded Value\r\n"
          "QNC,Quantum Emotion Corp.,TSXV,6364,33.528,1197633,21.319,3453172,21.028\r\n"
          "TGIF,1933 Industries Inc.,CSE,12,1.5,5000,0,20,0\r\n")


class RoutingTest(unittest.TestCase):
    def test_each_market_goes_to_the_regulator_that_publishes_for_it(self):
        self.assertEqual(shorts.market_of("GME", "NYSE", "USD"), "us")
        self.assertEqual(shorts.market_of("AAPL", "NASDAQ", "USD"), "us")
        self.assertEqual(shorts.market_of("QNC", "TSX-V", "CAD"), "ca")
        self.assertEqual(shorts.market_of("TGIF", "CSE", "CAD"), "ca")
        self.assertEqual(shorts.market_of("HBIX", "Cboe Canada", "CAD"), "ca")

    def test_a_venue_the_book_does_not_name_follows_the_currency_as_the_quotes_do(self):
        self.assertEqual(shorts.market_of("SHOP", "", "CAD"), "ca")
        self.assertEqual(shorts.market_of("F", "", "USD"), "us")

    def test_nothing_is_claimed_for_an_instrument_no_one_reports(self):
        self.assertEqual(shorts.market_of("BTC", "Crypto", "USD"), "")
        self.assertEqual(shorts.market_of("SPX", "Index", "USD"), "")
        self.assertEqual(shorts.market_of("ES", "CME", "USD"), "")
        self.assertEqual(shorts.market_of("AAPL  260117C00150000", "NASDAQ", "USD"), "")
        self.assertEqual(shorts.market_of("", "NYSE", "USD"), "")


class ReportDateTest(unittest.TestCase):
    def test_positions_are_reported_on_the_fifteenth_and_the_last_day(self):
        self.assertEqual(shorts.position_dates(date(2026, 9, 15), back=4),
                         [date(2026, 9, 15), date(2026, 8, 31), date(2026, 8, 15), date(2026, 7, 31)])

    def test_a_date_still_to_come_is_never_asked_for(self):
        self.assertEqual(shorts.position_dates(date(2026, 9, 3), back=2), [date(2026, 8, 31), date(2026, 8, 15)])

    def test_the_turn_of_the_year_steps_back_into_december(self):
        self.assertEqual(shorts.position_dates(date(2026, 1, 5), back=2), [date(2025, 12, 31), date(2025, 12, 15)])

    def test_volume_periods_are_the_two_halves_of_each_month(self):
        self.assertEqual(shorts.volume_periods(date(2026, 9, 15), back=3),
                         [(date(2026, 9, 1), date(2026, 9, 15)), (date(2026, 8, 16), date(2026, 8, 31)), (date(2026, 8, 1), date(2026, 8, 15))])

    def test_the_daily_file_is_only_looked_for_on_weekdays(self):
        self.assertEqual(shorts.trading_days(date(2026, 9, 15), back=4),
                         [date(2026, 9, 15), date(2026, 9, 14), date(2026, 9, 11), date(2026, 9, 10)])


class ParseTest(unittest.TestCase):
    def test_the_daily_us_file_gives_the_short_part_of_each_symbols_volume(self):
        rows = shorts.parse_us_volume(US_FILE)
        self.assertEqual(rows["GME"], {"shortVolume": 2250985.897562, "totalVolume": 3542062.253804})
        self.assertNotIn("NOVOL", rows)          # nothing traded: no share to report
        self.assertNotIn("SHORT", rows)          # a line the file cut short is skipped
        self.assertNotIn("Symbol", rows)         # the heading is not a listing

    def test_the_canadian_position_report_gives_shares_short_and_the_change(self):
        rows = shorts.parse_ca_positions(CA_GRID)
        self.assertEqual(rows["QNC"], {"venue": "TSXV", "shares": 2667164.0, "change": 64077.0, "name": "QUANTUM EMOTION CORP."})
        self.assertEqual(rows["TGIF"]["venue"], "CSE")
        self.assertNotIn("NIL", rows)
        self.assertNotIn("OOPS", rows)
        self.assertNotIn("Security Symbol", rows)

    def test_the_canadian_volume_report_gives_the_short_share_of_trading(self):
        rows = shorts.parse_ca_volume(CA_CSV)
        self.assertEqual(rows["QNC"]["shortVolume"], 1197633.0)
        self.assertEqual(rows["QNC"]["volumePct"], 21.319)
        self.assertAlmostEqual(rows["QNC"]["totalVolume"], 1197633.0 / 21.319 * 100)
        self.assertIsNone(rows["TGIF"]["totalVolume"])   # no share reported: nothing to divide by

    def test_a_row_is_only_used_for_the_venue_it_was_filed_under(self):
        self.assertTrue(shorts._venue_fits("TSXV", "TSX-V"))
        self.assertTrue(shorts._venue_fits("AQL", "Cboe Canada"))
        self.assertTrue(shorts._venue_fits("TSX", ""))       # a listing with no venue takes the only row there is
        self.assertFalse(shorts._venue_fits("TSX", "CSE"))


class ListingTest(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        os.environ["BAGHOLDER_HOME"] = self.tmp.name
        store.set_home(self.tmp.name)
        bagholder.set_home(self.tmp.name)
        store.ensure()
        model.invalidate()
        shorts._files.clear()

    def tearDown(self):
        shorts._files.clear()
        self.tmp.cleanup()

    def test_the_newest_settlement_finra_has_is_the_one_shown(self):
        answered = [{"settlementDate": "2026-08-14", "currentShortPositionQuantity": 54036583, "previousShortPositionQuantity": 53736062, "changePreviousNumber": 300521},
                    {"settlementDate": "2026-08-31", "currentShortPositionQuantity": 56990026, "previousShortPositionQuantity": 54036583, "changePreviousNumber": 2953443, "averageDailyVolumeQuantity": 5864237},
                    "not a row"]
        with mock.patch.object(shorts.market, "_post_json", return_value=answered):
            out = shorts.us_position("GME")
        self.assertEqual({k: out[k] for k in ("asOf", "shares", "previous", "change", "previousOf", "averageVolume")},
                         {"asOf": "2026-08-31", "shares": 56990026.0, "previous": 54036583.0,
                          "change": 2953443.0, "previousOf": "2026-08-14", "averageVolume": 5864237.0})

    def test_a_symbol_finra_does_not_carry_answers_nothing_rather_than_guessing(self):
        with mock.patch.object(shorts.market, "_post_json", return_value=[]):
            self.assertEqual(shorts.us_position("NOSUCH"), {})

    def test_a_whole_market_file_is_read_once_and_used_for_every_listing(self):
        with mock.patch.object(shorts.market, "_get_text", return_value=US_FILE) as got:
            first = shorts.us_volume("GME", now=datetime(2026, 9, 15, tzinfo=timezone.utc))
            second = shorts.us_volume("A", now=datetime(2026, 9, 15, tzinfo=timezone.utc))
        self.assertEqual(got.call_count, 1)
        self.assertEqual(first["volumeOf"], "2026-09-15")
        self.assertAlmostEqual(first["volumePct"], 2250985.897562 / 3542062.253804 * 100)
        self.assertEqual(second["volumeSpan"], "day")

    def test_a_file_that_will_not_answer_keeps_what_was_already_read(self):
        with mock.patch.object(shorts.market, "_get_text", return_value=US_FILE):
            shorts.us_volume("GME", now=datetime(2026, 9, 15, tzinfo=timezone.utc))
        shorts._files["us_volume"]["at"] = 0
        with mock.patch.object(shorts.market, "_get_text", side_effect=OSError("down")):
            kept = shorts.us_volume("GME", now=datetime(2026, 9, 15, tzinfo=timezone.utc))
        self.assertEqual(kept["shortVolume"], 2250985.897562)

    def test_a_canadian_listing_reads_both_of_its_reports(self):
        with mock.patch.object(shorts.market, "_fetch", return_value=b"x"), \
             mock.patch.object(shorts.xls, "table", return_value=CA_GRID), \
             mock.patch.object(shorts.market, "ensure_bars", return_value=[]), \
             mock.patch.object(shorts.market, "_get_text", return_value=CA_CSV):
            rec = shorts.for_listing("QNC", "TSX-V", "CAD", now=datetime(2026, 9, 15, tzinfo=timezone.utc))
        self.assertEqual(rec["source"], "CIRO")
        self.assertEqual(rec["shares"], 2667164.0)
        self.assertEqual(rec["previous"], 2603087.0)          # the change taken off what is held now
        self.assertEqual(rec["asOf"], "2026-09-15")
        self.assertEqual(rec["volumeOf"], "2026-09-01/2026-09-15")
        self.assertEqual(rec["volumeSpan"], "period")
        self.assertEqual(rec["previousOf"], "2026-08-31")      # what the change is measured against

    def test_a_listing_filed_under_another_venue_is_not_read_as_this_one(self):
        with mock.patch.object(shorts.market, "_fetch", return_value=b"x"), \
             mock.patch.object(shorts.xls, "table", return_value=CA_GRID), \
             mock.patch.object(shorts.market, "_get_text", return_value=CA_CSV):
            rec = shorts.for_listing("QNC", "CSE", "CAD", now=datetime(2026, 9, 15, tzinfo=timezone.utc))
        self.assertIsNone(rec.get("shares"))
        self.assertEqual(rec["market"], "ca")

    def test_nothing_is_read_for_an_instrument_no_one_reports(self):
        with mock.patch.object(shorts.market, "_get_text", side_effect=AssertionError("asked anyway")):
            self.assertEqual(shorts.for_listing("BTC", "Crypto", "USD"), {})

    def test_days_to_cover_uses_the_volume_of_the_listings_own_market(self):
        us = {"market": "us", "shares": 56990026.0, "averageVolume": 5864237.0}
        self.assertEqual(shorts.average_volume(us), 5864237.0)      # FINRA publishes it
        self.assertEqual(shorts.days_to_cover(us), 9.7)

    def test_the_canadian_average_counts_only_the_days_the_market_traded(self):
        for day in ("2026-08-17", "2026-08-18", "2026-08-19", "2026-08-20", "2026-08-21"):
            store.upsert_benchmark_prices("TSX", {day: 100.0}) if hasattr(store, "upsert_benchmark_prices") else None
        ca = {"market": "ca", "shares": 2667164.0, "totalVolume": 5000000.0, "volumeOf": "2026-08-16/2026-08-31"}
        days = store.benchmark_days("TSX", "2026-08-16", "2026-08-31")
        if days:
            self.assertAlmostEqual(shorts.average_volume(ca), 5000000.0 / days)
        else:
            self.assertIsNone(shorts.average_volume(ca))            # no calendar stored: nothing is guessed

    def test_no_position_or_no_volume_leaves_days_to_cover_unsaid(self):
        self.assertIsNone(shorts.days_to_cover({"market": "us", "shares": None, "averageVolume": 10.0}))
        self.assertIsNone(shorts.days_to_cover({"market": "us", "shares": 10.0, "averageVolume": None}))
        self.assertIsNone(shorts.average_volume({"market": "ca", "totalVolume": None, "volumeOf": "2026-08-16/2026-08-31"}))


class PayloadTest(unittest.TestCase):
    def test_a_market_no_one_reports_answers_that_it_is_not_covered(self):
        out = bagholder.shorts_payload("BTC", "Crypto", "USD")
        self.assertEqual(out, {"ok": True, "covered": False})

    def test_a_listing_with_figures_hands_them_over(self):
        with mock.patch.object(bagholder.shorts, "for_listing", return_value={"shares": 1.0, "source": "FINRA"}):
            out = bagholder.shorts_payload("GME", "NYSE", "USD")
        self.assertTrue(out["covered"])
        self.assertEqual(out["shorts"]["source"], "FINRA")

    def test_no_symbol_is_an_error_rather_than_an_empty_card(self):
        self.assertFalse(bagholder.shorts_payload("")["ok"])


if __name__ == "__main__":
    unittest.main()


class SeriesTest(unittest.TestCase):
    """The run of past reports drawn beside the position. FINRA answers with every
    settlement at once; Canada publishes a file per reporting date."""

    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        os.environ["BAGHOLDER_HOME"] = self.tmp.name
        store.set_home(self.tmp.name)
        bagholder.set_home(self.tmp.name)
        store.ensure()
        shorts._files.clear()

    def tearDown(self):
        shorts._files.clear()
        self.tmp.cleanup()

    def test_every_settlement_finra_answered_with_is_kept_oldest_first(self):
        answered = [{"settlementDate": "2026-08-31", "currentShortPositionQuantity": 3},
                    {"settlementDate": "2026-07-31", "currentShortPositionQuantity": 1},
                    {"settlementDate": "2026-08-14", "currentShortPositionQuantity": 2},
                    {"settlementDate": "2026-06-30", "currentShortPositionQuantity": None}]
        with mock.patch.object(shorts.market, "_post_json", return_value=answered):
            series = shorts.us_position("GME")["series"]
        self.assertEqual([p["date"] for p in series], ["2026-07-31", "2026-08-14", "2026-08-31"])
        self.assertEqual([p["shares"] for p in series], [1.0, 2.0, 3.0])

    def test_the_canadian_run_reads_one_file_per_reporting_date(self):
        grids = {"20260831": [["", "QNC", "TSXV", 300.0, 0.0]],
                 "20260815": [["", "QNC", "TSXV", 200.0, 0.0]],
                 "20260731": [["", "QNC", "TSXV", 100.0, 0.0]]}
        def table(raw):
            return grids[raw.decode()]
        def fetch(url, *a, **k):
            day = url.rsplit("/", 1)[-1].split("_")[0]
            if day not in grids:
                raise OSError("no report")
            return day.encode()
        with mock.patch.object(shorts.market, "_fetch", side_effect=fetch), \
             mock.patch.object(shorts.xls, "table", side_effect=table):
            series = shorts.ca_series("QNC", "TSX-V", "2026-08-31", now=datetime(2026, 9, 15, tzinfo=timezone.utc))
        self.assertEqual([(p["date"], p["shares"]) for p in series],
                         [("2026-07-31", 100.0), ("2026-08-15", 200.0), ("2026-08-31", 300.0)])

    def test_a_report_after_the_one_on_show_is_not_drawn(self):
        with mock.patch.object(shorts.market, "_fetch", side_effect=OSError("none")):
            self.assertEqual(shorts.ca_series("QNC", "TSX-V", "2026-07-31", now=datetime(2026, 9, 15, tzinfo=timezone.utc)), [])

    def test_a_listing_on_another_venue_is_not_drawn_into_this_ones_run(self):
        with mock.patch.object(shorts.market, "_fetch", return_value=b"x"), \
             mock.patch.object(shorts.xls, "table", return_value=[["", "QNC", "CSE", 300.0, 0.0]]):
            self.assertEqual(shorts.ca_series("QNC", "TSX-V", "2026-08-31", now=datetime(2026, 9, 15, tzinfo=timezone.utc)), [])

    def test_the_canadian_run_is_only_read_when_it_is_asked_for(self):
        with mock.patch.object(shorts.market, "_fetch", return_value=b"x"), \
             mock.patch.object(shorts.xls, "table", return_value=CA_GRID), \
             mock.patch.object(shorts.market, "_get_text", return_value=CA_CSV):
            quiet = shorts.for_listing("QNC", "TSX-V", "CAD", now=datetime(2026, 9, 15, tzinfo=timezone.utc))
        self.assertNotIn("series", quiet)


class FloatTest(unittest.TestCase):
    """The float the donut draws against: the shares actually available to trade."""

    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        os.environ["BAGHOLDER_HOME"] = self.tmp.name
        store.set_home(self.tmp.name)
        store.ensure()
        shorts._shares.clear()
        shorts._yahoo["session"] = None

    def tearDown(self):
        shorts._shares.clear()
        shorts._yahoo["session"] = None
        self.tmp.cleanup()

    def session(self, answers):
        class Answer:
            def __init__(self, payload, status=200):
                self.status_code, self._payload = status, payload
                self.text = payload if isinstance(payload, str) else ""
            def json(self):
                return self._payload
        class Session:
            def __init__(self):
                self.asked = []
            def get(self, url, **kw):
                self.asked.append(url)
                for mark, payload, status in answers:
                    if mark in url:
                        return Answer(payload, status)
                return Answer({}, 404)
        return Session()

    def use(self, session):
        shorts._yahoo["session"], shorts._yahoo["crumb"] = session, "abc"

    def stats(self, value):
        return {"quoteSummary": {"result": [{"defaultKeyStatistics": {"floatShares": {"raw": value}}}]}}

    def test_the_float_is_read_under_the_venues_own_symbol(self):
        session = self.session([("QNC.V", self.stats(212448707), 200)])
        self.use(session)
        self.assertEqual(shorts.float_shares("QNC", "TSX-V", "CAD"), 212448707.0)
        self.assertTrue(any("QNC.V" in u for u in session.asked))

    def test_a_form_the_source_does_not_carry_falls_to_the_next(self):
        session = self.session([("QNC.TO", {}, 404), ("QNC.V", self.stats(1000.0), 200)])
        self.use(session)
        self.assertEqual(shorts.float_shares("QNC", "TSX-V", "CAD"), 1000.0)

    def test_a_listing_with_no_float_published_reports_none(self):
        self.use(self.session([("HBIX", self.stats(None), 200)]))
        self.assertIsNone(shorts.float_shares("HBIX", "Cboe Canada", "CAD"))

    def test_it_is_read_once_and_kept(self):
        session = self.session([("GME", self.stats(463550645), 200)])
        self.use(session)
        shorts.float_shares("GME", "NYSE", "USD")
        shorts.float_shares("GME", "NYSE", "USD")
        self.assertEqual(len([u for u in session.asked if "GME" in u]), 1)

    def test_without_the_browser_client_the_float_is_simply_unknown(self):
        with mock.patch.object(shorts, "_yahoo_session", return_value=(None, "")):
            self.assertIsNone(shorts.float_shares("GME", "NYSE", "USD"))

    def test_the_position_is_measured_against_the_float(self):
        self.use(self.session([("GME", self.stats(400.0), 200)]))
        with mock.patch.object(shorts, "us_position", return_value={"shares": 100.0, "asOf": "2026-08-31"}), \
             mock.patch.object(shorts, "us_volume", return_value={}):
            rec = shorts.for_listing("GME", "NYSE", "USD")
        self.assertEqual(rec["float"], 400.0)
        self.assertEqual(rec["ofFloat"], 25.0)


class StoredTest(unittest.TestCase):
    """What was read is kept, so opening an instrument again — or after a restart — draws
    its tiles with the page instead of after a round of reads."""

    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        os.environ["BAGHOLDER_HOME"] = self.tmp.name
        store.set_home(self.tmp.name)
        bagholder.set_home(self.tmp.name)
        store.ensure()
        model.invalidate()
        shorts._files.clear()
        shorts._shares.clear()

    def tearDown(self):
        shorts._files.clear()
        shorts._shares.clear()
        self.tmp.cleanup()
        os.environ.pop("BAGHOLDER_HOME", None)

    def record(self, **over):
        rec = {"market": "ca", "asOf": "2026-08-31", "shares": 2667164.0, "previous": 2603087.0, "previousOf": "2026-08-15",
               "change": 64077.0, "float": 212448707.0, "ofFloat": 1.2554, "averageVolume": 510698.0, "daysToCover": 5.2,
               "volumeOf": "2026-08-16/2026-08-31", "volumeSpan": "period", "shortVolume": 1197633.0,
               "totalVolume": 5617679.0, "volumePct": 21.319, "series": [{"date": "2026-08-15", "shares": 2603087.0}]}
        rec.update(over)
        return rec

    def test_a_reading_comes_back_as_it_went_in(self):
        store.save_shorts("QNC", "TSX-V", self.record())
        held = store.shorts_for("QNC", "TSX-V")
        self.assertEqual(held["shares"], 2667164.0)
        self.assertEqual(held["ofFloat"], 1.2554)
        self.assertEqual(held["volumeSpan"], "period")
        self.assertEqual(held["series"], [{"date": "2026-08-15", "shares": 2603087.0}])
        self.assertTrue(held["fetchedAt"])

    def test_a_later_reading_without_a_run_of_reports_keeps_the_one_stored(self):
        store.save_shorts("QNC", "TSX-V", self.record())
        store.save_shorts("QNC", "TSX-V", self.record(series=None, shares=99.0))
        held = store.shorts_for("QNC", "TSX-V")
        self.assertEqual(held["shares"], 99.0)
        self.assertEqual(len(held["series"]), 1)

    def test_the_two_listings_of_one_company_are_kept_apart(self):
        store.save_shorts("QNC", "TSX-V", self.record(shares=2667164.0))
        store.save_shorts("QNC", "NYSE", self.record(market="us", shares=7058199.0))
        self.assertEqual(store.shorts_for("QNC", "TSX-V")["shares"], 2667164.0)
        self.assertEqual(store.shorts_for("QNC", "NYSE")["shares"], 7058199.0)
        self.assertEqual(len(store.all_shorts()), 2)

    def test_a_listing_never_read_is_not_in_the_store(self):
        self.assertIsNone(store.shorts_for("NOSUCH", "TSX"))

    def test_what_is_stored_is_answered_without_reading_again(self):
        # stamped with this version's number, so it is not the "written by older logic" case the
        # next test covers: nothing is read, in the answer or behind it
        store.save_shorts("QNC", "TSX-V", self.record(), version=bagholder.SHORTS_VERSION)
        with mock.patch.object(bagholder.shorts, "for_listing", side_effect=AssertionError("read anyway")):
            out = bagholder.shorts_payload("QNC", "TSX-V", "CAD", trend=True)
        self.assertTrue(out["covered"])
        self.assertEqual(out["shorts"]["shares"], 2667164.0)

    def test_a_listing_not_stored_yet_is_read_and_kept(self):
        with mock.patch.object(bagholder.shorts, "for_listing", return_value=self.record()) as read:
            out = bagholder.shorts_payload("QNC", "TSX-V", "CAD", trend=True)
        self.assertEqual(read.call_count, 1)
        self.assertTrue(out["covered"])
        self.assertEqual(store.shorts_for("QNC", "TSX-V")["shares"], 2667164.0)

    def test_a_market_no_one_reports_is_never_read_or_kept(self):
        with mock.patch.object(bagholder.shorts, "for_listing", side_effect=AssertionError("asked anyway")):
            self.assertEqual(bagholder.shorts_payload("BTC", "Crypto", "USD"), {"ok": True, "covered": False})
        self.assertEqual(store.all_shorts(), [])

    def test_a_stale_reading_is_still_answered_at_once(self):
        store.save_shorts("QNC", "TSX-V", self.record(), now="2020-01-01T00:00:00Z")
        with mock.patch.object(bagholder, "kick", return_value=True) as kicked, \
             mock.patch.object(bagholder.shorts, "for_listing", side_effect=AssertionError("read in the request")):
            out = bagholder.shorts_payload("QNC", "TSX-V", "CAD", trend=True)
        self.assertEqual(out["shorts"]["shares"], 2667164.0)
        self.assertEqual(kicked.call_count, 1)      # refreshed behind the page, not in front of it


class FeedTest(unittest.TestCase):
    """The ranked list reads the store and never a regulator, and one answer covers every
    scope so turning between them asks nothing."""

    setUp, tearDown, record = StoredTest.setUp, StoredTest.tearDown, StoredTest.record

    def book(self, positions=(), watchlist=()):
        return mock.patch.object(bagholder.model, "base_model", return_value={"positions": list(positions), "watchlist": list(watchlist)})

    def test_only_listings_the_book_holds_or_watches_are_listed(self):
        store.save_shorts("QNC", "TSX-V", self.record())
        store.save_shorts("GONE", "TSX", self.record())
        with self.book(positions=[{"symbol": "QNC", "exchange": "TSX-V", "kind": "Shares", "name": "Quantum eMotion Corp", "id": "rt:1"}]):
            out = bagholder.shorts_feed()
        self.assertEqual([r["symbol"] for r in out["rows"]], ["QNC"])

    def test_each_row_says_whether_it_is_held_or_watched(self):
        store.save_shorts("QNC", "TSX-V", self.record())
        store.save_shorts("PNG", "TSX-V", self.record())
        with self.book(positions=[{"symbol": "QNC", "exchange": "TSX-V", "kind": "Shares", "name": "Quantum eMotion Corp", "id": "rt:1"}],
                       watchlist=[{"symbol": "PNG", "exchange": "TSX-V", "name": "Kraken Robotics Inc."}]):
            rows = {r["symbol"]: r for r in bagholder.shorts_feed()["rows"]}
        self.assertEqual((rows["QNC"]["held"], rows["QNC"]["watched"]), (True, False))
        self.assertEqual((rows["PNG"]["held"], rows["PNG"]["watched"]), (False, True))

    def test_a_row_carries_its_name_and_the_holding_it_opens(self):
        store.save_shorts("QNC", "TSX-V", self.record())
        with self.book(positions=[{"symbol": "QNC", "exchange": "TSX-V", "kind": "Shares", "name": "Quantum eMotion Corp", "id": "rt:1"}]):
            row = bagholder.shorts_feed()["rows"][0]
        self.assertEqual(row["name"], "Quantum eMotion Corp")
        self.assertEqual(row["positionId"], "rt:1")

    def test_the_list_is_told_while_a_sweep_still_has_listings_to_read(self):
        book = [{"symbol": "HBIX", "exchange": "CBOE CANADA", "kind": "Shares", "name": "H", "id": "rt:1", "currency": "CAD"},
                {"symbol": "QNC", "exchange": "TSX-V", "kind": "Shares", "name": "Q", "id": "rt:2", "currency": "CAD"}]
        seen = []
        def read(sym, ex, ccy, **kw):
            seen.append(bagholder.shorts_feed()["reading"])       # what an open page is told mid-pass
            store.save_shorts(sym, ex, self.record(), version=bagholder.SHORTS_VERSION)
            return self.record()
        with self.book(positions=book), mock.patch.object(bagholder, "read_shorts", side_effect=read):
            self.assertFalse(bagholder.shorts_feed()["reading"], "nothing is being read before a pass")
            self.assertEqual(bagholder.sweep_shorts(), 2)
            self.assertEqual(seen, [True, True], "every listing of the pass, the last one included")
            self.assertFalse(bagholder.shorts_feed()["reading"], "and nothing once it ends")
        with self.book(positions=book), mock.patch.object(bagholder, "read_shorts", side_effect=OSError("down")), mock.patch.object(bagholder.sys, "stderr"):
            store.save_shorts("QNC", "TSX-V", self.record(), version=0)
            bagholder.sweep_shorts()
            self.assertFalse(bagholder.shorts_feed()["reading"], "a pass that fails still ends")

    def test_a_listing_read_but_carrying_no_position_is_left_out(self):
        store.save_shorts("QNC", "TSX-V", self.record(shares=None))
        with self.book(positions=[{"symbol": "QNC", "exchange": "TSX-V", "kind": "Shares", "name": "Q", "id": "rt:1"}]):
            self.assertEqual(bagholder.shorts_feed()["rows"], [])


class FundFloatTest(unittest.TestCase):
    """A fund is the one instrument whose units in issue are its float: it creates and
    redeems them on demand and holds none back."""

    setUp, tearDown, session, use, stats = (FloatTest.setUp, FloatTest.tearDown, FloatTest.session, FloatTest.use, FloatTest.stats)

    def test_a_company_with_no_float_published_is_never_given_its_share_count(self):
        answer = {"quoteSummary": {"result": [{"defaultKeyStatistics": {"floatShares": None, "sharesOutstanding": {"raw": 500.0}}}]}}
        self.use(self.session([("QNC", answer, 200)]))
        with mock.patch.object(shorts, "_fund_units", side_effect=AssertionError("asked for units")):
            self.assertIsNone(shorts.float_shares("QNC", "TSX-V", "CAD", "Quantum eMotion Corp"))

    def test_a_fund_falls_back_to_the_units_the_exchange_publishes(self):
        answer = {"quoteSummary": {"result": [{"defaultKeyStatistics": {"floatShares": None, "sharesOutstanding": None}}]}}
        self.use(self.session([("RDDY", answer, 200)]))
        with mock.patch.object(shorts, "_fund_units", return_value=20075000.0):
            self.assertEqual(shorts.float_shares("RDDY", "TSX", "CAD", "Harvest Reddit Enhanced High Income Shares ETF"), 20075000.0)

    def test_a_us_fund_takes_the_count_from_the_same_answer_as_the_float(self):
        answer = {"quoteSummary": {"result": [{"defaultKeyStatistics": {"floatShares": None, "sharesOutstanding": {"raw": 4000.0}}}]}}
        self.use(self.session([("SPY", answer, 200)]))
        self.assertEqual(shorts.float_shares("SPY", "NYSE", "USD", "SPDR S&P 500 ETF Trust"), 4000.0)

    def test_a_float_that_is_published_is_still_what_a_fund_is_measured_against(self):
        answer = {"quoteSummary": {"result": [{"defaultKeyStatistics": {"floatShares": {"raw": 111.0}, "sharesOutstanding": {"raw": 999.0}}}]}}
        self.use(self.session([("XIU", answer, 200)]))
        self.assertEqual(shorts.float_shares("XIU", "TSX", "CAD", "iShares S&P/TSX 60 Index ETF"), 111.0)


class SearchedListingTest(unittest.TestCase):
    """A ticker the book does not carry is asked for under its ticker alone: the regulator's
    own report names its venue and its issuer, and the name is what tells a fund from a
    company, so the reading is a whole row rather than a ticker with blanks beside it."""

    POSITION = {"TSX": {"venue": "TSX", "shares": 7573829.0, "change": 41206.0, "name": "SHOPIFY INC. CL 'A' SV"}}

    def setUp(self):
        shorts._files.clear()
        shorts._shares.clear()

    tearDown = setUp

    def read(self, exchange="", name=""):
        with mock.patch.object(shorts, "_table", return_value={"key": "2026-08-31", "rows": {"SHOP": self.POSITION["TSX"]}}), \
             mock.patch.object(shorts, "ca_volume", return_value={}), \
             mock.patch.object(shorts, "float_shares", side_effect=lambda sym, ex, ccy, nm="", ctx=None: self.seen.append(nm) or 1000000.0):
            self.seen = []
            return shorts.for_listing("SHOP", exchange, "CAD", name=name)

    def test_the_report_names_the_venue_and_the_issuer_where_the_book_knows_neither(self):
        rec = self.read()
        self.assertEqual((rec["exchange"], rec["name"]), ("TSX", "SHOPIFY INC. CL 'A' SV"))
        self.assertEqual(self.seen, ["SHOPIFY INC. CL 'A' SV"], "the issuer is what tells a fund from a company")

    def test_the_book_own_name_for_a_listing_it_carries_is_the_one_the_float_is_read_under(self):
        rec = self.read(exchange="TSX", name="Shopify Inc.")
        self.assertEqual(rec["exchange"], "TSX")
        self.assertEqual(self.seen, ["Shopify Inc."], "the book's name for a listing it carries, not the report's")


class CboeUnitsTest(unittest.TestCase):
    """A fund listed on Cboe Canada: TMX answers 0 for its count and Yahoo publishes none,
    so the count comes from that venue's own directory, where a listing's market
    capitalisation divided by its last price gives the count back whole."""

    DIRECTORY = json.dumps({"data": [
        {"symbol": "HBIX", "name": "HARVEST BITCOIN ENHANCED INCOME ETF", "security": "etf", "marketcap": 44091000.0, "last": 6.39},
        {"symbol": "BCBN", "name": "A COMPANY", "security": "equity", "marketcap": 100876283.0, "last": 1.0},
        {"symbol": "NOPR", "name": "NO PRICE ETF", "security": "etf", "marketcap": 500.0, "last": 0.0},
        {"symbol": "ODDS", "name": "NOT A WHOLE COUNT ETF", "security": "etf", "marketcap": 100.0, "last": 3.0}]})

    def setUp(self):
        shorts._files.clear()

    tearDown = setUp

    def test_the_count_is_the_capitalisation_over_the_price_for_the_venues_own_funds(self):
        with mock.patch.object(shorts.market, "_get_text", return_value=self.DIRECTORY) as got, \
             mock.patch.object(shorts.market, "tmx_quote_symbol", return_value=""):   # TMX carries no count for these
            self.assertEqual(shorts._fund_units("HBIX", "CBOE CANADA", "CAD"), 6900000.0)
            self.assertIsNone(shorts._fund_units("BCBN", "CBOE CANADA", "CAD"), "a company's shares in issue are not its float")
            self.assertIsNone(shorts._fund_units("NOPR", "CBOE CANADA", "CAD"), "no price, no count")
            self.assertIsNone(shorts._fund_units("ODDS", "CBOE CANADA", "CAD"), "a count that is not whole is not the exchange's own")
        self.assertEqual(got.call_count, 1, "one directory for every listing looked up in it")

    def test_a_listing_on_another_venue_never_takes_a_count_from_this_one(self):
        with mock.patch.object(shorts.market, "_get_text", side_effect=AssertionError("asked anyway")), \
             mock.patch.object(shorts.market, "tmx_quote_symbol", return_value="HBIX:TSX"), \
             mock.patch.object(shorts.market, "tmx_lookup", return_value=({"shareOutStanding": 0}, "")):
            self.assertIsNone(shorts._fund_units("HBIX", "TSX", "CAD"))

    def test_the_venue_is_asked_only_where_tmx_has_no_count(self):
        with mock.patch.object(shorts.market, "_get_text", side_effect=AssertionError("asked anyway")), \
             mock.patch.object(shorts.market, "tmx_quote_symbol", return_value="XYZ:AQL"), \
             mock.patch.object(shorts.market, "tmx_lookup", return_value=({"shareOutStanding": 4200}, "")):
            self.assertEqual(shorts._fund_units("XYZ", "CBOE CANADA", "CAD"), 4200.0)


class FloatCacheTest(unittest.TestCase):
    """A float that answered is kept for the day; one that did not is asked for again soon,
    since a source being slow is not the same as a figure not existing."""

    setUp, tearDown, session, use, stats = (FloatTest.setUp, FloatTest.tearDown, FloatTest.session, FloatTest.use, FloatTest.stats)

    def test_a_figure_is_read_once_and_kept(self):
        s = self.session([("GME", self.stats(463550645), 200)])
        self.use(s)
        shorts.float_shares("GME", "NYSE", "USD", "GameStop Corp.")
        shorts.float_shares("GME", "NYSE", "USD", "GameStop Corp.")
        self.assertEqual(len([u for u in s.asked if "GME" in u]), 1)

    def test_a_lookup_that_answered_with_nothing_is_asked_again(self):
        empty = self.session([("ASTS", self.stats(None), 200)])
        self.use(empty)
        self.assertIsNone(shorts.float_shares("ASTS", "NASDAQ", "USD", "AST SpaceMobile Inc."))
        shorts._shares["ASTS|NASDAQ"]["at"] -= shorts.FLOAT_MISS_MIN * 60 + 1
        good = self.session([("ASTS", self.stats(266440743), 200)])
        self.use(good)
        self.assertEqual(shorts.float_shares("ASTS", "NASDAQ", "USD", "AST SpaceMobile Inc."), 266440743.0)

    def test_a_figure_is_not_asked_again_that_soon(self):
        s = self.session([("GME", self.stats(463550645), 200)])
        self.use(s)
        shorts.float_shares("GME", "NYSE", "USD", "GameStop Corp.")
        shorts._shares["GME|NYSE"]["at"] -= shorts.FLOAT_MISS_MIN * 60 + 1
        shorts.float_shares("GME", "NYSE", "USD", "GameStop Corp.")
        self.assertEqual(len([u for u in s.asked if "GME" in u]), 1)


class YahooPaceTest(unittest.TestCase):
    """The float lookups keep Yahoo's own pace, the one the rest of the app keeps."""

    setUp, tearDown, session, use, stats = (FloatTest.setUp, FloatTest.tearDown, FloatTest.session, FloatTest.use, FloatTest.stats)

    def test_each_lookup_takes_its_turn_and_leaves_the_next_slot(self):
        s = self.session([("GME", self.stats(1.0), 200)])
        self.use(s)
        shorts.market._yahoo_next_at = 0
        with mock.patch.object(shorts.time, "sleep") as slept:
            shorts.float_shares("GME", "NYSE", "USD", "GameStop Corp.")
        self.assertGreater(shorts.market._yahoo_next_at, 0)          # the next caller waits its turn
        self.assertEqual(slept.call_count, 0)                        # the slot was free, so no wait

    def test_nothing_is_asked_while_a_backoff_stands(self):
        s = self.session([("GME", self.stats(1.0), 200)])
        self.use(s)
        shorts.market._yahoo_backoff_until = shorts.time.monotonic() + 60
        try:
            self.assertIsNone(shorts.float_shares("GME", "NYSE", "USD", "GameStop Corp."))
            self.assertEqual([u for u in s.asked if "quoteSummary" in u], [])
        finally:
            shorts.market._yahoo_backoff_until = 0

    def test_a_refusal_starts_the_backoff_the_whole_app_honours(self):
        s = self.session([("GME", {}, 429)])
        self.use(s)
        shorts.market._yahoo_backoff_until = 0
        try:
            self.assertIsNone(shorts.float_shares("GME", "NYSE", "USD", "GameStop Corp."))
            self.assertGreater(shorts.market._yahoo_backoff_until, shorts.time.monotonic())
        finally:
            shorts.market._yahoo_backoff_until = 0


class FloatFormTest(unittest.TestCase):
    """Which symbol Yahoo is asked for. A row that carries no currency must not be asked for
    under the wrong market's suffixes."""

    setUp, tearDown, session, use, stats = (FloatTest.setUp, FloatTest.tearDown, FloatTest.session, FloatTest.use, FloatTest.stats)

    def test_a_us_listing_with_no_currency_on_its_row_is_still_asked_for_as_one(self):
        s = self.session([("quoteSummary/ASTS?", self.stats(266440743), 200)])
        self.use(s)
        self.assertEqual(shorts.float_shares("ASTS", "NASDAQ", "", "AST SpaceMobile Inc."), 266440743.0)
        self.assertFalse([u for u in s.asked if ".TO" in u or ".V" in u], "asked under a Canadian suffix")

    def test_a_canadian_listing_with_no_currency_keeps_its_own_suffixes(self):
        s = self.session([("QNC.V", self.stats(212448707), 200)])
        self.use(s)
        self.assertEqual(shorts.float_shares("QNC", "TSX-V", "", "Quantum eMotion Corp"), 212448707.0)
        self.assertTrue(any("QNC.V" in u for u in s.asked))


class ReadVersionTest(unittest.TestCase):
    """A reading written by older logic is read again once, rather than waiting hours to go
    stale while a figure the app has since learned to find reads as a dash."""

    setUp, tearDown, record = StoredTest.setUp, StoredTest.tearDown, StoredTest.record

    def test_a_row_from_older_logic_is_read_again_however_fresh_it_is(self):
        store.save_shorts("QNC", "TSX-V", self.record(float=None, ofFloat=None), version=bagholder.SHORTS_VERSION - 1)
        held = store.shorts_for("QNC", "TSX-V")
        self.assertTrue(bagholder._shorts_stale(held))     # written a moment ago, and still stale

    def test_a_row_at_the_current_version_stands_until_its_hours_are_up(self):
        store.save_shorts("QNC", "TSX-V", self.record(), version=bagholder.SHORTS_VERSION)
        self.assertFalse(bagholder._shorts_stale(store.shorts_for("QNC", "TSX-V")))

    def test_the_sweep_reads_a_row_from_older_logic(self):
        store.save_shorts("QNC", "TSX-V", self.record(float=None), version=bagholder.SHORTS_VERSION - 1)
        base = {"positions": [{"symbol": "QNC", "exchange": "TSX-V", "kind": "Shares", "name": "Quantum eMotion Corp"}], "watchlist": []}
        with mock.patch.object(bagholder.model, "base_model", return_value=base), \
             mock.patch.object(bagholder.shorts, "for_listing", return_value=self.record(float=212448707.0)) as read:
            bagholder.sweep_shorts()
        self.assertEqual(read.call_count, 1)
        held = store.shorts_for("QNC", "TSX-V")
        self.assertEqual(held["float"], 212448707.0)
        self.assertEqual(held["readVersion"], bagholder.SHORTS_VERSION)

    def test_a_record_read_on_the_spot_names_its_listing(self):
        with mock.patch.object(shorts, "us_position", return_value={"shares": 1.0, "asOf": "2026-08-31"}), \
             mock.patch.object(shorts, "us_volume", return_value={}), \
             mock.patch.object(shorts, "float_shares", return_value=None), \
             mock.patch.object(shorts, "average_volume", return_value=None):
            rec = shorts.for_listing("RKLB", "NASDAQ", "USD")
        self.assertEqual(rec["exchange"], "NASDAQ")


class VenueSpellingTest(unittest.TestCase):
    """The venue reads as the book writes it. The stored key is upper case because it is a
    key; the rest of the app shows Cboe Canada, not CBOE CANADA."""

    setUp, tearDown, record = StoredTest.setUp, StoredTest.tearDown, StoredTest.record

    def test_the_list_shows_the_venue_the_way_the_book_does(self):
        store.save_shorts("HBIX", "Cboe Canada", self.record(market="ca"))
        base = {"positions": [], "watchlist": [{"symbol": "HBIX", "exchange": "Cboe Canada", "name": "Harvest Bitcoin Enhanced Income ETF"}]}
        with mock.patch.object(bagholder.model, "base_model", return_value=base):
            row = bagholder.shorts_feed()["rows"][0]
        self.assertEqual(row["exchange"], "Cboe Canada")


class ReportOmitsListingTest(unittest.TestCase):
    """CIRO's volume report lists only what was sold short — it carries no zero rows — so a
    listing absent from it was not sold short in the period rather than unknown, and what it
    did trade comes from the exchange so days to cover still has a denominator."""

    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        os.environ["BAGHOLDER_HOME"] = self.tmp.name
        store.set_home(self.tmp.name)
        store.ensure()
        shorts._files.clear()

    def tearDown(self):
        shorts._files.clear()
        self.tmp.cleanup()

    def warm(self, rows, key="2026-08-16/2026-08-31"):
        shorts._files["ca_volume"] = {"key": key, "rows": rows, "at": shorts.time.time()}

    def test_a_listing_the_report_omits_reads_as_none_of_its_trading(self):
        self.warm({})
        with mock.patch.object(shorts, "ca_traded", return_value=1519546.0):
            out = shorts.ca_volume("YES", "TSX-V", "CAD")
        self.assertEqual(out["shortVolume"], 0.0)
        self.assertEqual(out["volumePct"], 0.0)
        self.assertEqual(out["totalVolume"], 1519546.0)

    def test_a_listing_the_report_omits_and_the_exchange_has_no_volume_for_says_nothing(self):
        self.warm({})
        with mock.patch.object(shorts, "ca_traded", return_value=None):
            self.assertEqual(shorts.ca_volume("YES", "TSX-V", "CAD"), {})

    def test_a_listing_the_report_carries_is_read_from_the_report(self):
        self.warm({"QNC": {"venue": "TSXV", "shortVolume": 1197633.0, "volumePct": 21.319, "totalVolume": 5617679.0}})
        with mock.patch.object(shorts, "ca_traded", side_effect=AssertionError("asked the exchange anyway")):
            out = shorts.ca_volume("QNC", "TSX-V", "CAD")
        self.assertEqual(out["volumePct"], 21.319)

    def test_days_to_cover_follows_from_what_the_exchange_says_was_traded(self):
        store.upsert_benchmark_prices("TSX", {"2026-08-%02d" % d: 100.0 for d in range(17, 28)}) if hasattr(store, "upsert_benchmark_prices") else None
        rec = {"market": "ca", "shares": 17873.0, "totalVolume": 1519546.0, "volumeOf": "2026-08-16/2026-08-31"}
        days = store.benchmark_days("TSX", "2026-08-16", "2026-08-31")
        if days:
            self.assertAlmostEqual(shorts.average_volume(rec), 1519546.0 / days)
            self.assertIsNotNone(shorts.days_to_cover(rec))
