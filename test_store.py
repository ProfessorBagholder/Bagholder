#!/usr/bin/env python3
"""SQLite store and Wealthsimple sync bounds. Synthetic rows only."""

from __future__ import annotations

import gzip
import json
import os
import sqlite3
import tempfile
from pathlib import Path
import unittest
from datetime import datetime, timezone
from zoneinfo import ZoneInfo
from unittest import mock

import bagholder
import store

# Fake Wealthsimple production clientId for scrape tests. Not a real id.
FAKE_CLIENT_ID = "ab" * 32


class _FakeHTTPResp:
    def __init__(self, body, headers=None, status=200):
        if isinstance(body, (bytes, bytearray)):
            self._body = bytes(body)
        else:
            self._body = str(body).encode("utf-8")
        self.headers = headers or {}
        self.status = status

    def read(self):
        return self._body

    def __enter__(self):
        return self

    def __exit__(self, *exc):
        return False


def _ws_item(**overrides):
    item = {
        "occurredAt": "2024-06-15T13:45:22.123Z",
        "canonicalId": "ws-cid-aaa-001",
        "status": "POSTED",
        "type": "DIY_BUY",
        "subType": "BUY",
        "assetSymbol": "AAA",
        "assetQuantity": 10,
        "amount": 100,
        "accountId": "acct-1",
        "currency": "CAD",
    }
    item.update(overrides)
    return item


