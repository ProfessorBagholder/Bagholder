"""Regression tests for porting data corrections onto upstream's caches and API."""
import copy
import tempfile
import unittest
from unittest import mock

import model
import store
from tests.test_export_reconciliation import event, parse
from tests import test_sync_details as fixtures
import bagholder
import sync_details


class DataPortIntegrationTest(unittest.TestCase):
    def setUp(self):
        self.previous = store.home()
        self.tmp = tempfile.TemporaryDirectory()
        store.set_home(self.tmp.name)
        store.ensure()
        model.invalidate(book=True)

    def tearDown(self):
        model.invalidate(book=True)
        store.set_home(self.previous)
        self.tmp.cleanup()

    def test_reimport_same_count_and_dates_rebuilds_cached_book(self):
        rows = parse(event('AAA', 10))
        windows = [dict(accountId='A', first='2026-01-01', last='2026-01-01')]
        store.reconcile_export(rows, windows)
        self.assertEqual(model.base_model()['positions'][0]['qty'], 10)
        store.reconcile_export([dict(rows[0], quantity=20)], windows)
        self.assertEqual(model.base_model()['positions'][0]['qty'], 20)

    def test_revised_order_same_ids_and_times_rebuilds_cached_book(self):
        _, groups, _ = sync_details.enrich([fixtures.item()], {}, bagholder.map_activity_rows, fixtures.DetailMappingTest().query(), {})
        store.apply_wealthsimple_detail_groups(groups)
        with mock.patch.object(model, 'today_local', return_value='2026-09-09'):
            first = model.base_model()
            original_cost = sum(p['cost'] for p in first['positions'])
            revised = copy.deepcopy(groups)
            revised[0][1][0]['unitPrice'] *= 2
            revised[0][1][0]['netCashAmount'] *= 2
            store.apply_wealthsimple_detail_groups(revised)
            self.assertGreater(sum(p['cost'] for p in model.base_model()['positions']), original_cost)

    def test_quote_refresh_keeps_closed_broker_inventory_hidden(self):
        row = parse(event('AAA', 10))[0]
        row.update(source='wealthsimple', accountType='TFSA', securityId='sec-aaa')
        store.insert_activity(row)
        store.replace_accounts([dict(id='A', nickname='TFSA')])
        store.replace_balances([])
        store.set_meta('balances_read_at', '2026-01-02T00:00:00Z')
        self.assertEqual(model.base_model()['positions'], [])
        with mock.patch.object(store, 'market_data', return_value={'quotes': {'AAA': {'price': 20}}}):
            first = model._cache['base']
            remarked = model._remark(first, model._cache['inputs'], first['today'], 'new', 'core')
        self.assertEqual(remarked['positions'], [])

    def test_revised_fx_rebuilds_dlr_carrying_cost(self):
        rows = parse(event('DLR', 10), event('DLR.U', 10, '2026-01-02',
            activity_type='ListingSwap', currency='USD', unit_price='', net_cash_amount=''))
        store.reconcile_export(rows, [dict(accountId='A', first='2026-01-01', last='2026-01-02')])
        store.upsert_fx_rates({'2026-01-02': 1.25})
        self.assertAlmostEqual(model.base_model()['positions'][0]['cost'], 80)
        conn = store._connect()
        try:
            conn.execute("UPDATE fx_rates SET rate=2 WHERE date='2026-01-02'")
            conn.commit()
        finally:
            conn.close()
        self.assertAlmostEqual(model.base_model()['positions'][0]['cost'], 50)

    def test_trade_details_withhold_unknown_basis(self):
        rows = parse(event('AAA', 10, activity_type='SecurityTransfer', unit_price='', net_cash_amount=''),
                     event('AAA', -10, '2026-01-02', unit_price='20', net_cash_amount='200'))
        base = model.build_base({'activities': rows}, {}, {}, '2026-01-03')
        trade = model.build_view(base)['trades'][0]
        self.assertIsNone(trade['pnl'])
        detail = model.trade_detail(trade['id'], base)
        self.assertTrue(all(leg['pnl'] is None for leg in detail['legs']))

    def test_unknown_crypto_deposit_stays_visible_without_zero_cost_claim(self):
        from tests.test_model import act
        row = act(id='deposit', rawType='CRYPTO_TRANSFER', activityType='CRYPTO_TRANSFER',
            activitySubType='TRANSFER_IN', symbol='ETH', currency='CAD', quantity=2,
            netCashAmount=200, unitPrice=100, transactionDate='2026-01-01')
        base = model.build_base({'activities': [row]}, {'quotes': {'ETH': {'price': 150}}}, {}, '2026-01-02')
        view = model.build_view(base)
        position = view['positions'][0]
        self.assertEqual((position['qty'], position['mv']), (2, 300))
        for field in ('cost', 'avg', 'unreal', 'unrealPct'):
            self.assertIsNone(position[field])
        self.assertIsNone(view['positionsSummary']['book'])
        self.assertIsNone(view['portfolio']['costBasis'])

    def test_statement_audit_survives_reusing_matched_book(self):
        from tests import test_statement_reconcile as fixture
        for row in fixture.history():
            store.insert_activity(row, canonical_id=row.get('canonicalId'))
        store.save_statement_corrections([fixture.rule()])
        first = model.base_model()
        self.assertEqual(first['statementCorrections'][0]['status'], 'applied')
        # A broker report refresh changes the core view but not inventory matching.
        store.set_meta('ws_realized_report_v1', '{"status":"ok","total":95}')
        with mock.patch.object(model, 'build_book', side_effect=AssertionError('must reuse book')):
            refreshed = model.base_model()
        self.assertEqual(refreshed['statementCorrections'], first['statementCorrections'])

    def test_broker_only_share_does_not_take_a_crypto_quote(self):
        snapshot = dict(accounts=[dict(id='A', nickname='Shares')],
            balances=[dict(accountId='A', securityId='share', quantity=3)],
            balancesReadAt='2026-01-01',
            securities=[dict(id='share', symbol='BTC', currency='CAD', primaryExchange='TSX')])
        base = model.build_base(snapshot, {'quotes': {'BTC': {'price': 100000, 'source': 'coinbase'}}}, {}, '2026-01-01')
        self.assertEqual(base['positions'][0]['priceSource'], 'unavailable')
        self.assertNotEqual(base['positions'][0]['mv'], 300000)
