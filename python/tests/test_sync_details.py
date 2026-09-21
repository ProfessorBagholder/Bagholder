"""Synthetic web-order fixtures. No CSV or customer data is used by sync."""
import copy
import json
import tempfile
import unittest
from contextlib import ExitStack
from unittest import mock

import bagholder as app
import model
import store
import sync_details as details


def item(**changes):
    row = dict(canonicalId='summary-one', externalCanonicalId='batch-one',
        accountId='account-one', status='POSTED', occurredAt='2026-09-09T14:01:00Z',
        type='OPTIONS_MULTILEG', subType='BUY', assetSymbol='XYZ', assetQuantity=0,
        amount=100, amountSign='negative', currency='USD')
    row.update(changes)
    return row


def security(ident='sec-call', right='CALL'):
    return dict(id=ident, currency='USD', stock=dict(symbol='XYZ'),
        optionDetails=dict(multiplier=100, optionType=right, strikePrice='100',
            expiryDate='2026-09-09', underlyingSecurity=dict(stock=dict(symbol='XYZ'))))


def leg(ident='leg-call', sid='sec-call', side='BUY', intent='OPEN', price='1.00', net='500.00', when='2026-09-09T14:01:00Z'):
    return dict(orderId=ident, securityId=sid, side=side, openClose=intent,
        status='FILLED', filledQuantity='5', averageFillPrice=dict(amount=price, currency='USD'),
        filledNetValue=net, orderCurrency='USD', lastFilledAtUtc=when)


def option_detail():
    return dict(orderBatchId='batch-one', status='FILLED', totalFee='0',
        legs=[leg(), leg('leg-put', 'sec-put', price='.50', net='250')])


def crypto_item(typ='CRYPTO_SELL'):
    return item(type=typ, subType='SWAP', assetSymbol='DOGE', counterAssetSymbol='BTC',
                currency='CAD', assetQuantity='.005', amount=990, securityId='doge-security')


def crypto_detail(**changes):
    data = dict(id='batch-one', status='filled', currency='CAD',
        filledAt='2026-09-09T14:01:00Z', executedQuantity='1000', executedValue='.005',
        swapFee='10', fee='10', totalCost='990')
    data.update(changes)
    return data


