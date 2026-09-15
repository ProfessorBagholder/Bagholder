"""Short selling: which regulator answers for a listing, what each report says, and the
one figure the app derives from them."""
from __future__ import annotations

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
                    {"settlementDate": "2026-08-31", "currentShortPositionQuantity": 56990026, "previousShortPositionQuantity": 54036583, "changePreviousNumber": 2953443},
                    "not a row"]
        with mock.patch.object(shorts.market, "_post_json", return_value=answered):
            self.assertEqual(shorts.us_position("GME"), {"asOf": "2026-08-31", "shares": 56990026.0, "previous": 54036583.0, "change": 2953443.0})

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

    def test_days_to_cover_divides_the_position_by_the_apps_own_average_volume(self):
        bars = [{"date": "2026-09-%02d" % d, "close": 10.0, "volume": 1000000.0} for d in range(1, 15)]
        store.upsert_price_history("GME", bars, "test")
        self.assertEqual(shorts.days_to_cover("GME", 5000000.0, now=datetime(2026, 9, 15, tzinfo=timezone.utc)), 5.0)

    def test_only_the_most_recent_sessions_count_toward_the_average(self):
        bars = [{"date": "2026-08-%02d" % d, "close": 10.0, "volume": 1.0} for d in range(1, 29)]
        bars += [{"date": "2026-09-%02d" % d, "close": 10.0, "volume": 1000000.0} for d in range(1, 15)]
        store.upsert_price_history("OLD", bars, "test")
        self.assertIsNotNone(shorts.COVER_SESSIONS)
        self.assertGreater(shorts.days_to_cover("OLD", 1000000.0, now=datetime(2026, 9, 15, tzinfo=timezone.utc)), 0.9)

    def test_no_position_and_no_volume_leave_days_to_cover_unsaid(self):
        self.assertIsNone(shorts.days_to_cover("GME", None))
        with mock.patch.object(shorts.market, "ensure_bars", return_value=[]):
            self.assertIsNone(shorts.days_to_cover("NEVERSEEN", 1000.0, "NASDAQ", "USD"))

    def test_a_listing_with_no_bars_stored_gets_them_the_way_its_chart_would(self):
        with mock.patch.object(shorts.market, "ensure_bars", return_value=[{"volume": 2000.0}, {"volume": 2000.0}]) as bars:
            self.assertEqual(shorts.days_to_cover("NEW", 4000.0, "NASDAQ", "USD"), 2.0)
        self.assertEqual(bars.call_count, 1)


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
