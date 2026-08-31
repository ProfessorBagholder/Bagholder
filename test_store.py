#!/usr/bin/env python3
"""SQLite store and Wealthsimple sync bounds. Synthetic rows only."""

from __future__ import annotations

import gzip
import os
import sqlite3
import tempfile
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
        self.assertEqual(store.get_meta("schema_version"), "2")
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
        self.assertNotIn("$accountIds", bagholder.Q_IDENTITY_HISTORICAL_FINANCIALS)
        self.assertNotIn("accounts: $accountIds", bagholder.Q_IDENTITY_HISTORICAL_FINANCIALS)

    def test_fetch_nav_history_with_account_ids_uses_filter(self):
        calls = []

        def fake_graphql(sess, operation, variables, query=None):
            calls.append((operation, dict(variables), query))
            return {"identity": {"financials": {"historicalDaily": {"edges": [], "pageInfo": {}}}}}

        with mock.patch.object(bagholder, "graphql", side_effect=fake_graphql):
            bagholder.fetch_nav_history({"access_token": "t"}, "ident-1", account_ids=["a", "b", ""])
        self.assertTrue(calls)
        for op, variables, query in calls:
            self.assertEqual(op, "IdentityHistoricalFinancialsQuery")
            self.assertEqual(variables.get("accountIds"), ["a", "b"])
            self.assertIs(query, bagholder.Q_IDENTITY_HISTORICAL_FINANCIALS_ACCOUNTS)
        self.assertIn("$accountIds: [ID!]", bagholder.Q_IDENTITY_HISTORICAL_FINANCIALS_ACCOUNTS)
        self.assertIn("accounts: $accountIds", bagholder.Q_IDENTITY_HISTORICAL_FINANCIALS_ACCOUNTS)

    def test_fetch_nav_history_empty_list_is_identity_wide(self):
        calls = []

        def fake_graphql(sess, operation, variables, query=None):
            calls.append(variables)
            return {"identity": {"financials": {"historicalDaily": {"edges": [], "pageInfo": {}}}}}

        with mock.patch.object(bagholder, "graphql", side_effect=fake_graphql):
            bagholder.fetch_nav_history({"access_token": "t"}, "ident-1", account_ids=[])
        self.assertTrue(calls)
        for variables in calls:
            self.assertNotIn("accountIds", variables)

    def test_ledger_nav_follows_account_filter(self):
        html = bagholder.ledger_path().read_text(encoding="utf-8")
        self.assertIn("navByAccount", html)
        self.assertIn("ledger.navByAccount.v1", html)
        self.assertIn("function yearsFromNavOrActivities", html)
        self.assertIn("const hist = navHist();", html)
        self.assertIn("state.navByAccount = (book && book.navByAccount", html)
        self.assertNotIn("const hist = state.navHistory || [];", html)

if __name__ == "__main__":
    unittest.main()