class StoreTest(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.home = self.tmp.name
        os.environ["BAGHOLDER_HOME"] = self.home
        store.set_home(self.home)
        bagholder.set_home(self.home)
        store.ensure()
        with bagholder._lock:
            bagholder._state["connected"] = False
            bagholder._state["error"] = ""
            bagholder._state["syncing"] = False
            bagholder._state["capturing"] = False
            bagholder._state["email"] = ""

    def tearDown(self):
        self.tmp.cleanup()
        os.environ.pop("BAGHOLDER_HOME", None)

    def test_options_sell_maps_as_sell_to_open(self):
        item = _ws_item(
            type="OPTIONS_SELL",
            subType="LIMIT_ORDER",
            assetSymbol="QNC",
            contractType="CALL",
            strikePrice=3,
            expiryDate="2027-02-19",
            assetQuantity=35,
            amount=1050,
            amountSign="positive",
        )
        row = bagholder.map_activity(item)
        self.assertEqual(row["activitySubType"], "SELLTOOPEN")
        self.assertEqual(row["category"], "trade")
        self.assertEqual(row["quantity"], -35)
        self.assertEqual(row["netCashAmount"], 1050)
        self.assertEqual(row["symbol"], "QNC 19FEB27 3.00 CALL")

    def test_options_buy_maps_as_buy_to_open(self):
        item = _ws_item(
            type="OPTIONS_BUY",
            subType="LIMIT_ORDER",
            assetSymbol="QNC",
            contractType="CALL",
            strikePrice=3,
            expiryDate="2027-02-19",
            assetQuantity=5,
            amount=150,
            amountSign="negative",
        )
        row = bagholder.map_activity(item)
        self.assertEqual(row["activitySubType"], "BUYTOOPEN")
        self.assertEqual(row["category"], "trade")
        self.assertEqual(row["quantity"], 5)
        self.assertEqual(row["netCashAmount"], -150)

    def test_relabel_stored_options_sell(self):
        store.apply_wealthsimple_mapped(
            [
                {
                    "canonicalId": "opt-sell-1",
                    "occurredAt": "2026-08-31T14:16:58Z",
                    "transactionDate": "2026-08-31",
                    "accountId": "acct-1",
                    "accountType": "Trading",
                    "activityType": "OPTIONS_SELL",
                    "activitySubType": "LIMIT_ORDER",
                    "symbol": "QNC 19FEB27 3.00 CALL",
                    "currency": "USD",
                    "quantity": 35,
                    "unitPrice": 0.3,
                    "netCashAmount": 1050,
                    "category": "other",
                    "source": "wealthsimple",
                    "rawType": "OPTIONS_SELL",
                }
            ]
        )
        store.ensure()
        snap = store.snapshot()
        row = [a for a in snap["activities"] if a.get("canonicalId") == "opt-sell-1"][0]
        self.assertEqual(row["activitySubType"], "SELLTOOPEN")
        self.assertEqual(row["category"], "trade")
        self.assertEqual(row["quantity"], -35)
        self.assertEqual(row["netCashAmount"], 1050)

    def test_insert_if_new_by_canonical_id(self):
        row = bagholder.map_activity(_ws_item())
        first = store.apply_wealthsimple_mapped([row])
        self.assertEqual(first["inserted"], 1)
        self.assertEqual(store.activity_count(), 1)
        again = store.apply_wealthsimple_mapped([row])
        self.assertEqual(again["inserted"], 0)
        self.assertEqual(again["skipped"], 1)
        self.assertEqual(store.activity_count(), 1)

    def test_second_sync_does_not_replace_row(self):
        original = bagholder.map_activity(_ws_item())
        store.apply_wealthsimple_mapped([original])
        changed = bagholder.map_activity(_ws_item(amount=999, assetQuantity=10))
        changed["description"] = "should not land"
        store.apply_wealthsimple_mapped([changed])
        stored = store.snapshot()["activities"]
        self.assertEqual(len(stored), 1)
        self.assertEqual(stored[0]["canonicalId"], "ws-cid-aaa-001")
        self.assertNotEqual(stored[0]["description"], "should not land")
        self.assertAlmostEqual(stored[0]["netCashAmount"], -100.0)

    def test_manual_has_no_canonical_id(self):
        result = bagholder.append_manual(
            {
                "date": "2024-07-01",
                "symbol": "ZZZ",
                "side": "BUY",
                "qty": 3,
                "price": 12.5,
                "currency": "CAD",
                "accountId": "manual",
            }
        )
        act = result["activity"]
        self.assertTrue(act["id"])
        self.assertFalse(store.looks_like_homemade_id(act["id"]))
        self.assertIn(act.get("canonicalId"), (None, "", False))
        stored = store.snapshot()["activities"][0]
        self.assertIsNone(stored.get("canonicalId"))
        self.assertEqual(stored["source"], "manual")

    def test_csv_has_no_canonical_id(self):
        saved = store.insert_local(
            {
                "transactionDate": "2024-07-02",
                "occurredAt": "2024-07-02",
                "accountId": "acct-1",
                "symbol": "ZZZ",
                "quantity": 4,
                "unitPrice": 8,
                "netCashAmount": -32,
                "activityType": "Trade",
                "activitySubType": "BUY",
                "source": "csv",
                "canonicalId": "do-not-keep-this",
            }
        )
        self.assertIsNone(saved.get("canonicalId"))
        self.assertEqual(saved["source"], "csv")
        self.assertFalse(store.looks_like_homemade_id(saved["id"]))

    def test_occurred_at_not_cut_to_date_for_wealthsimple_row(self):
        row = bagholder.map_activity(_ws_item())
        self.assertEqual(row["occurredAt"], "2024-06-15T13:45:22.123Z")
        self.assertEqual(row["transactionDate"], "2024-06-15")
        self.assertNotIn("id", row)
        store.apply_wealthsimple_mapped([row])
        stored = store.snapshot()["activities"][0]
        self.assertEqual(stored["occurredAt"], "2024-06-15T13:45:22.123Z")
        self.assertTrue("T" in stored["occurredAt"])

    def test_activity_pull_due_weekdays_at_2pm_mountain(self):
        mt = ZoneInfo("America/Edmonton")
        monday_1359 = datetime(2026, 8, 31, 13, 59, tzinfo=mt)
        monday_1400 = datetime(2026, 8, 31, 14, 0, tzinfo=mt)
        monday_1500 = datetime(2026, 8, 31, 15, 0, tzinfo=mt)
        saturday = datetime(2026, 8, 29, 15, 0, tzinfo=mt)
        self.assertFalse(store.activity_pull_due(now=monday_1359))
        self.assertTrue(store.activity_pull_due(now=monday_1400))
        self.assertTrue(store.activity_pull_due(now=monday_1500))
        self.assertFalse(store.activity_pull_due(now=saturday))
        store.mark_activity_pulled("2026-08-31T20:05:00Z")
        self.assertFalse(store.activity_pull_due(now=monday_1500))
        tuesday_1400 = datetime(2026, 9, 1, 14, 0, tzinfo=mt)
        self.assertTrue(store.activity_pull_due(now=tuesday_1400))

    def test_daily_path_does_not_page_whole_history_when_rows_exist(self):
        store.apply_wealthsimple_mapped([bagholder.map_activity(_ws_item())])
        bounds = bagholder.activity_sync_bounds()
        self.assertFalse(bounds["full_history"])
        self.assertTrue(bounds["start_date"])
        self.assertEqual(bounds["start_date"][:10], "2024-06-15")

        calls = []

        def fake_graphql(sess, operation, variables):
            calls.append(variables)
            return {
                "activityFeedItems": {
                    "edges": [
                        {
                            "node": _ws_item(
                                canonicalId="ws-cid-aaa-001",
                                occurredAt="2024-06-15T13:45:22.123Z",
                            )
                        }
                    ],
                    "pageInfo": {"hasNextPage": True, "endCursor": "cursor-keep-going"},
                }
            }

        with mock.patch.object(bagholder, "graphql", side_effect=fake_graphql):
            bagholder.fetch_activities_for_account(
                {"access_token": "x"},
                "acct-1",
                start_date=bounds["start_date"],
                known_canonical_ids=store.canonical_ids(),
            )

        self.assertEqual(len(calls), 1)
        cond = calls[0]["condition"]
        self.assertIn("startDate", cond)
        self.assertTrue(str(cond["startDate"]).startswith("2024-06-15"))

    def test_empty_table_full_history_omits_start_date(self):
        self.assertEqual(store.activity_count(), 0)
        bounds = bagholder.activity_sync_bounds()
        self.assertTrue(bounds["full_history"])
        self.assertIsNone(bounds["start_date"])
        cond = bagholder.activity_fetch_condition("acct-1", start_date=bounds["start_date"])
        self.assertNotIn("startDate", cond)

    def test_existing_rows_make_daily_sync_incremental(self):
        store.apply_wealthsimple_mapped([bagholder.map_activity(_ws_item())])
        store.insert_local(
            {
                "transactionDate": "2024-07-01",
                "occurredAt": "2024-07-01",
                "accountId": "manual",
                "symbol": "ZZZ",
                "quantity": 1,
                "unitPrice": 2,
                "netCashAmount": -2,
                "activityType": "Trade",
                "activitySubType": "BUY",
                "source": "manual",
            }
        )
        self.assertEqual(store.activity_count(), 2)
        ws = [a for a in store.snapshot()["activities"] if a["source"] == "wealthsimple"][0]
        self.assertEqual(ws["canonicalId"], "ws-cid-aaa-001")
        self.assertFalse(store.looks_like_homemade_id(ws["id"]))
        manual = [a for a in store.snapshot()["activities"] if a["source"] == "manual"][0]
        self.assertIsNone(manual.get("canonicalId"))
        bounds = bagholder.activity_sync_bounds()
        self.assertFalse(bounds["full_history"])
        self.assertTrue(bounds["start_date"])

    def test_token_refresh_needed_uses_expires_at(self):
        now = 1_700_000_000
        sess = {"expires_at": now + 60, "refresh_token": "r"}
        self.assertTrue(bagholder.token_refresh_needed(sess, now=now))
        sess_ok = {"expires_at": now + 3600, "refresh_token": "r"}
        self.assertFalse(bagholder.token_refresh_needed(sess_ok, now=now))
        sess_missing = {"refresh_token": "r"}
        self.assertTrue(bagholder.token_refresh_needed(sess_missing, now=now))

    def test_ensure_fresh_token_does_not_pull_activity(self):
        called = {"graphql": 0, "token": 0}

        def fake_refresh(sess):
            called["token"] += 1
            return True

        def fake_fail(sess):
            called["token"] += 1
            return False

        def fake_graphql(*args, **kwargs):
            called["graphql"] += 1
            return {}

        sess = {
            "refresh_token": "r",
            "expires_at": time_now_minus(),
        }
        bagholder.save_session(sess)
        with bagholder._lock:
            bagholder._state["connected"] = False
        with mock.patch.object(bagholder, "refresh_session", side_effect=fake_refresh):
            with mock.patch.object(bagholder, "graphql", side_effect=fake_graphql):
                ok = bagholder.ensure_fresh_token(sess)
        self.assertTrue(ok)
        self.assertTrue(bagholder._state["connected"])
        self.assertEqual(bagholder._state["error"], "")
        self.assertEqual(called["token"], 1)
        self.assertEqual(called["graphql"], 0)

        called["token"] = 0
        with bagholder._lock:
            bagholder._state["connected"] = False
            bagholder._state["error"] = ""
        with mock.patch.object(bagholder, "refresh_session", side_effect=fake_fail):
            with mock.patch.object(bagholder, "graphql", side_effect=fake_graphql):
                ok = bagholder.ensure_fresh_token(sess)
        self.assertFalse(ok)
        self.assertFalse(bagholder._state["connected"])
        self.assertTrue(bagholder._state["error"])
        self.assertTrue(bagholder.SESSION_PATH.exists())
        self.assertEqual(called["graphql"], 0)

    def test_link_single_manual_match_stamps_canonical_id(self):
        bagholder.append_manual(
            {
                "date": "2024-06-15",
                "symbol": "AAA",
                "side": "BUY",
                "qty": 10,
                "price": 10,
                "currency": "CAD",
                "accountId": "acct-1",
            }
        )
        self.assertEqual(store.activity_count(), 1)
        store.apply_wealthsimple_mapped([bagholder.map_activity(_ws_item())])
        rows = store.snapshot()["activities"]
        self.assertEqual(len(rows), 1)
        self.assertEqual(rows[0]["canonicalId"], "ws-cid-aaa-001")
        self.assertEqual(rows[0]["source"], "manual")

    def test_map_activity_copies_security_id(self):
        row = bagholder.map_activity(_ws_item(securityId="sec-s-abc123"))
        self.assertEqual(row["securityId"], "sec-s-abc123")

    def test_second_sync_stamps_security_id_only(self):
        row = bagholder.map_activity(_ws_item())
        store.apply_wealthsimple_mapped([row])
        later = dict(row)
        later["securityId"] = "sec-s-later"
        later["quantity"] = 999
        later["netCashAmount"] = 1
        again = store.apply_wealthsimple_mapped([later])
        self.assertEqual(again["inserted"], 0)
        self.assertEqual(again["skipped"], 1)
        got = store.snapshot()["activities"][0]
        self.assertEqual(got["securityId"], "sec-s-later")
        self.assertEqual(got["quantity"], 10)
        self.assertEqual(got["netCashAmount"], -100)

    def test_snapshot_includes_securities(self):
        store.upsert_securities(
            [
                {
                    "id": "sec-s-ch",
                    "symbol": "CH",
                    "name": "Charbone Corporation",
                    "primaryExchange": "TSX Venture Exchange",
                    "primaryMic": "XTSV",
                    "currency": "CAD",
                }
            ]
        )
        secs = store.snapshot()["securities"]
        self.assertEqual(len(secs), 1)
        self.assertEqual(secs[0]["id"], "sec-s-ch")
        self.assertEqual(secs[0]["name"], "Charbone Corporation")
        self.assertEqual(secs[0]["primaryMic"], "XTSV")

    def test_fetch_security_reads_stock_fields(self):
        def fake_graphql(sess, operation, variables, query=None):
            self.assertEqual(operation, "FetchSecurity")
            self.assertEqual(variables["securityId"], "sec-s-ch")
            return {
                "security": {
                    "id": "sec-s-ch",
                    "currency": "CAD",
                    "stock": {
                        "name": "Charbone Corporation",
                        "primaryExchange": "TSX Venture Exchange",
                        "primaryMic": "XTSV",
                        "symbol": "CH",
                    },
                    "optionDetails": {},
                }
            }

        with mock.patch.object(bagholder, "graphql", side_effect=fake_graphql):
            rec = bagholder.fetch_security({"access_token": "t"}, "sec-s-ch")
        self.assertEqual(rec["name"], "Charbone Corporation")
        self.assertEqual(rec["symbol"], "CH")
        self.assertEqual(rec["primaryMic"], "XTSV")
        self.assertEqual(rec["currency"], "CAD")

    def test_ledger_uses_listing_line(self):
        html = bagholder.ledger_path().read_text(encoding="utf-8")
        self.assertIn("function listingLine(", html)
        self.assertIn("function listingTicker(", html)
        self.assertIn("book.securities", html)
        self.assertIn("esc(listingLine(t))", html)
        self.assertNotIn('esc(t.side) + " " + formatNumber(t.quantity', html)

    def test_ledger_shows_background_step_on_status_line(self):
        html = bagholder.ledger_path().read_text(encoding="utf-8")
        self.assertIn("listingsFilling || state.ws.syncStep", html)
        self.assertIn("state.ws.listingsFilling || state.ws.syncStep) pollWsStatus", html)
        src = bagholder.ledger_path().with_name("bagholder.py").read_text(encoding="utf-8")
        self.assertIn('"listingsFilling": bool(_state.get("listingsFilling"))', src)

    def test_ledger_executions_heading_includes_count(self):
        html = bagholder.ledger_path().read_text(encoding="utf-8")
        self.assertIn("Executions (' + acts.length + ')", html)
        self.assertNotIn('section-title">Executions</h3>', html)
        self.assertIn(".inner-acts table.blotter th { position: static; }", html)

    def test_ledger_exchange_filter(self):
        html = bagholder.ledger_path().read_text(encoding="utf-8")
        self.assertIn('<label>Exchange</label>', html)
        self.assertIn("function listingExchange(", html)
        self.assertIn("function knownExchanges(", html)
        self.assertIn("f.exchange && listingExchange(t) !== f.exchange", html)

    def test_ledger_prefers_listed_one_over_alpha(self):
        html = bagholder.ledger_path().read_text(encoding="utf-8")
        self.assertIn("function isAlphaVenue(", html)
        self.assertIn("function preferredListing(", html)
        self.assertIn('exch === "ALPHA EXCHANGE"', html)
        self.assertIn("return preferredListing(sec);", html)

    def test_ledger_listing_filters_drive_nav_tiles(self):
        html = bagholder.ledger_path().read_text(encoding="utf-8")
        self.assertIn("function listingFiltersOn(", html)
        self.assertIn("function closedYearReturn(", html)
        self.assertIn("listingFiltersOn()", html)
        self.assertIn("realizedPnlCurve(closedForMetrics)", html)
        self.assertIn("closedAnnualizedReturn(closedFx || [], years)", html)

    def test_ledger_favicon_link(self):
        html = bagholder.ledger_path().read_text(encoding="utf-8")
        self.assertIn('<link rel="icon" type="image/png" href="favicon.png"/>', html)
        icon = bagholder.ledger_path().parent / "favicon.png"
        self.assertTrue(icon.is_file())
        with open(bagholder.__file__, encoding="utf-8") as fh:
            src = fh.read()
        self.assertIn('path in ("/favicon.png", "/favicon.ico")', src)

    def test_ledger_price_filter(self):
        html = bagholder.ledger_path().read_text(encoding="utf-8")
        self.assertIn('<label>Price</label>', html)
        self.assertIn("function priceFilterOn(", html)
        self.assertIn("function rowPriceOk(", html)
        self.assertIn('id="priceOp"', html)
        self.assertIn("rowPriceOk([t.entryPrice, t.exitPrice])", html)
        self.assertIn("priceFilterOn()", html)

def time_now_minus():
    return datetime.now(timezone.utc).timestamp() - 10


def _login_html(js_url="https://assets.wealthsimple.com/app-abc123.js"):
    return '<html><script src="%s"></script></html>' % js_url


def _app_js(client_id):
    return 'var cfg={production:{env:"prod",clientId:"%s"}};' % client_id


class WealthsimpleHttpTest(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.home = self.tmp.name
        os.environ["BAGHOLDER_HOME"] = self.home
        store.set_home(self.home)
        bagholder.set_home(self.home)
        store.ensure()
        with bagholder._lock:
            bagholder._state["connected"] = False
            bagholder._state["error"] = ""
            bagholder._state["syncing"] = False
            bagholder._state["capturing"] = False

    def tearDown(self):
        self.tmp.cleanup()
        os.environ.pop("BAGHOLDER_HOME", None)

    def _urlopen_for_js(self, js_body, html_body=None, js_headers=None, html_headers=None):
        html = html_body if html_body is not None else _login_html()
        if not isinstance(html, (bytes, bytearray)):
            html = html.encode("utf-8")

        def fake_urlopen(req, timeout=None, context=None):
            url = req.full_url if hasattr(req, "full_url") else str(req)
            if "app-" in url and ".js" in url:
                return _FakeHTTPResp(js_body, headers=js_headers)
            return _FakeHTTPResp(html, headers=html_headers)

        return fake_urlopen

    def test_scrape_client_id_from_gzip_js(self):
        js = _app_js(FAKE_CLIENT_ID)
        gz = gzip.compress(js.encode("utf-8"))
        self.assertEqual(gz[:2], b"\x1f\x8b")
        fake = self._urlopen_for_js(
            gz,
            js_headers={"Content-Encoding": "gzip"},
        )
        with mock.patch.object(bagholder, "urlopen", side_effect=fake):
            cid = bagholder.scrape_client_id()
        self.assertEqual(cid, FAKE_CLIENT_ID)
        self.assertEqual(bagholder.CLIENT_ID_PATH.read_text(encoding="utf-8").strip(), FAKE_CLIENT_ID)

    def test_scrape_client_id_from_uncompressed_js(self):
        js = _app_js(FAKE_CLIENT_ID)
        fake = self._urlopen_for_js(js.encode("utf-8"))
        with mock.patch.object(bagholder, "urlopen", side_effect=fake):
            cid = bagholder.scrape_client_id()
        self.assertEqual(cid, FAKE_CLIENT_ID)

    def test_scrape_client_id_from_gzip_login_html(self):
        html_gz = gzip.compress(_login_html().encode("utf-8"))
        js = _app_js(FAKE_CLIENT_ID)
        fake = self._urlopen_for_js(
            js.encode("utf-8"),
            html_body=html_gz,
            html_headers={"Content-Encoding": "gzip"},
        )
        with mock.patch.object(bagholder, "urlopen", side_effect=fake):
            cid = bagholder.scrape_client_id()
        self.assertEqual(cid, FAKE_CLIENT_ID)

    def test_session_client_id_is_written_to_disk(self):
        sess = {"client_id": FAKE_CLIENT_ID}
        found = bagholder.client_id_for(sess)
        self.assertEqual(found, FAKE_CLIENT_ID)
        self.assertTrue(bagholder.CLIENT_ID_PATH.exists())
        self.assertEqual(bagholder.CLIENT_ID_PATH.read_text(encoding="utf-8").strip(), FAKE_CLIENT_ID)

    def test_token_info_uid_is_stored(self):
        sess = {"access_token": "tok", "refresh_token": "r"}
        info = {"application_uid": FAKE_CLIENT_ID}
        found = bagholder.apply_token_info_client_id(sess, info)
        self.assertEqual(found, FAKE_CLIENT_ID)
        self.assertEqual(sess.get("client_id"), FAKE_CLIENT_ID)
        self.assertEqual(bagholder.CLIENT_ID_PATH.read_text(encoding="utf-8").strip(), FAKE_CLIENT_ID)

        sess2 = {"access_token": "tok"}
        nested = bagholder.client_id_from_token_info({"application": {"uid": FAKE_CLIENT_ID}})
        self.assertEqual(nested, FAKE_CLIENT_ID)
        bagholder.apply_token_info_client_id(sess2, {"application": {"uid": FAKE_CLIENT_ID}})
        self.assertEqual(sess2.get("client_id"), FAKE_CLIENT_ID)

    def test_boot_stores_client_id_from_token_info_without_refresh(self):
        bagholder.save_session({
            "access_token": "tok",
            "refresh_token": "r",
            "expires_at": time_now_minus(),
        })
        with mock.patch.object(
            bagholder,
            "token_info",
            return_value={"application_uid": FAKE_CLIENT_ID},
        ):
            with mock.patch.object(bagholder, "refresh_session") as refresh:
                with mock.patch.object(bagholder, "scrape_client_id") as scrape:
                    bagholder.boot_session()
        scrape.assert_not_called()
        refresh.assert_not_called()
        saved = bagholder.load_session()
        self.assertEqual(saved.get("client_id"), FAKE_CLIENT_ID)
        self.assertTrue(bagholder._state["connected"])
        self.assertEqual(bagholder.CLIENT_ID_PATH.read_text(encoding="utf-8").strip(), FAKE_CLIENT_ID)

    def test_capture_stores_client_id_from_token_info_uid(self):
        with mock.patch.object(
            bagholder,
            "token_info",
            return_value={"application_uid": FAKE_CLIENT_ID, "identity_canonical_id": "ident-1"},
        ):
            with mock.patch.object(bagholder, "scrape_client_id") as scrape:
                with mock.patch.object(bagholder, "refresh_session", return_value=True):
                    with mock.patch.object(bagholder.threading, "Thread"):
                        result = bagholder.capture_tokens({
                            "access_token": "tok",
                            "refresh_token": "r",
                        })
        self.assertTrue(result.get("ok"))
        scrape.assert_not_called()
        saved = bagholder.load_session()
        self.assertEqual(saved.get("client_id"), FAKE_CLIENT_ID)
        self.assertEqual(bagholder.CLIENT_ID_PATH.read_text(encoding="utf-8").strip(), FAKE_CLIENT_ID)

    def test_header_copy_and_hover_exist(self):
        html = bagholder.ledger_path().read_text(encoding="utf-8")
        self.assertIn("copyWatchStatus", html)
        self.assertIn('id="watchCopy"', html)
        self.assertIn("watch-status", html)
        self.assertIn("watchStatusFullText", html)
        self.assertIn('title="', html)
        self.assertIn("navigator.clipboard.writeText", html)

    def test_menu_has_refresh_session(self):
        html = bagholder.ledger_path().read_text(encoding="utf-8")
        self.assertIn('id="refreshSession"', html)
        self.assertIn("Refresh session", html)
        self.assertIn("/api/refresh", html)
        self.assertIn("Refreshing session…", html)
        self.assertIn("Wealthsimple token refresh ok", html)

    def test_refresh_now_posts_when_expiry_is_not_near(self):
        sess = {
            "refresh_token": "r",
            "client_id": FAKE_CLIENT_ID,
            "expires_at": "2099-01-01T00:00:00.000Z",
        }
        bagholder.save_session(sess)
        with bagholder._lock:
            bagholder._state["connected"] = True
            bagholder._state["error"] = ""
        with mock.patch.object(
            bagholder,
            "refresh_session",
            return_value=True,
        ) as refresh:
            result = bagholder.refresh_now()
        refresh.assert_called_once()
        self.assertTrue(result.get("ok"))
        self.assertTrue(bagholder._state["connected"])
        self.assertEqual(bagholder._state["error"], "")

    def test_refresh_session_without_client_id_does_not_scrape_or_post(self):
        bagholder.save_session({"refresh_token": "r"})
        self.assertFalse(bagholder.CLIENT_ID_PATH.exists())
        with mock.patch.object(bagholder, "scrape_client_id") as scrape:
            with mock.patch.object(bagholder, "_http_json") as http:
                ok = bagholder.refresh_session({"refresh_token": "r"})
        self.assertFalse(ok)
        scrape.assert_not_called()
        http.assert_not_called()
        self.assertEqual(bagholder._state["error"], "session has no client id")
        self.assertTrue(bagholder.SESSION_PATH.exists())
        self.assertEqual(bagholder.load_session().get("refresh_token"), "r")

    def test_refresh_session_uses_cached_client_id_file(self):
        bagholder.save_client_id(FAKE_CLIENT_ID)
        sess = {"refresh_token": "r"}
        with mock.patch.object(bagholder, "scrape_client_id") as scrape:
            with mock.patch.object(
                bagholder,
                "_http_json",
                return_value={"access_token": "tok", "expires_in": 3600},
            ) as http:
                ok = bagholder.refresh_session(sess)
        self.assertTrue(ok)
        scrape.assert_not_called()
        http.assert_called_once()
        self.assertEqual(sess.get("client_id"), FAKE_CLIENT_ID)
        self.assertIsInstance(sess.get("expires_at"), str)
        self.assertIn("T", sess.get("expires_at"))
        self.assertTrue(sess.get("expires_at").endswith("Z"))

    def test_refresh_session_sets_http_and_oauth_error(self):
        sess = {"refresh_token": "r", "client_id": FAKE_CLIENT_ID}
        bagholder.save_session(sess)
        with mock.patch.object(
            bagholder,
            "_http_json",
            return_value={"error": "invalid_client", "_http_status": 401},
        ):
            ok = bagholder.refresh_session(sess)
        self.assertFalse(ok)
        err = bagholder._state["error"]
        self.assertIn("HTTP 401", err)
        self.assertIn("invalid_client", err)
        self.assertTrue(err.startswith("Wealthsimple token refresh HTTP 401"))
        self.assertNotIn(FAKE_CLIENT_ID, err)
        self.assertNotIn("r", err.split())
        self.assertTrue(bagholder.SESSION_PATH.exists())
        saved = bagholder.load_session()
        self.assertEqual(saved.get("refresh_token"), "r")

    def test_refresh_session_sets_http_error(self):
        sess = {"refresh_token": "r", "client_id": FAKE_CLIENT_ID}
        bagholder.save_session(sess)
        with mock.patch.object(
            bagholder,
            "_http_json",
            return_value={"error": "invalid_grant", "_http_status": 400},
        ):
            ok = bagholder.refresh_session(sess)
        self.assertFalse(ok)
        self.assertEqual(
            bagholder._state["error"],
            "Wealthsimple token refresh HTTP 400 invalid_grant",
        )
        self.assertTrue(bagholder.SESSION_PATH.exists())
        saved = bagholder.load_session()
        self.assertEqual(saved.get("refresh_token"), "r")

    def test_refresh_session_sets_oauth_error_text(self):
        sess = {"refresh_token": "r", "client_id": FAKE_CLIENT_ID}
        with mock.patch.object(bagholder, "_http_json", return_value={"error": "invalid_grant"}):
            ok = bagholder.refresh_session(sess)
        self.assertFalse(ok)
        self.assertEqual(bagholder._state["error"], "invalid_grant")

    def test_ensure_fresh_token_sets_error_on_failed_refresh(self):
        sess = {"refresh_token": "r", "expires_at": time_now_minus()}
        bagholder.save_session(sess)
        with mock.patch.object(bagholder, "refresh_session", return_value=False):
            ok = bagholder.ensure_fresh_token(sess)
        self.assertFalse(ok)
        self.assertFalse(bagholder._state["connected"])
        self.assertTrue(bagholder._state["error"])
        self.assertTrue(bagholder.SESSION_PATH.exists())

    def test_boot_session_sets_error_when_refresh_fails(self):
        sess = {"refresh_token": "r", "expires_at": time_now_minus()}
        bagholder.save_session(sess)
        with mock.patch.object(bagholder, "refresh_session", return_value=False):
            bagholder.boot_session()
        self.assertFalse(bagholder._state["connected"])
        self.assertTrue(bagholder._state["error"])
        self.assertTrue(bagholder.SESSION_PATH.exists())

    def test_http_json_invalid_body_returns_error_dict(self):
        def fake_urlopen(req, timeout=None, context=None):
            return _FakeHTTPResp(b"not-json{", status=200)

        with mock.patch.object(bagholder, "urlopen", side_effect=fake_urlopen):
            data = bagholder._http_json("GET", "https://example.test/token")
        self.assertEqual(data.get("error"), "invalid_json")
        self.assertIn("_http_status", data)

    def test_http_json_reads_gzip_json(self):
        raw = gzip.compress(b'{"access_token":"tok","expires_in":3600}')

        def fake_urlopen(req, timeout=None, context=None):
            return _FakeHTTPResp(raw, headers={"Content-Encoding": "gzip"})

        with mock.patch.object(bagholder, "urlopen", side_effect=fake_urlopen):
            data = bagholder._http_json("POST", "https://example.test/token", {"grant_type": "refresh_token"})
        self.assertEqual(data.get("access_token"), "tok")
        self.assertNotIn("_http_status", data)



    def test_trade_groups_roundtrip(self):
        groups = [
            {"id": "g_one", "locked": True, "members": ["a|b|1.00000000", "c|d|2.00000000"]},
            {"id": "g_one", "locked": False, "members": ["dup"]},
            {"id": "", "members": ["x"]},
            {"id": "g_empty", "members": []},
            {"id": "g_two", "locked": 1, "members": ["x", "x", "y"]},
        ]
        saved = store.save_trade_groups(groups)
        self.assertEqual([g["id"] for g in saved], ["g_one", "g_two"])
        self.assertEqual(saved[0]["members"], ["a|b|1.00000000", "c|d|2.00000000"])
        self.assertEqual(saved[1]["members"], ["x", "y"])
        self.assertTrue(saved[0]["locked"])
        self.assertTrue(saved[1]["locked"])
        snap = store.snapshot()
        self.assertEqual(snap["tradeGroups"], saved)
        self.assertEqual(store.trade_groups(), saved)

    def test_trade_groups_rejects_non_list(self):
        store.set_meta("trade_groups", "{}")
        self.assertEqual(store.trade_groups(), [])
        self.assertEqual(store.save_trade_groups(None), [])

    def test_trade_notes_roundtrip(self):
        saved = store.save_trade_notes({
            "g_one": {"thesis": "scale in", "tag": "hold", "grade": "A"},
            "g_empty": {"thesis": "", "tag": "", "grade": ""},
            "g_bad": {"thesis": "x", "tag": "y", "grade": "Z"},
            "": {"thesis": "nope"},
        })
        self.assertEqual(saved["g_one"]["grade"], "A")
        self.assertEqual(saved["g_one"]["tag"], "hold")
        self.assertNotIn("g_empty", saved)
        self.assertEqual(saved["g_bad"]["grade"], "")
        self.assertEqual(saved["g_bad"]["thesis"], "x")
        snap = store.snapshot()
        self.assertEqual(snap["notes"], saved)
        self.assertEqual(store.trade_notes(), saved)

    def test_ledger_posts_notes(self):
        html = bagholder.ledger_path().read_text(encoding="utf-8")
        self.assertIn("function persistNotes(", html)
        self.assertIn("function saveDrawerNote(", html)
        self.assertIn('api("POST", "/api/notes"', html)
        src = bagholder.ledger_path().with_name("bagholder.py").read_text(encoding="utf-8")
        self.assertIn('path == "/api/notes"', src)
        self.assertIn('"notes": book.get("notes") or {}', src)

    def test_nav_history_migrates_date_pk_to_account_date(self):
        path = store.db_path()
        conn = sqlite3.connect(str(path))
        conn.execute("DROP TABLE nav_history")
        conn.execute(
            """
            CREATE TABLE nav_history (
                date TEXT PRIMARY KEY,
                equity REAL,
                currency TEXT,
                net_deposits REAL
            )
            """
        )
        conn.execute(
            "INSERT INTO nav_history (date, equity, currency, net_deposits) "
            "VALUES (?, ?, ?, ?)",
            ("2024-01-02", 1000.0, "CAD", 100.0),
        )
        conn.execute(
            "INSERT OR REPLACE INTO meta(key, value) VALUES (?, ?)",
            ("schema_version", "1"),
        )
        conn.commit()
        conn.close()
        store.ensure()
        snap = store.snapshot()
        self.assertEqual(store.get_meta("schema_version"), "3")
        self.assertEqual(len(snap["navHistory"]), 1)
        self.assertEqual(snap["navHistory"][0]["date"], "2024-01-02")
        self.assertEqual(snap["navHistory"][0]["equity"], 1000.0)
        self.assertEqual(snap["navHistory"][0]["netDeposits"], 100.0)
        self.assertNotIn("accountId", snap["navHistory"][0])
        self.assertEqual(snap["navByAccount"], {})
        info_conn = sqlite3.connect(str(path))
        info_conn.row_factory = sqlite3.Row
        pk = {r["name"] for r in info_conn.execute("PRAGMA table_info(nav_history)") if r["pk"]}
        info_conn.close()
        self.assertEqual(pk, {"account_id", "date"})

    def test_replace_nav_by_account_and_snapshot(self):
        store.replace_nav(
            [
                {"date": "2024-01-01", "equity": 10, "currency": "CAD", "netDeposits": 1, "accountId": ""},
                {"date": "2024-01-02", "equity": 11, "net_deposits": 2},
                {"date": "2024-01-01", "equity": 5, "currency": "CAD", "accountId": "TFSA"},
                {"date": "2024-01-02", "equity": 6, "account_id": "TFSA", "netDeposits": 3},
                {"date": "2024-01-01", "equity": 7, "accountId": "RRSP"},
            ]
        )
        snap = store.snapshot()
        self.assertEqual([p["date"] for p in snap["navHistory"]], ["2024-01-01", "2024-01-02"])
        self.assertEqual(snap["navHistory"][0]["equity"], 10)
        self.assertEqual(set(snap["navByAccount"]), {"TFSA", "RRSP"})
        self.assertEqual(snap["navByAccount"]["TFSA"][0]["equity"], 5)
        self.assertEqual(snap["navByAccount"]["TFSA"][1]["netDeposits"], 3)
        self.assertNotIn("accountId", snap["navByAccount"]["TFSA"][0])
        store.replace_nav(
            [
                {"date": "2024-06-01", "equity": 20, "accountId": ""},
                {"date": "2024-06-01", "equity": 8, "accountId": "TFSA"},
            ]
        )
        snap = store.snapshot()
        self.assertEqual([p["date"] for p in snap["navHistory"]], ["2024-06-01"])
        self.assertEqual(set(snap["navByAccount"]), {"TFSA"})
        self.assertNotIn("RRSP", snap["navByAccount"])

    def test_upsert_nav_keeps_existing_days(self):
        store.replace_nav(
            [
                {"date": "2024-01-01", "equity": 10, "accountId": ""},
                {"date": "2024-01-01", "equity": 5, "accountId": "TFSA"},
            ]
        )
        store.upsert_nav(
            [
                {"date": "2024-01-02", "equity": 11, "accountId": ""},
                {"date": "2024-01-01", "equity": 6, "accountId": "TFSA"},
            ]
        )
        snap = store.snapshot()
        self.assertEqual([p["date"] for p in snap["navHistory"]], ["2024-01-01", "2024-01-02"])
        self.assertEqual(snap["navHistory"][1]["equity"], 11)
        self.assertEqual(snap["navByAccount"]["TFSA"][0]["equity"], 6)
        self.assertEqual(store.nav_last_dates(), {"": "2024-01-02", "TFSA": "2024-01-01"})

    def test_fetch_nav_history_since_date_skips_older_years(self):
        calls = []

        def fake_graphql(sess, operation, variables, query=None):
            calls.append(dict(variables))
            return {"identity": {"financials": {"historicalDaily": {"edges": [], "pageInfo": {}}}}}

        with mock.patch.object(bagholder, "graphql", side_effect=fake_graphql):
            bagholder.fetch_nav_history({"access_token": "t"}, "ident-1", since_date="2026-08-30")
        self.assertTrue(calls)
        for variables in calls:
            self.assertGreaterEqual(variables["startDate"], "2026-08-30")
            self.assertTrue(variables["startDate"].startswith("2026"))

    def test_nav_account_groups_joins_same_nickname(self):
        groups = bagholder.nav_account_groups(
            [
                {"id": "cad-1", "nickname": "TFSA", "currency": "CAD"},
                {"id": "usd-1", "nickname": "TFSA", "currency": "USD"},
                {"id": "rrsp-1", "nickname": "", "unifiedAccountType": "RRSP"},
            ]
        )
        self.assertEqual(groups["TFSA"], ["cad-1", "usd-1"])
        self.assertEqual(groups["RRSP"], ["rrsp-1"])

    def test_fetch_nav_history_identity_wide_omits_account_ids(self):
        calls = []

        def fake_graphql(sess, operation, variables, query=None):
            calls.append((operation, dict(variables), query))
            return {"identity": {"financials": {"historicalDaily": {"edges": [], "pageInfo": {}}}}}

        with mock.patch.object(bagholder, "graphql", side_effect=fake_graphql):
            bagholder.fetch_nav_history({"access_token": "t"}, "ident-1")
        self.assertTrue(calls)
        for op, variables, query in calls:
            self.assertEqual(op, "IdentityHistoricalFinancialsQuery")
            self.assertNotIn("accountIds", variables)
            self.assertIs(query, bagholder.Q_IDENTITY_HISTORICAL_FINANCIALS)
            self.assertEqual(variables.get("limit"), 400)
        self.assertNotIn("$accountIds", bagholder.Q_IDENTITY_HISTORICAL_FINANCIALS)
        self.assertNotIn("accounts: $accountIds", bagholder.Q_IDENTITY_HISTORICAL_FINANCIALS)

    def test_fetch_account_nav_history_uses_account_query(self):
        calls = []

        def fake_graphql(sess, operation, variables, query=None):
            calls.append((operation, dict(variables), query))
            return {
                "account": {
                    "financials": {
                        "historicalDaily": {
                            "edges": [
                                {
                                    "node": {
                                        "date": "2024-01-02",
                                        "netLiquidationValueV2": {"amount": "12.5", "currency": "CAD"},
                                        "netDepositsV2": {"amount": "3", "currency": "CAD"},
                                    }
                                }
                            ],
                            "pageInfo": {},
                        }
                    }
                }
            }

        with mock.patch.object(bagholder, "graphql", side_effect=fake_graphql):
            pts = bagholder.fetch_account_nav_history({"access_token": "t"}, "acct-1")
        self.assertTrue(calls)
        self.assertEqual(pts[0]["date"], "2024-01-02")
        self.assertEqual(pts[0]["equity"], 12.5)
        self.assertEqual(pts[0]["netDeposits"], 3.0)
        for op, variables, query in calls:
            self.assertEqual(op, "FetchAccountHistoricalFinancials")
            self.assertEqual(variables.get("id"), "acct-1")
            self.assertEqual(variables.get("first"), 400)
            self.assertEqual(variables.get("resolution"), "DAILY")
            self.assertNotIn("accountIds", variables)
            self.assertNotIn("identityId", variables)
            self.assertIs(query, bagholder.Q_FETCH_ACCOUNT_HISTORICAL_FINANCIALS)
        self.assertIn("account(id: $id)", bagholder.Q_FETCH_ACCOUNT_HISTORICAL_FINANCIALS)
        self.assertIn("$resolution: DateResolution!", bagholder.Q_FETCH_ACCOUNT_HISTORICAL_FINANCIALS)
        self.assertIn("FetchAccountHistoricalFinancials", bagholder.QUERIES)

    def test_nav_points_from_payload_accepts_v2_and_identity(self):
        ident_pts, _ = bagholder._nav_points_from_payload(
            {
                "identity": {
                    "financials": {
                        "historicalDaily": {
                            "edges": [
                                {
                                    "node": {
                                        "date": "2024-02-01",
                                        "netLiquidationValue": {"amount": 10, "currency": "CAD"},
                                        "netDeposits": {"amount": 1, "currency": "CAD"},
                                    }
                                }
                            ],
                            "pageInfo": {},
                        }
                    }
                }
            }
        )
        self.assertEqual(ident_pts[0]["equity"], 10.0)
        self.assertEqual(ident_pts[0]["netDeposits"], 1.0)
        acc_pts, _ = bagholder._nav_points_from_payload(
            {
                "account": {
                    "financials": {
                        "historicalDaily": {
                            "edges": [
                                {
                                    "node": {
                                        "date": "2024-02-01",
                                        "netLiquidationValueV2": {"amount": "20", "currency": "CAD"},
                                        "netDepositsV2": {"amount": "4", "currency": "CAD"},
                                    }
                                }
                            ],
                            "pageInfo": {},
                        }
                    }
                }
            }
        )
        self.assertEqual(acc_pts[0]["equity"], 20.0)
        self.assertEqual(acc_pts[0]["netDeposits"], 4.0)

    def test_merge_nav_points_sums_equity_and_deposits(self):
        merged = bagholder.merge_nav_points(
            [
                [{"date": "2024-01-01", "equity": 10, "currency": "CAD", "netDeposits": 1}],
                [
                    {"date": "2024-01-01", "equity": 5, "currency": "CAD", "netDeposits": 2},
                    {"date": "2024-01-02", "equity": 6, "currency": "CAD"},
                ],
            ]
        )
        self.assertEqual(merged[0]["date"], "2024-01-01")
        self.assertEqual(merged[0]["equity"], 15.0)
        self.assertEqual(merged[0]["netDeposits"], 3.0)
        self.assertEqual(merged[1]["date"], "2024-01-02")
        self.assertEqual(merged[1]["equity"], 6.0)
        self.assertNotIn("netDeposits", merged[1])

    def test_fetch_nickname_nav_history_merges_and_records_errors(self):
        def fake_account(sess, account_id, since_date=None):
            if account_id == "rrsp-1":
                raise RuntimeError("nope")
            if account_id == "cad-1":
                return [{"date": "2024-01-01", "equity": 10, "currency": "CAD", "netDeposits": 1}]
            if account_id == "usd-1":
                return [{"date": "2024-01-01", "equity": 5, "currency": "CAD", "netDeposits": 2}]
            return []

        accounts = [
            {"id": "cad-1", "nickname": "TFSA"},
            {"id": "usd-1", "nickname": "TFSA"},
            {"id": "rrsp-1", "nickname": "RRSP"},
        ]
        with mock.patch.object(bagholder, "fetch_account_nav_history", side_effect=fake_account):
            pts, errors = bagholder.fetch_nickname_nav_history({"access_token": "t"}, accounts)
        self.assertEqual([p["accountId"] for p in pts], ["TFSA"])
        self.assertEqual(pts[0]["equity"], 15.0)
        self.assertEqual(pts[0]["netDeposits"], 3.0)
        self.assertTrue(any(e.startswith("RRSP:") for e in errors))
        self.assertIn("nope", errors[0])

    def test_ledger_nav_follows_account_filter(self):
        html = bagholder.ledger_path().read_text(encoding="utf-8")
        self.assertIn("navByAccount", html)
        self.assertIn("ledger.navByAccount.v1", html)
        self.assertIn("function yearsFromActivities", html)
        self.assertNotIn("yearsFromNavOrActivities", html)
        self.assertIn("const hist = navHist();", html)
        self.assertIn("state.navByAccount = (book && book.navByAccount", html)
        self.assertNotIn("const hist = state.navHistory || [];", html)

    def test_parse_yahoo_chart_prefers_adjclose(self):
        payload = {
            "chart": {
                "result": [
                    {
                        "timestamp": [1388620800, 1388707200, 1388966400],
                        "indicators": {
                            "quote": [{"close": [184.35, None, 184.90]}],
                            "adjclose": [{"adjclose": [184.35, None, 184.69]}],
                        },
                    }
                ]
            }
        }
        got = bagholder.parse_yahoo_chart(payload)
        self.assertEqual(got, {"2014-01-02": 184.35, "2014-01-06": 184.69})
        got2 = bagholder.parse_yahoo_chart(json.dumps(payload))
        self.assertEqual(got2, got)

    def test_parse_stooq_csv_uses_close_column(self):
        csv = """Date,Open,High,Low,Close,Volume
2014-01-02,184.00,185.00,183.00,184.35,1000
2014-01-03,1,1,1,0,1
not-a-date,1,1,1,1,1
2014-01-06,184.00,185.00,183.00,184.69,1000

"""
        got = bagholder.parse_stooq_csv(csv)
        self.assertEqual(got, {"2014-01-02": 184.35, "2014-01-06": 184.69})

    def test_spy_by_date_meta_roundtrip(self):
        self.assertEqual(store.spy_by_date(), {})
        store.save_spy_by_date({})
        self.assertEqual(store.spy_by_date(), {})
        store.save_spy_by_date({"2014-01-02": 184.35, "bad": 1, "2014-01-03": 0})
        self.assertEqual(store.spy_by_date(), {"2014-01-02": 184.35})
        snap = store.snapshot()
        self.assertEqual(snap["spyByDate"], {"2014-01-02": 184.35})
        book = bagholder.load_book()
        self.assertEqual(book["spyByDate"], {"2014-01-02": 184.35})

    def _yahoo_fixture(self):
        return json.dumps(
            {
                "chart": {
                    "result": [
                        {
                            "timestamp": [1388620800, 1388966400],
                            "indicators": {
                                "adjclose": [{"adjclose": [184.35, 184.69]}],
                                "quote": [{"close": [184.35, 184.69]}],
                            },
                        }
                    ]
                }
            }
        )

    def _fake_curl_run(self, body, returncode=0, stderr=b"", http_code="200", expect_url=None):
        payload = body if body.endswith("\n") or not body else body + "\n"
        stdout = (payload + http_code).encode("utf-8")
        allowed = set(bagholder.YAHOO_SPY_URLS) | {bagholder.STOOQ_SPY_URL}

        def fake_run(args, capture_output=True, timeout=None):
            self.assertEqual(args[0], "/usr/bin/curl")
            self.assertIn("-sL", args)
            self.assertIn("--max-time", args)
            self.assertIn("25", args)
            url = args[-1]
            self.assertIn(url, allowed)
            if expect_url is not None:
                self.assertEqual(url, expect_url)
            self.assertNotIn("fred.stlouisfed.org", " ".join(str(a) for a in args))
            self.assertIsNone(getattr(fake_run, "context", None))
            class Result:
                pass
            r = Result()
            r.returncode = returncode
            r.stdout = stdout
            r.stderr = stderr if isinstance(stderr, bytes) else str(stderr).encode("utf-8")
            return r

        return fake_run

    def test_refresh_spy_prices_fixture_yahoo_not_live(self):
        fake_run = self._fake_curl_run(self._yahoo_fixture())
        with mock.patch.object(bagholder.shutil, "which", return_value="/usr/bin/curl"):
            with mock.patch.object(bagholder.subprocess, "run", side_effect=fake_run):
                with mock.patch.object(bagholder, "urlopen", side_effect=AssertionError("live yahoo")):
                    got = bagholder.refresh_spy_prices()
        self.assertEqual(got["2014-01-02"], 184.35)
        self.assertEqual(got["2014-01-06"], 184.69)
        self.assertEqual(store.spy_by_date()["2014-01-02"], 184.35)
        self.assertFalse((bagholder._state.get("error") or "").startswith("S&P 500"))

    def test_refresh_spy_prices_url_order_yahoo_then_stooq(self):
        seen = []
        payload = self._yahoo_fixture()

        def fake_run(args, capture_output=True, timeout=None):
            url = args[-1]
            seen.append(url)
            class Result:
                pass
            r = Result()
            r.returncode = 0
            r.stderr = b""
            if url == bagholder.YAHOO_SPY_URLS[0]:
                r.stdout = (payload + "\n200").encode("utf-8")
            else:
                r.stdout = b'{"chart":{"result":[]}}\n200'
            return r

        with mock.patch.object(bagholder.shutil, "which", return_value="/usr/bin/curl"):
            with mock.patch.object(bagholder.subprocess, "run", side_effect=fake_run):
                with mock.patch.object(bagholder, "urlopen", side_effect=AssertionError("live")):
                    got = bagholder.refresh_spy_prices()
        self.assertEqual(seen, [bagholder.YAHOO_SPY_URLS[0]])
        self.assertEqual(got["2014-01-02"], 184.35)

    def test_refresh_spy_prices_falls_back_stooq(self):
        seen = []
        stooq = "Date,Open,High,Low,Close,Volume\n2014-01-02,1,1,1,184.35,1\n"

        def fake_run(args, capture_output=True, timeout=None):
            url = args[-1]
            seen.append(url)
            class Result:
                pass
            r = Result()
            r.returncode = 0
            r.stderr = b""
            if url == bagholder.STOOQ_SPY_URL:
                r.stdout = (stooq + "200").encode("utf-8")
            else:
                r.stdout = b'{"chart":{"result":[]}}\n200'
            return r

        with mock.patch.object(bagholder.shutil, "which", return_value="/usr/bin/curl"):
            with mock.patch.object(bagholder.subprocess, "run", side_effect=fake_run):
                got = bagholder.refresh_spy_prices()
        self.assertEqual(
            seen,
            [
                bagholder.YAHOO_SPY_URLS[0],
                bagholder.YAHOO_SPY_URLS[1],
                bagholder.YAHOO_SPY_URLS[2],
                bagholder.STOOQ_SPY_URL,
            ],
        )
        self.assertEqual(got["2014-01-02"], 184.35)

    def test_refresh_spy_prices_keeps_existing_on_fail(self):
        store.save_spy_by_date({"2014-01-02": 184.35})
        with mock.patch.object(bagholder.shutil, "which", return_value="/usr/bin/curl"):
            with mock.patch.object(
                bagholder.subprocess, "run", side_effect=RuntimeError("offline")
            ):
                got = bagholder.refresh_spy_prices()
        self.assertEqual(got, {"2014-01-02": 184.35})
        self.assertIn("S&P 500", bagholder._state["error"])
        self.assertIn("offline", bagholder._state["error"])
        empty = self._fake_curl_run('{"chart":{"result":[]}}', http_code="200")
        with mock.patch.object(bagholder.shutil, "which", return_value="/usr/bin/curl"):
            with mock.patch.object(bagholder.subprocess, "run", side_effect=empty):
                got = bagholder.refresh_spy_prices()
        self.assertEqual(got, {"2014-01-02": 184.35})
        self.assertIn("empty table", bagholder._state["error"])
        self.assertIn("HTTP 200", bagholder._state["error"])

    def test_ledger_uses_python_spy_before_persist(self):
        html = bagholder.ledger_path().read_text(encoding="utf-8")
        self.assertIn("book.spyByDate", html)
        idx = html.find("function replaceWsActivities(book)")
        self.assertGreater(idx, 0)
        chunk = html[idx:idx + 1800]
        spy_at = chunk.find("state.spyByDate = book.spyByDate")
        persist_at = chunk.find("persist();")
        render_at = chunk.find("render();")
        self.assertGreater(spy_at, 0)
        self.assertGreater(persist_at, spy_at)
        self.assertGreater(render_at, persist_at)

    def test_api_book_does_not_block_on_spy(self):
        src = Path(bagholder.__file__).read_text(encoding="utf-8")
        idx = src.find('if path == "/api/book"')
        self.assertGreater(idx, 0)
        chunk = src[idx : idx + 1200]
        self.assertIn("load_book()", chunk)
        self.assertIn('"spyByDate"', chunk)
        self.assertIn("threading.Thread", chunk)
        self.assertIn("refresh_spy_prices", chunk)
        self.assertNotIn("refresh_spy_prices()\n                book = load_book()", chunk)
        self.assertIn("self._send(", chunk)
        send_at = chunk.find("self._send(")
        refresh_call = chunk.find("refresh_spy_prices()")
        # blocking call refresh_spy_prices() must not sit before send
        self.assertEqual(refresh_call, -1)
        self.assertGreater(send_at, 0)

    def test_ledger_persist_skips_empty_spy(self):
        html = bagholder.ledger_path().read_text(encoding="utf-8")
        persist_at = html.find("function persist()")
        self.assertGreater(persist_at, 0)
        chunk = html[persist_at : persist_at + 700]
        self.assertIn("if (Object.keys(spy).length) localStorage.setItem(LS_SPY", chunk)
        empty_at = html.find("function emptyStateHtml()")
        empty = html[empty_at : empty_at + 2800]
        yet = empty.find("No activity yet")
        self.assertGreater(yet, 0)
        before = empty[:yet]
        self.assertIn("state.ws.lastSync", before)
        self.assertIn("state.ws.activityCount", before)
        self.assertIn("return \"\";", empty[empty.find("savedBook") : yet] if "savedBook" in empty else before)
        load_at = html.find("function loadPersisted()")
        after_load = html[load_at : load_at + 9000]
        # still call ensureSpyPrices after boot/load
        self.assertIn("ensureSpyPrices();", html.split("loadPersisted();", 1)[1][:800])

    def test_ledger_polls_book_until_spy_dates(self):
        html = bagholder.ledger_path().read_text(encoding="utf-8")
        idx = html.find("function replaceWsActivities(book)")
        self.assertGreater(idx, 0)
        chunk = html[idx:idx + 2800]
        self.assertIn("pollSpyByDateIfEmpty(book)", chunk)
        helper = html.find("function pollSpyByDateIfEmpty(book)")
        self.assertGreater(helper, 0)
        h = html[helper:helper + 900]
        self.assertIn('api("GET", "/api/book")', h)
        self.assertIn("state.ws.connected", h)
        self.assertIn("book.spyByDate", h)
        self.assertIn("__spyBookPolls", h)

    def test_ledger_spy_is_yahoo_not_fred(self):
        html = bagholder.ledger_path().read_text(encoding="utf-8")
        self.assertIn("query1.finance.yahoo.com/v8/finance/chart/SPY", html)
        self.assertIn("query2.finance.yahoo.com/v8/finance/chart/SPY", html)
        self.assertIn("stooq.com/q/d/l/?s=spy.us", html)
        self.assertIn("function ingestYahooChart", html)
        self.assertIn('const LS_SPY = "ledger.spy.v1"', html)
        self.assertNotIn("fred.stlouisfed.org", html)
        self.assertNotIn("ingestFredSP500Csv", html)
        src = Path(bagholder.__file__).read_text(encoding="utf-8")
        self.assertNotIn("fred.stlouisfed.org", src)
        self.assertIn(bagholder.YAHOO_SPY_URLS[0], src)
        self.assertIn(bagholder.STOOQ_SPY_URL, src)

    def test_refresh_spy_prices_not_live_urlopen_only_fixture(self):
        store.save_spy_by_date({"2014-01-02": 184.35})
        with mock.patch.object(bagholder.shutil, "which", return_value="/usr/bin/curl"):
            with mock.patch.object(bagholder, "urlopen", side_effect=AssertionError("live yahoo")):
                with mock.patch.object(
                    bagholder.subprocess, "run", side_effect=RuntimeError("offline")
                ):
                    got = bagholder.refresh_spy_prices()
        self.assertEqual(got["2014-01-02"], 184.35)
        self.assertIn("S&P 500", bagholder._state["error"])

    def test_refresh_spy_prices_curl_http_error_status_line(self):
        store.save_spy_by_date({"2014-01-02": 184.35})
        calls = []

        def fake_run(args, capture_output=True, timeout=None):
            calls.append(args[-1])
            class Result:
                pass
            r = Result()
            r.returncode = 0
            r.stdout = b"nope\n503"
            r.stderr = b""
            return r

        with mock.patch.object(bagholder.shutil, "which", return_value="/usr/bin/curl"):
            with mock.patch.object(bagholder.subprocess, "run", side_effect=fake_run):
                got = bagholder.refresh_spy_prices()
        self.assertEqual(got["2014-01-02"], 184.35)
        self.assertIn("HTTP 503", bagholder._state["error"])
        self.assertEqual(len(calls), 4)


if __name__ == "__main__":
    unittest.main()
