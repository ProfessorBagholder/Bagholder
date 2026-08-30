#!/usr/bin/env python3
"""SQLite store and Wealthsimple sync bounds. Synthetic rows only."""

from __future__ import annotations

import os
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

    def test_json_import_once_then_daily_is_incremental(self):
        payload = {
            "activities": [
                bagholder.map_activity(_ws_item()),
                {
                    "id": "manual|2024-07-01|manual|ZZZ|BUY|1|2",
                    "transactionDate": "2024-07-01",
                    "accountId": "manual",
                    "symbol": "ZZZ",
                    "quantity": 1,
                    "unitPrice": 2,
                    "netCashAmount": -2,
                    "activityType": "Trade",
                    "activitySubType": "BUY",
                    "source": "manual",
                },
            ],
            "accounts": [],
            "balances": [],
            "syncedAt": "2024-07-03T00:00:00Z",
        }
        json_file = store.json_path()
        json_file.write_text(__import__("json").dumps(payload), encoding="utf-8")
        # new empty db dir already has schema; drop rows to simulate first open
        # import runs only when the table is empty — already true before apply
        self.assertEqual(store.activity_count(), 0)
        self.assertTrue(store.import_json_if_needed())
        self.assertEqual(store.activity_count(), 2)
        ws = [a for a in store.snapshot()["activities"] if a["source"] == "wealthsimple"][0]
        self.assertEqual(ws["canonicalId"], "ws-cid-aaa-001")
        self.assertFalse(store.looks_like_homemade_id(ws["id"]))
        manual = [a for a in store.snapshot()["activities"] if a["source"] == "manual"][0]
        self.assertIsNone(manual.get("canonicalId"))
        bounds = bagholder.activity_sync_bounds()
        self.assertFalse(bounds["full_history"])

    def test_token_refresh_needed_uses_expires_at(self):
        now = 1_700_000_000
        sess = {"expires_at": now + 60, "refresh_token": "r"}
        self.assertTrue(bagholder.token_refresh_needed(sess, now=now))
        sess_ok = {"expires_at": now + 3600, "refresh_token": "r"}
        self.assertFalse(bagholder.token_refresh_needed(sess_ok, now=now))

    def test_ensure_fresh_token_does_not_pull_activity(self):
        called = {"graphql": 0, "token": 0}

        def fake_refresh(sess):
            called["token"] += 1
            return True

        def fake_graphql(*args, **kwargs):
            called["graphql"] += 1
            return {}

        sess = {
            "refresh_token": "r",
            "expires_at": time_now_minus(),
        }
        bagholder.save_session(sess)
        with mock.patch.object(bagholder, "refresh_session", side_effect=fake_refresh):
            with mock.patch.object(bagholder, "graphql", side_effect=fake_graphql):
                bagholder.ensure_fresh_token(sess)
        self.assertEqual(called["token"], 1)
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


if __name__ == "__main__":
    unittest.main()
