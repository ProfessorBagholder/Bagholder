import csv
import io
import tempfile
import unittest

import csvimport
import export_history
import model
import store


def event(symbol, quantity, day="2026-01-01", **kw):
    row = dict(effective_date=day, effective_time="10:00:00", account_id="A",
               account_type="TFSA", activity_type="Trade", activity_sub_type="BUY" if quantity > 0 else "SELL",
               symbol=symbol, quantity=str(quantity), currency="CAD", direction="LONG",
               unit_price="10", net_cash_amount=str(-quantity * 10))
    row.update(kw)
    return row


def parse(*rows):
    stream = io.StringIO()
    writer = csv.DictWriter(stream, fieldnames=sorted({k for r in rows for k in r}))
    writer.writeheader()
    writer.writerows(rows)
    result = csvimport.parse_csv(stream.getvalue())
    assert not result["skipped"], result["skipped"]
    return result["activities"]


def change(symbol, quantity, day, **kw):
    return event(symbol, quantity, day, activity_type="LegacyCorporateAction",
                 activity_sub_type="NAME_CHANGE", unit_price="", net_cash_amount="", **kw)


class ExportInventoryTest(unittest.TestCase):
    def test_new_header_contract_units_and_short_expiry(self):
        rows = parse(event("ABC   260116C00020000", -2, currency="USD", direction="SHORT", unit_price="150", net_cash_amount="300"),
                     event("ABC   260116C00020000", 2, "2026-01-16", currency="", direction="SHORT", activity_type="OptionExpiry", unit_price="", net_cash_amount=""))
        self.assertEqual(rows[0]["symbol"], "ABC 16JAN26 20.00 CALL")
        self.assertEqual(rows[0]["unitPrice"], 1.5)
        self.assertEqual(rows[1]["currency"], "USD")
        result = model.match_fifo(rows)
        self.assertEqual(result["open"], [])
        self.assertEqual(result["unmatched"], [])
        self.assertAlmostEqual(result["closed"][0]["pnl"], 300)

    def test_chained_renames_preserve_basis_and_only_final_sale_realizes(self):
        rows = parse(event("OLD", 10), change("OLD", -10, "2026-01-02"), change("MID", 10, "2026-01-02"),
                     change("MID", -10, "2026-01-03"), change("NEW", 10, "2026-01-03"),
                     event("NEW", -10, "2026-01-04", unit_price="15", net_cash_amount="150"))
        result = model.match_fifo(rows)
        self.assertFalse(result["open"])
        self.assertFalse(result["unmatched"])
        self.assertEqual(len(result["closed"]), 1)
        self.assertAlmostEqual(result["closed"][0]["pnl"], 50)

    def test_reusing_old_ticker_later_is_not_permanent_alias(self):
        rows = parse(event("OLD", 10), change("OLD", -10, "2026-01-02"), change("NEW", 10, "2026-01-02"), event("OLD", 5, "2026-01-03"))
        positions = model.match_fifo(rows)["open"]
        self.assertEqual({p["symbol"]: p["qty"] for p in positions}, {"NEW": 10, "OLD": 5})

    def test_unrelated_renames_same_day_do_not_cross_match(self):
        rows = parse(event("AAA", 10), event("BBB", 10), change("AAA", -10, "2026-01-02"), change("BBB", -10, "2026-01-02"), change("CCC", 10, "2026-01-02"), change("DDD", 10, "2026-01-02"))
        result = model.match_fifo(rows)
        self.assertTrue(result["unmatched"])
        self.assertTrue(all("missing-transfer-basis" in p["flags"] for p in result["open"]))
        self.assertEqual(result["closed"], [])

    def test_dlr_fallback_both_directions_and_either_reported_leg(self):
        for source, destination, currency, dest_currency in (("DLR", "DLR.U", "CAD", "USD"), ("DLR.U", "DLR", "USD", "CAD")):
            for incoming in (True, False):
                with self.subTest(source=source, incoming=incoming):
                    move = event(destination if incoming else source, 10 if incoming else -10, "2026-01-02", currency=dest_currency if incoming else currency, activity_type="ListingSwap", activity_sub_type="-", unit_price="", net_cash_amount="")
                    result = model.match_fifo(parse(event(source, 10, currency=currency), move), {"2026-01-02": 1.4})
                    self.assertFalse(result["closed"])
                    self.assertFalse(result["unmatched"])
                    self.assertEqual(result["open"][0]["symbol"], destination)
                    self.assertAlmostEqual(result["open"][0]["price"], 10 / 1.4 if currency == "CAD" else 14)

    def test_explicit_dlr_pair_is_not_synthesized_twice(self):
        rows = parse(event("DLR", 10), event("DLR", -10, "2026-01-02", activity_type="ListingSwap", currency="", unit_price="", net_cash_amount=""), event("DLR.U", 10, "2026-01-02", activity_type="ListingSwap", currency="", unit_price="", net_cash_amount=""))
        result = model.match_fifo(rows, {"2026-01-02": 1.4})
        self.assertEqual(sum(p["qty"] for p in result["open"]), 10)
        self.assertFalse(result["unmatched"])

    def test_synced_dlr_journals_work_in_both_directions(self):
        for source, destination in (("DLR", "DLR.U"), ("DLR.U", "DLR")):
            rows = parse(event(source, 10, currency="USD" if source == "DLR.U" else "CAD"),
                         event(source, 10, "2026-01-02", activity_type="JOURNAL_SHARES", unit_price="", net_cash_amount=""))
            for row in rows:
                row["source"] = "wealthsimple"
            result = model.match_fifo(rows, {"2026-01-02": 1.4})
            self.assertEqual([(p["symbol"], p["qty"]) for p in result["open"]], [(destination, 10)])
            self.assertFalse(result["closed"])

    def test_crypto_swaps_use_one_coin_inventory_across_currencies(self):
        rows = parse(event("BTC", 1, account_type="Crypto", currency="CAD", net_cash_amount="-100"),
                     event("BTC", 1, "2026-01-02", account_type="Crypto", activity_type="CryptoSwap", currency="USD", net_cash_amount="100"))
        base = model.build_base({"activities": rows}, {"fx": {"2026-01-02": 1.4}}, {}, "2026-01-03")
        self.assertEqual(len(base["positions"]), 1)
        self.assertEqual(base["positions"][0]["qty"], 2)
        self.assertAlmostEqual(base["positions"][0]["cost"], 240)

    def test_crypto_swap_has_two_coin_legs_and_no_external_cashflow(self):
        rows = parse(event("DOGE", 1000, account_type="Crypto", currency="CAD", net_cash_amount="-100"),
            event("DOGE", -1000, "2026-01-02", account_type="Crypto", activity_type="CryptoSwap", currency="USD", net_cash_amount="-200"),
            event("BTC", .002, "2026-01-02", account_type="Crypto", activity_type="CryptoSwap", currency="USD", net_cash_amount="200"))
        normalized = model.normalize_activities(model.export_currencies(rows, {"2026-01-02": 1.4}))
        swap = normalized[1:]
        self.assertAlmostEqual(sum(a["netCashAmount"] for a in swap), 0)
        result = model.match_fifo(normalized)
        self.assertEqual(len(result["closed"]), 1)
        sale = result["closed"][0]
        self.assertEqual((sale["symbol"], sale["quantity"]), ("DOGE", 1000))
        self.assertAlmostEqual(sale["exitPrice"], .28)
        self.assertAlmostEqual(sale["pnl"], 180)
        self.assertEqual([(p["symbol"], p["qty"]) for p in result["open"]], [("BTC", .002)])
        self.assertAlmostEqual(result["open"][0]["price"] * .002, 280)
        base = model.build_base({"activities": rows}, {"fx": {"2026-01-02": 1.4}}, {}, "2026-01-03")
        self.assertFalse(base["cashflow"])

    def test_incomplete_feed_swap_cannot_be_a_doge_sale_at_btc_price(self):
        row = dict(id="swap", source="wealthsimple", rawType="CRYPTO_SELL", activityType="CRYPTO_SELL",
            activitySubType="SWAP_MARKET_ORDER", symbol="DOGE", counterSymbol="BTC", quantity=.055608,
            unitPrice=134038.99, netCashAmount=7453.64, currency="CAD", transactionDate="2026-01-15",
            accountId="crypto", accountType="Crypto")
        result = model.match_fifo([row])
        self.assertFalse(result["closed"])
        self.assertFalse(result["open"])
        self.assertEqual(result["unmatched"][0]["reason"], "missing-swap-legs")
        self.assertIsNone(result["unmatched"][0]["price"])

    def test_sale_of_transfer_without_basis_is_not_reported_as_pure_profit(self):
        rows = parse(event("DOGE", 100, account_type="Crypto", activity_type="SecurityTransfer", net_cash_amount="20"),
                     event("DOGE", -100, "2026-01-02", account_type="Crypto", net_cash_amount="30"))
        base = model.build_base({"activities": rows}, {}, {}, "2026-01-03")
        view = model.build_view(base)
        self.assertEqual(view["incompleteTradeCount"], 1)
        self.assertIsNone(view["trades"][0]["pnl"])
        self.assertIsNone(view["trades"][0]["entry"])
        self.assertEqual(view["kpi"]["count"], 0)
        self.assertEqual(view["monthly"], [])
        self.assertEqual(view["bySymbol"], [])
        self.assertIsNotNone(base["trades"][0]["pnl"])

    def test_unpaired_transfer_exposes_unknown_basis(self):
        rows = parse(event("AAA", 10, activity_type="InternalSecurityTransfer", net_cash_amount="999"))
        base = model.build_base({"activities": rows}, {}, {}, "2026-01-03")
        self.assertIn("missing-transfer-basis", base["positions"][0]["basisWarnings"])
        self.assertTrue(base["unmatched"])

    def test_transfer_preserves_cost_and_account_separation(self):
        rows = parse(event("AAA", 10), event("AAA", -6, "2026-01-02", activity_type="InternalSecurityTransfer", net_cash_amount="-999"), event("AAA", 6, "2026-01-02", account_id="B", activity_type="InternalSecurityTransfer", net_cash_amount="999"))
        result = model.match_fifo(rows)
        self.assertEqual({p["accountId"]: p["qty"] for p in result["open"]}, {"A": 4, "B": 6})
        self.assertTrue(all(p["price"] == 10 for p in result["open"]))
        self.assertEqual(len({p["rt"] for p in result["open"]}), 2)
        self.assertFalse(result["closed"])

    def test_split_delta_and_two_leg_reverse_split_preserve_total_basis(self):
        for changes, expected in (([event("AAA", 30, "2026-01-02", activity_type="CorporateAction", activity_sub_type="SUBDIVISION")], 40),
                                  ([event("AAA", -10, "2026-01-02", activity_type="CorporateAction", activity_sub_type="CONSOLIDATION"), event("AAA", 2, "2026-01-02", activity_type="CorporateAction", activity_sub_type="CONSOLIDATION")], 2)):
            result = model.match_fifo(parse(event("AAA", 10), *changes))
            self.assertFalse(result["unmatched"])
            self.assertAlmostEqual(sum(p["qty"] for p in result["open"]), expected)
            self.assertAlmostEqual(sum(p["qty"] * p["price"] for p in result["open"]), 100)

    def test_exercise_share_leg_is_a_real_purchase(self):
        rows = parse(event("AAA", 100, activity_type="OptionExercise"), event("AAA", -100, "2026-01-02", unit_price="12", net_cash_amount="1200"))
        result = model.match_fifo(rows)
        self.assertFalse(result["unmatched"])
        self.assertAlmostEqual(result["closed"][0]["pnl"], 200)

    def test_export_inventory_is_not_replaced_with_stale_balance(self):
        rows = parse(event("AAA", 500))
        base = model.build_base({"activities": rows, "accounts": [{"id": "A", "nickname": "TFSA (A)"}], "balances": [{"accountId": "A", "securityId": "s", "quantity": 100}], "securities": [{"id": "s", "symbol": "AAA"}], "balancesReadAt": "2026-01-03"}, {}, {}, "2026-01-03")
        position = base["positions"][0]
        self.assertEqual(position["qty"], 500)
        self.assertTrue(position["balanceMismatch"])


