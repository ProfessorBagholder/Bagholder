"""Synthetic statement evidence: cost conservation and rebuild equivalence."""
import copy
import json
import tempfile
import unittest
from contextlib import ExitStack
from unittest import mock

import bagholder as app
import model
import statement_reconcile as sr
import store
import ws_reconcile as ws
from tests.test_model import act, buy, sell
from tests.test_sync_details import item
from tests.test_ws_reconcile import report_page

EVENT = "asset_movement_request_group-fixture"
ACCOUNTS = {"account-one": dict(id="account-one", nickname="First"),
            "account-two": dict(id="account-two", nickname="Second")}


def rule():
    return dict(event=EVENT, date="2026-09-09", sourceAccount="account-one", destinationAccount="account-two",
                currency="CAD", evidence="Synthetic monthly statement, page 2: 10 AAA shares transferred",
                assets=[dict(symbol="AAA", securityId="sec-one", quantity="10")])


def parents():
    return [act(id=aid, canonicalId=EVENT + "-" + aid, source="wealthsimple", accountId=aid,
                fifoId=aid, accountType=ACCOUNTS[aid]["nickname"], symbol="", currency="CAD",
                transactionDate="2026-09-09", rawType="ASSET_MOVEMENT", activitySubType=side,
                netCashAmount=cash) for aid, side, cash in
            (("account-one", "SOURCE", -9999), ("account-two", "DESTINATION", 9999))]


def history():
    return [buy("purchase", "AAA", 10, 20, "2026-09-01", commission=2, netCashAmount=-202,
                accountId="account-one", accountType="First", securityId="sec-one"), *parents(),
            sell("sale", "AAA", 10, 30, "2026-09-11", commission=3, netCashAmount=297,
                 accountId="account-two", accountType="Second", securityId="sec-one")]


def fifo(snapshot):
    return model.match_fifo(model.normalize_activities(snapshot["activities"]))


class CorrectionTest(unittest.TestCase):
    def test_carries_cost_date_and_fees_without_cash_or_duplicate_rows(self):
        before = dict(activities=history())
        original = copy.deepcopy(before)
        result = sr.apply(before, [rule()])
        self.assertEqual(before, original)
        self.assertEqual(result, sr.apply(result, [rule()]))
        self.assertEqual(sr.apply(result, [])['activities'], before['activities'])
        f = fifo(result)
        self.assertEqual(len(f["closed"]), 1)
        self.assertFalse(f["open"])
        self.assertAlmostEqual(f["closed"][0]["pnl"], 95)
        self.assertEqual(f["closed"][0]["entryDate"], "2026-09-01")
        self.assertEqual(f["closed"][0]["commission"], 5)
        self.assertFalse(model.inventory_warnings(result))
        moves = [a for a in result['activities'] if a.get('source') == sr.SOURCE]
        self.assertEqual(len(moves), 2)
        self.assertEqual(sum(a['quantity'] for a in moves), 0)
        self.assertTrue(all(a['netCashAmount'] == a['unitPrice'] == a['commission'] == 0 for a in moves))
        self.assertIn('STATEMENT-CONFIRMED', model._fill_row(moves[0])['sub'])

    def test_preserves_broker_fifo_pool_and_consolidation_cost(self):
        rows = history()
        rows[2]['fifoId'] = 'account-two-pool'
        rows[-1].update(quantity=1.25, unitPrice=30, netCashAmount=34.5, fifoId='account-two-pool')
        for qty, sid in ((-10, 'sec-one'), (1.25, 'sec-new')):
            rows.append(act(id='split:' + sid, canonicalId='split:' + sid, source='wealthsimple',
                accountId='account-two', fifoId='account-two-pool', accountType='Second', currency='CAD',
                symbol='AAA', securityId=sid, transactionDate='2026-09-10',
                activityType='CorporateAction', activitySubType='split', rawType='WS_DETAIL_INVENTORY', quantity=qty))
        result = sr.apply(dict(activities=rows), [rule()])
        incoming = next(a for a in result['activities'] if a.get('source') == sr.SOURCE and a['quantity'] > 0)
        self.assertEqual(incoming['fifoId'], 'account-two-pool')
        f = fifo(result)
        self.assertFalse(f['open'])
        self.assertAlmostEqual(sum(a['pnl'] for a in f['closed']), 34.5 - 202)
        self.assertFalse(any('missing-transfer-basis' in a['flags'] for a in f['closed']))

    def test_missing_source_cost_stays_unknown(self):
        result = sr.apply(dict(activities=history()[1:]), [rule()])
        self.assertTrue(any('missing-transfer-basis' in a['flags'] for a in fifo(result)['closed']))

    def test_no_guess_when_parent_is_missing_or_changed(self):
        for key, value in (('transactionDate', '2026-09-08'), ('currency', 'USD'), ('quantity', 10),
                           ('symbol', 'AAA'), ('securityId', 'sec-one'), ('accountId', 'other'),
                           ('activitySubType', 'DESTINATION'), ('source', 'ws-export')):
            with self.subTest(key=key):
                rows = parents()
                rows[0][key] = value
                result = sr.apply(dict(activities=rows), [rule()])
                self.assertEqual(result['activities'], rows)
                self.assertEqual(result['statementCorrections'][0]['status'], 'blocked')
                self.assertTrue(model.inventory_warnings(result))
        for rows, status in (([], 'pending'), (parents()[:1], 'blocked'), (parents()+parents()[:1], 'blocked')):
            result = sr.apply(dict(activities=rows), [rule()])
            self.assertEqual(result['activities'], rows)
            self.assertEqual(result['statementCorrections'][0]['status'], status)

    def test_broker_details_supersede_statement_even_when_only_one_leg_arrives(self):
        rows = parents()
        child = ws.movement(rows[0], 'web-leg', ACCOUNTS['account-one'], 'AAA', -10, 'CAD', EVENT,
                            'InternalSecurityTransfer', sid='sec-one')
        rows.append(dict(child, id='web-leg'))
        result = sr.apply(dict(activities=rows), [rule()])
        self.assertEqual(result['activities'], rows)
        self.assertEqual(result['statementCorrections'][0]['status'], 'broker-details')
        self.assertFalse(any(a.get('source') == sr.SOURCE for a in result['activities']))

    def test_validation_rejects_assumed_cost_invalid_quantities_and_duplicate_rules(self):
        bad = []
        for value in (0, -1, True, 'NaN', 'Infinity', '1e9999', '0.0', '1e-9999'):
            r = rule(); r['assets'][0]['quantity'] = value; bad.append([r])
        r = rule(); r['assets'][0]['cost'] = 200; bad.append([r])
        r = rule(); r['assets'] *= 2; bad.append([r])
        r = rule(); r['sourceAccount'] = r['destinationAccount']; bad.append([r])
        r = rule(); r['date'] = '2026-02-30'; bad.append([r])
        r = rule(); r['evidence'] = ''; bad.append([r])
        bad.append([rule(), rule()])
        for rules in bad:
            with self.subTest(rules=rules), self.assertRaises(ValueError):
                sr.validate(rules)