class DetailMappingTest(unittest.TestCase):
    def query(self, option=None, crypto=None):
        def run(op, variables, query):
            if op == 'DetailSecurities':
                return dict(securities=[security(), security('sec-put', 'PUT')])
            if op == 'FetchSoOrdersMultilegOrder':
                self.assertEqual(variables['orderBatchId'], 'batch-one')
                return dict(soOrdersMultilegOrder=option or option_detail())
            return dict(cryptoOrder=crypto or crypto_detail())
        return mock.Mock(side_effect=run)

    def enrich(self, rows, query=None, cache=None):
        return details.enrich(rows, {}, app.map_activity_rows, query or self.query(), cache if cache is not None else {})

    def test_straddle_recovers_both_contracts_not_summary_cash(self):
        ordinary, groups, missing = self.enrich([item()])
        self.assertEqual((ordinary, missing), ([], 0))
        parent, legs = groups[0]
        self.assertEqual(parent['category'], 'other')
        self.assertEqual({r['symbol'] for r in legs}, {'XYZ 09SEP26 100.00 CALL', 'XYZ 09SEP26 100.00 PUT'})
        self.assertEqual(sum(r['netCashAmount'] for r in legs), -750)
        self.assertEqual([r['activitySubType'] for r in legs], ['BUYTOOPEN'] * 2)
        self.assertEqual([r['unitPrice'] for r in legs], [1, .5])

    def test_sell_to_open_and_buy_to_close_keep_explicit_direction(self):
        data = option_detail()
        data['legs'] = [leg(side='SELL', price='.84', net='420')]
        first = self.enrich([item()], self.query(option=data))[1][0][1][0]
        second = dict(first, id='close', canonicalId='close', occurredAt='2026-09-09T14:08:00Z',
            activitySubType='BUYTOCLOSE', quantity=5, netCashAmount=-435, unitPrice=.87)
        first['id'] = 'open'
        base = model.build_base({'activities': [first, second]}, {}, {}, today='2026-09-10')
        self.assertEqual(len(base['closed']), 1)
        self.assertAlmostEqual(base['closed'][0]['pnl'], -15)
        self.assertFalse(base['openLots'])

    def test_credit_spread_and_fees_match_leg_cash(self):
        data = option_detail()
        data['totalFee'] = '2'
        data['legs'] = [leg(side='SELL', price='2', net='999'), leg('leg-put', 'sec-put', price='1', net='501')]
        legs = self.enrich([item()], self.query(option=data))[1][0][1]
        self.assertEqual([r['quantity'] for r in legs], [-5, 5])
        self.assertEqual(sum(r['netCashAmount'] for r in legs), 498)
        self.assertEqual([r['commission'] for r in legs], [1, 1])

    def test_incomplete_or_inconsistent_option_order_never_emits_partial_legs(self):
        for mutate in (
            lambda d: d['legs'][1].update(status='PENDING'),
            lambda d: d['legs'][1].update(securityId='unknown'),
            lambda d: d['legs'][1].update(filledNetValue='999'),
            lambda d: d['legs'][1].update(orderId='leg-call'),
            lambda d: d['legs'][1].update(orderCurrency='CAD'),
            lambda d: d.update(orderBatchId='wrong-order'),
        ):
            data = option_detail(); mutate(data)
            with self.subTest(data=data):
                ordinary, groups, pending = self.enrich([item()], self.query(option=data))
                self.assertEqual(pending, 1)
                self.assertEqual(groups[0][1], [])
                a = model.normalize_activity(groups[0][0])
                self.assertEqual(a['flags'], ['missing-option-legs'])
                self.assertEqual(a['category'], 'other')

    def test_crypto_sell_includes_fee_units_once_and_both_legs_net_to_zero(self):
        parent, legs = self.enrich([crypto_item()])[1][0]
        self.assertEqual([r['symbol'] for r in legs], ['DOGE', 'BTC'])
        self.assertEqual([r['quantity'] for r in legs], [-1010, .005])
        self.assertEqual([r['netCashAmount'] for r in legs], [990, -990])
        self.assertEqual([r['commission'] for r in legs], [10, 0])
        self.assertAlmostEqual(legs[0]['unitPrice'] * 1010 - legs[0]['commission'], 990)
        self.assertEqual(model.normalize_activity(legs[1])['kind'], 'Crypto')
        self.assertEqual(model.normalize_activity(parent)['category'], 'other')

    def test_crypto_buy_still_spends_feed_asset_and_acquires_counter_asset(self):
        data = crypto_detail(executedQuantity='.005', executedValue='1000', totalCost='1000')
        legs = self.enrich([crypto_item('CRYPTO_BUY')], self.query(crypto=data))[1][0][1]
        self.assertEqual([(r['symbol'], r['quantity'], r['netCashAmount']) for r in legs],
            [('DOGE', -1010, 990), ('BTC', .005, -990)])

    def test_crypto_sale_profit_charges_fee_only_once(self):
        parent, legs = self.enrich([crypto_item()])[1][0]
        purchase = dict(legs[0], id='purchase', occurredAt='2026-09-08T14:00:00Z',
            transactionDate='2026-09-08', quantity=1010, unitPrice=.5, netCashAmount=-505,
            commission=0, activitySubType='BUY')
        acts = [purchase] + [dict(r, id=r['canonicalId']) for r in legs + [parent]]
        base = model.build_base({'activities': acts}, {}, {}, today='2026-09-10')
        self.assertAlmostEqual(sum(r['pnl'] for r in base['closed']), 485)
        self.assertAlmostEqual(sum(r['qty'] for r in base['openLots']), .005)

    def test_unknown_detail_is_retried_but_success_is_cached(self):
        cache = {}; query = self.query()
        self.enrich([item()], query, cache)
        count = query.call_count
        self.enrich([item()], query, cache)
        self.assertEqual(query.call_count, count)
        self.enrich([item(amount=102)], query, cache)
        self.assertGreater(query.call_count, count)
        empty = {}; failure = mock.Mock(side_effect=RuntimeError('unavailable'))
        self.assertEqual(self.enrich([item()], failure, empty)[2], 1)
        self.assertFalse(empty['orders'])
        self.assertEqual(self.enrich([item()], self.query(), empty)[2], 0)

    def test_auth_error_is_not_swallowed(self):
        with self.assertRaises(PermissionError):
            self.enrich([item()], mock.Mock(side_effect=PermissionError()))

    def test_missing_swap_details_are_excluded_and_reported(self):
        ordinary, groups, pending = self.enrich([crypto_item()], mock.Mock(side_effect=RuntimeError()))
        self.assertEqual(pending, 1)
        parent = dict(groups[0][0], id='missing')
        base = model.build_base({'activities': [parent]}, {}, {}, today='2026-09-10')
        self.assertFalse(base['closed'])
        self.assertFalse(base['openLots'])
        self.assertEqual(base['unmatched'][0]['reason'], 'missing-swap-legs')

    def test_nonfinite_and_missing_swap_amounts_are_excluded(self):
        for changes in ({'executedQuantity': None}, {'executedValue': 'NaN'}, {'swapFee': '-1'}, {'id': 'wrong'}):
            self.assertEqual(self.enrich([crypto_item()], self.query(crypto=crypto_detail(**changes)))[2], 1)

    def test_cancelled_orders_and_ordinary_trades_need_no_detail_requests(self):
        query = self.query()
        ordinary, groups, pending = self.enrich([item(status='CANCELLED'), item(type='DIY_BUY', assetQuantity=5)], query)
        self.assertEqual(len(ordinary), 1)
        self.assertFalse(groups)
        query.assert_not_called()