class ExportStoreTest(unittest.TestCase):
    def setUp(self):
        self.previous = store.home()
        self.tmp = tempfile.TemporaryDirectory()
        store.set_home(self.tmp.name)
        store.ensure()

    def tearDown(self):
        store.set_home(self.previous)
        self.tmp.cleanup()

    def test_mapping_windows_and_reimport_do_not_duplicate_or_delete_feed(self):
        store.replace_accounts([{"id": "real", "nickname": "Investments"}])
        store.replace_balances([{"accountId": "real", "custodianAccountId": "A", "securityId": "s", "quantity": 10}])
        for day in ("2026-01-01", "2026-01-03"):
            a = parse(event("AAA", 10, day))[0]
            a.update(source="wealthsimple", accountId="real")
            store.insert_activity(a, assigned_id="feed:" + day)
        raw = parse(event("AAA", 10), event("AAA", 10))
        prepared, windows = export_history.prepare(raw, store.snapshot())
        self.assertEqual(len({a["id"] for a in prepared}), 2)
        store.reconcile_export(prepared, windows)
        first = store.snapshot()["activities"]
        version = store.data_version()
        store.reconcile_export(prepared, windows)
        self.assertEqual(first, store.snapshot()["activities"])
        self.assertEqual(len(first), 3)
        conn = store._connect()
        try:
            self.assertEqual(conn.execute("SELECT COUNT(*) FROM activities WHERE source='wealthsimple'").fetchone()[0], 2)
        finally:
            conn.close()
        changed = [dict(a, quantity=20) for a in prepared]
        store.reconcile_export(changed, windows)
        self.assertNotEqual(version, store.data_version())

    def test_unknown_mapping_is_rejected_before_import(self):
        with self.assertRaisesRegex(ValueError, "unambiguously"):
            export_history.prepare(parse(event("AAA", 10)), {"accounts": [{"id": "other"}]})

    def test_plain_import_requires_explicit_reconciliation(self):
        result = csvimport.import_text("export.csv", "effective_date,activity_type,account_id,quantity,symbol\n2026-01-01,Trade,A,10,AAA")
        self.assertFalse(result["ok"])
        self.assertTrue(result["requiresReconciliation"])
        self.assertFalse(store.snapshot()["activities"])


if __name__ == "__main__":
    unittest.main()