class PersistentSyncTest(unittest.TestCase):
    def test_repeat_web_sync_and_full_clear_rebuild_same_result_with_preserved_rule(self):
        old_app_home, old_store_home = app.HOME, store._home
        with tempfile.TemporaryDirectory() as tmp, ExitStack() as stack:
            app.set_home(tmp); store.set_home(tmp); store.ensure()
            self.addCleanup(store.set_home, old_store_home)
            self.addCleanup(app.set_home, old_app_home)
            fixtures = {'load_session': {'access_token':'fake', 'identity_canonical_id':'identity'}, 'save_session':None,
                'fetch_all_accounts':list(ACCOUNTS.values()), 'fetch_balances':[], 'fetch_margin':[],
                'fetch_nav_history':[], 'fetch_nickname_nav_history':([], []), 'fill_listings':None,
                'graphql':report_page()}
            for name, value in fixtures.items():
                stack.enter_context(mock.patch.object(app, name, return_value=value))
            feed = {aid: [item(canonicalId=EVENT+'-'+aid, accountId=aid, type='ASSET_MOVEMENT', subType=side,
                assetSymbol=None, assetQuantity=0, securityId=None, amount=9999, amountSign=sign, currency='CAD')]
                for aid, side, sign in (('account-one', 'SOURCE', 'negative'), ('account-two', 'DESTINATION', 'positive'))}
            for aid, typ, day, amount, sign, fee in (
                    ('account-one', 'DIY_BUY', '2026-09-01', 200, 'negative', 2),
                    ('account-two', 'DIY_SELL', '2026-09-11', 300, 'positive', 3)):
                feed[aid].append(item(canonicalId=typ+'-fixture', accountId=aid, type=typ, subType='',
                    assetSymbol='AAA', assetQuantity=10, securityId='sec-one', amount=amount, fees=fee,
                    amountSign=sign, currency='CAD', occurredAt=day+'T14:01:00Z'))
            stack.enter_context(mock.patch.object(app, 'fetch_activities_for_account', side_effect=lambda s, aid, **kw: feed[aid]))
            version = store.data_version()
            store.save_statement_corrections([rule()])
            self.assertNotEqual(version, store.data_version())
            app._state['syncing'] = False
            self.assertTrue(app.run_sync())
            first = store.snapshot()
            self.assertEqual(first['statementCorrections'][0]['status'], 'applied')
            self.assertAlmostEqual(fifo(first)['closed'][0]['pnl'], 95)
            self.assertTrue(app.run_sync())
            self.assertEqual(store.snapshot()['activities'], first['activities'])
            conn = store._connect()
            try:
                raw = store._all_activities(conn)
            finally:
                conn.close()
            self.assertEqual(len(raw), 4)
            self.assertEqual(sorted(a['netCashAmount'] for a in raw if a['rawType'] == 'ASSET_MOVEMENT'), [-9999,9999])
            saved = store.statement_corrections()
            store.clear_synced_data(keep_journal=False, keep_market=False)
            self.assertEqual(store.statement_corrections(), saved)
            self.assertEqual(store.data_summary()['statementCorrections'], 1)
            self.assertFalse(store.snapshot()['activities'])
            self.assertEqual(store.snapshot()['statementCorrections'][0]['status'], 'pending')
            # All activities, including purchases and sales, come back through web sync.
            self.assertTrue(app.run_sync())
            second = store.snapshot()
            self.assertEqual(second['statementCorrections'], first['statementCorrections'])
            self.assertAlmostEqual(fifo(second)['closed'][0]['pnl'], 95)
            self.assertEqual(len(second['activities']), len(first['activities']))
            def economic(rows):
                return sorted([{k:v for k,v in a.items() if k != 'id'} for a in rows], key=lambda a:a['canonicalId'])
            self.assertEqual(economic(second['activities']), economic(first['activities']))
            store.save_statement_corrections([])
            self.assertEqual(len(store.snapshot()['activities']), 4)


if __name__ == '__main__':
    unittest.main()
