#!/usr/bin/env python3
"""SQLite store and Wealthsimple sync bounds. Synthetic rows only."""

from __future__ import annotations

import os
import sqlite3
import tempfile
import unittest
from datetime import datetime, timezone
from unittest import mock

import bagholder
import store


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
        self.assertEqual(stored[0]["id"], "ws-cid-aaa-001")
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
        self.assertEqual(stored["id"], act["id"])

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
        self.assertNotEqual(saved["id"], "do-not-keep-this")

    def test_occurred_at_not_cut_to_date_for_wealthsimple_row(self):
        row = bagholder.map_activity(_ws_item())
        self.assertEqual(row["occurredAt"], "2024-06-15T13:45:22.123Z")
        self.assertEqual(row["transactionDate"], "2024-06-15")
        self.assertEqual(row["id"], "ws-cid-aaa-001")
        store.apply_wealthsimple_mapped([row])
        stored = store.snapshot()["activities"][0]
        self.assertEqual(stored["occurredAt"], "2024-06-15T13:45:22.123Z")
        self.assertTrue("T" in stored["occurredAt"])
        self.assertEqual(stored["id"], "ws-cid-aaa-001")
        self.assertEqual(stored["canonicalId"], "ws-cid-aaa-001")

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
        self.assertEqual(ws["id"], "ws-cid-aaa-001")
        self.assertFalse(store.looks_like_homemade_id(ws["id"]))
        manual = [a for a in store.snapshot()["activities"] if a["source"] == "manual"][0]
        self.assertIsNone(manual.get("canonicalId"))
        self.assertNotEqual(manual["id"], ws["id"])
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
        self.assertEqual(called["token"], 1)
        self.assertEqual(called["graphql"], 0)

        called["token"] = 0
        with bagholder._lock:
            bagholder._state["connected"] = False
        with mock.patch.object(bagholder, "refresh_session", side_effect=fake_fail):
            with mock.patch.object(bagholder, "graphql", side_effect=fake_graphql):
                ok = bagholder.ensure_fresh_token(sess)
        self.assertFalse(ok)
        self.assertFalse(bagholder._state["connected"])
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
        self.assertNotEqual(rows[0]["id"], "ws-cid-aaa-001")

    def test_wealthsimple_insert_uses_canonical_id_as_row_id(self):
        row = bagholder.map_activity(_ws_item())
        self.assertEqual(row["id"], "ws-cid-aaa-001")
        self.assertEqual(row["canonicalId"], "ws-cid-aaa-001")
        stored = store.insert_activity(row)
        self.assertEqual(stored["id"], "ws-cid-aaa-001")
        self.assertEqual(stored["canonicalId"], "ws-cid-aaa-001")

        other = bagholder.map_activity(_ws_item(canonicalId="ws-cid-bbb-002"))
        other.pop("id", None)
        stored2 = store.insert_activity(other)
        self.assertEqual(stored2["id"], "ws-cid-bbb-002")
        self.assertEqual(stored2["canonicalId"], "ws-cid-bbb-002")

    def test_migrates_wealthsimple_row_id_to_canonical_id(self):
        conn = sqlite3.connect(str(store.db_path()))
        conn.execute(
            "INSERT INTO activities (id, canonical_id, transaction_date, source) "
            "VALUES (?, ?, ?, ?)",
            (
                "11111111-2222-3333-4444-555555555555",
                "ws-cid-mig-001",
                "2024-06-15",
                "wealthsimple",
            ),
        )
        conn.execute(
            "INSERT INTO activities (id, canonical_id, transaction_date, source) "
            "VALUES (?, ?, ?, ?)",
            (
                "aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee",
                None,
                "2024-07-02",
                "csv",
            ),
        )
        conn.execute(
            "INSERT INTO activities (id, canonical_id, transaction_date, source) "
            "VALUES (?, ?, ?, ?)",
            (
                "cccccccc-dddd-eeee-ffff-000000000000",
                "2024-06-16|acct-1|AAA|10|10|-100|Trade|BUY",
                "2024-06-16",
                "wealthsimple",
            ),
        )
        conn.execute(
            "UPDATE meta SET value = '1' WHERE key = 'schema_version'"
        )
        conn.commit()
        conn.close()

        store.ensure()
        self.assertEqual(store.get_meta("schema_version"), "2")
        snap = store.snapshot()["activities"]
        self.assertEqual(len(snap), 3)
        ws = [a for a in snap if a.get("canonicalId") == "ws-cid-mig-001"][0]
        self.assertEqual(ws["id"], "ws-cid-mig-001")
        csv = [a for a in snap if a["source"] == "csv"][0]
        self.assertEqual(csv["id"], "aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee")
        self.assertIsNone(csv.get("canonicalId"))
        homemade = [
            a
            for a in snap
            if a["id"] == "cccccccc-dddd-eeee-ffff-000000000000"
        ][0]
        self.assertEqual(
            homemade["canonicalId"],
            "2024-06-16|acct-1|AAA|10|10|-100|Trade|BUY",
        )


def time_now_minus():
    return datetime.now(timezone.utc).timestamp() - 10


if __name__ == "__main__":
    unittest.main()