class DetailStorageTest(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        app.set_home(self.tmp.name)
        store.set_home(self.tmp.name)
        store.ensure()
        self.raw = item()
        query = DetailMappingTest().query()
        _, self.groups, _ = details.enrich([self.raw], {}, app.map_activity_rows, query, {})

    def tearDown(self):
        self.tmp.cleanup()

    def test_repeated_sync_preserves_ids_and_cannot_double_count_summary(self):
        store.apply_wealthsimple_mapped([app.map_activity(self.raw)])
        parent_id = store.snapshot()['activities'][0]['id']
        store.apply_wealthsimple_detail_groups(self.groups)
        first = store.snapshot()['activities']
        store.apply_wealthsimple_detail_groups(self.groups)
        self.assertEqual(store.snapshot()['activities'], first)
        self.assertEqual(len(first), 3)
        self.assertEqual(next(r['id'] for r in first if r['canonicalId']=='summary-one'), parent_id)
        store.apply_wealthsimple_mapped([dict(self.groups[0][1][0], quantity=999, rawType='OPTIONS_BUY')])
        self.assertEqual(store.snapshot()['activities'], first)
        self.assertAlmostEqual(sum(r['netCashAmount'] for r in first if r['category']=='trade'), -750)

    def test_revised_or_unavailable_group_removes_obsolete_children(self):
        store.apply_wealthsimple_detail_groups(self.groups)
        parent, legs = copy.deepcopy(self.groups[0])
        store.apply_wealthsimple_detail_groups([(parent, legs[:1])])
        self.assertEqual(store.activity_count(), 2)
        parent.update(rawType='WS_DETAIL_MISSING_OPTION')
        store.apply_wealthsimple_detail_groups([(parent, [])])
        self.assertEqual(store.activity_count(), 1)
        self.assertEqual(store.snapshot()['activities'][0]['rawType'], 'WS_DETAIL_MISSING_OPTION')
        store.apply_wealthsimple_detail_groups(self.groups)
        self.assertEqual(store.activity_count(), 3)

    def test_failed_group_transaction_rolls_back(self):
        store.apply_wealthsimple_detail_groups(self.groups)
        first = store.snapshot()['activities']
        parent, legs = copy.deepcopy(self.groups[0])
        legs[0]['quantity'] = 99
        invalid = (dict(parent, canonicalId='another'), [{'canonicalId': 'another'}])
        with self.assertRaises(ValueError):
            store.apply_wealthsimple_detail_groups([(parent, legs), invalid])
        self.assertEqual(store.snapshot()['activities'], first)

    def test_upgrade_backfills_without_clear_then_returns_to_incremental(self):
        store.apply_wealthsimple_mapped([app.map_activity(self.raw)])
        self.assertTrue(app.activity_sync_bounds()['full_history'])
        store.set_meta(details.VERSION_KEY, details.VERSION)
        store.set_meta(app.ws_reconcile.VERSION_KEY, app.ws_reconcile.VERSION)
        self.assertFalse(app.activity_sync_bounds()['full_history'])
        store.set_meta(details.CACHE_KEY, '{"orders": {}}')
        store.clear_synced_data()
        self.assertEqual(store.get_meta(details.CACHE_KEY), '')
        self.assertEqual(store.get_meta(details.VERSION_KEY), '')
        self.assertTrue(app.activity_sync_bounds()['full_history'])


    def test_incoming_coin_listing_reuses_ws_identity_without_rewalking_feed(self):
        query = DetailMappingTest().query()
        _, groups, _ = details.enrich([crypto_item()], {}, app.map_activity_rows, query, {})
        store.apply_wealthsimple_detail_groups(groups)
        # A synthetic incoming leg has no matching feed canonical ID. Rewalking
        # all accounts cannot fill it, even if the coin's ID remains unknown.
        self.assertFalse(store.needs_security_id_backfill())
        purchase = app.map_activity(item(canonicalId='btc-buy', type='CRYPTO_BUY',
            subType='BUY', assetSymbol='BTC', assetQuantity='.01', securityId='btc-security'))
        store.apply_wealthsimple_mapped([purchase])
        store.fill_crypto_detail_security_ids()
        incoming = next(r for r in store.snapshot()['activities'] if r['canonicalId'].endswith(':swap:in'))
        self.assertEqual(incoming['securityId'], 'btc-security')
        self.assertFalse(store.needs_security_id_backfill())
        # An ambiguous ticker must not be assigned another instrument's ID.
        store.apply_wealthsimple_mapped([dict(purchase, canonicalId='btc-other', securityId='different-security')])
        store.apply_wealthsimple_detail_groups(groups)
        store.fill_crypto_detail_security_ids()
        incoming = next(r for r in store.snapshot()['activities'] if r['canonicalId'].endswith(':swap:in'))
        self.assertFalse(incoming.get('securityId'))

    def test_run_sync_fetches_details_uses_cache_and_never_imports_csv(self):
        query = DetailMappingTest().query()
        with app._lock:
            app._state['syncing'] = False
        with ExitStack() as stack:
            replacements = {
                'load_session': {'access_token': 'fake', 'identity_canonical_id': 'fake-identity'},
                'save_session': None, 'fetch_all_accounts': [{'id': 'account-one'}],
                'fetch_activities_for_account': [self.raw], 'fetch_balances': [],
                'fetch_margin': [], 'fetch_nav_history': [], 'fetch_nickname_nav_history': ([], []),
                'fill_listings': None,
            }
            for name, result in replacements.items():
                stack.enter_context(mock.patch.object(app, name, return_value=result))
            stack.enter_context(mock.patch.object(app.ws_reconcile, 'fetch_report', return_value={'status':'ok','total':0,'securities':[]}))
            stack.enter_context(mock.patch.object(app, 'graphql', side_effect=lambda sess, op, v, q: query(op, v, q)))
            self.assertTrue(app.run_sync())
            self.assertEqual(store.activity_count(), 3)
            self.assertEqual(store.get_meta(details.VERSION_KEY), details.VERSION)
            count = query.call_count
            self.assertTrue(app.run_sync())
            self.assertEqual(query.call_count, count)
            self.assertEqual(store.activity_count(), 3)
            self.assertTrue(all(a['source']=='wealthsimple' for a in store.snapshot()['activities']))



if __name__ == '__main__':
    unittest.main()
