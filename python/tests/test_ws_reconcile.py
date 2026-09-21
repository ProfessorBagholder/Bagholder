"""Web-only inventory/return reconciliation, with synthetic broker fixtures."""
import copy
import json
import tempfile
import unittest
from contextlib import ExitStack
from unittest import mock

import bagholder as app
import model
import store
import ws_reconcile as ws
from tests.test_model import buy, sell, act
from tests.test_sync_details import item

ACCOUNTS = {'account-one': {'id':'account-one','nickname':'First'},
            'account-two': {'id':'account-two','nickname':'Second'}}


def transfer(**kw):
    row=item(type='INTERNAL_TRANSFER', subType='IN_KIND', externalCanonicalId='funding_intent-one',
                assetSymbol='AAA', assetQuantity=0, currency='CAD')
    row.update(kw)
    return row


def transfer_detail():
    return dict(sourceFundingPoint={'fundingPointId':'account-one'},
        destinationFundingPoint={'fundingPointId':'account-two'},
        details=dict(transferType='partial_in_kind', totalAmount={'amount':'0','currency':'CAD'},
        internalTransferItems=[dict(security_id='sec-one',symbol='AAA',quantity='10',currency='CAD')]))


def corporate(before='2000', after='66.666667', symbol='AAA', dest='AAA', entitlement='SUBMIT'):
    raw = item(type='CORPORATE_ACTION', subType='CONSOLIDATION', assetSymbol=symbol, currency='CAD')
    nodes = [dict(activityCanonicalId=raw['canonicalId'],assetSymbol=symbol,assetType='EQUITY',
                  entitlementType=entitlement,quantity=before,currency='CAD'),
             dict(activityCanonicalId=raw['canonicalId'],assetSymbol=dest,assetType='EQUITY',
                  entitlementType='RECEIVE',quantity=after,currency=None)]
    return raw, nodes


def with_ids(rows):
    return [dict(r, id=r.get('id') or r['canonicalId']) for r in rows]


def fill_rows(groups):
    return with_ids([r for parent, children in groups for r in [parent]+children])


def report_page(amount='30', value='30', sid='sec-one', more=False, cursor=None):
    return dict(identity=dict(financials=dict(realizedReturns=dict(totalValue=dict(amount=amount,currency='CAD'),
        timeRangeBreakdown=[dict(year=2026, month=9,totalValue=dict(amount=amount,currency='CAD'))],
        securityBreakdown=dict(edges=[dict(node=dict(security=dict(id=sid,stock=dict(symbol='AAA')),
            totalValue=dict(amount=value,currency='CAD')))],pageInfo=dict(hasNextPage=more,endCursor=cursor))))))


class InventoryTest(unittest.TestCase):
    def _public_trades(self, rows):
        normalized = [model.normalize_activity(r) for r in rows]
        fifo = model.match_fifo(normalized)
        return [model.public_trade(t) for t in model.build_trades(fifo['closed'], fifo['open'], [],
            {a['id']:a for a in normalized}, model.Securities([]), {})]

    def test_missing_transfer_portion_is_distinct_from_full_broker_sale(self):
        detail = transfer_detail()
        detail['details']['internalTransferItems'][0]['quantity'] = '795'
        group = ws.transfer_group([transfer()],ACCOUNTS,app.map_activity_rows,detail,{})
        rows = [*fill_rows([group]),
            buy('b','AAA',1205,40,'2026-09-10',accountId='account-two',accountType='Second'),
            sell('s','AAA',2000,65,'2026-09-11',accountId='account-two',accountType='Second')]
        original = copy.deepcopy(rows)
        trades = self._public_trades(rows)
        missing = next(t for t in trades if t.get('basisIncomplete'))
        self.assertEqual((missing['qty'],missing['legs'][0]['qty']), (795,795))
        self.assertIsNone(missing['pnl'])
        movement = next(f for f in missing['fills'] if f['inventory'])
        self.assertEqual((movement['side'],movement['sub'],movement['qty']),('IN','TRANSFER IN',795))
        self.assertIsNone(movement['price'])
        sale = next(f for f in missing['fills'] if f['id']=='s')
        self.assertEqual((sale['qty'],sale['amount']),(-2000,130000))
        self.assertEqual((missing['opened']['fills'],missing['closed']['fills']),(1,1))
        self.assertEqual(rows,original)

    def test_consolidation_missing_fraction_is_not_a_zero_cost_purchase(self):
        raw,nodes = corporate('30','3.75')
        group = ws.corporate_group(raw,ACCOUNTS,app.map_activity_rows,nodes)
        rows = [buy('b','AAA',27,47/27,'2026-09-01',accountId='account-one',accountType='First'),
            *fill_rows([group]),
            sell('s','AAA',3.75,3.16,'2026-09-10',accountId='account-one',accountType='First')]
        trades = self._public_trades(rows)
        missing = next(t for t in trades if t.get('basisIncomplete'))
        known = next(t for t in trades if not t.get('basisIncomplete'))
        self.assertEqual((missing['qty'],known['qty']),(.375,3.375))
        self.assertAlmostEqual(known['pnl'],3.375*3.16-47)
        self.assertIsNone(missing['entry'])
        self.assertIsNone(missing['legs'][0]['pnl'])
        movement = next(f for f in missing['fills'] if f['inventory'])
        self.assertEqual((movement['sub'],movement['qty'],movement['amount']),('REORGANIZATION IN',3.75,0))
        self.assertIsNone(movement['price'])
        sale = next(f for f in missing['fills'] if f['id']=='s')
        self.assertEqual(sale['qty'],-3.75)
        self.assertAlmostEqual(sale['amount'],11.85)

    def test_manual_asset_movement_warns_once_without_inventing_inventory(self):
        rows = [act(canonicalId='asset_movement_request_group-one-'+aid,
                    accountId=aid, rawType='ASSET_MOVEMENT', quantity=0,
                    netCashAmount=sign*5000) for aid,sign in [('source',-1),('destination',1)]]
        original = copy.deepcopy(rows)
        warnings = model.inventory_warnings({'activities':rows})
        self.assertEqual(len(warnings),1)
        self.assertEqual(warnings[0]['event'],'asset_movement_request_group-one')
        self.assertIn('lacks security quantities',warnings[0]['reason'])
        self.assertEqual(rows,original)

    def test_transfer_uses_dated_feed_symbol_and_replays_without_purchase_refetch(self):
        purchase = item(canonicalId='purchase', externalCanonicalId='order-purchase', type='DIY_BUY',
            subType='BUY', assetSymbol='AAA.TO', securityId='sec-one', currency='CAD',
            assetQuantity=10, amount=50, occurredAt='2026-09-01T12:00:00Z')
        raw = transfer()
        cache = {}
        def response(op, variables, query):
            if op == 'FetchFundingIntentStatusSummary':
                return {'fundingIntentStatusSummary':transfer_detail()}
            return {'securities':[dict(id='sec-one',stock={'symbol':'AAA'},currency='CAD')]}
        query = mock.Mock(side_effect=response)
        groups, warnings = ws.enrich([raw,purchase],ACCOUNTS,app.map_activity_rows,query,cache)
        self.assertFalse(warnings)
        self.assertEqual({r['symbol'] for r in groups[0][1]}, {'AAA.TO'})
        rows = with_ids(app.map_activity_rows(purchase,ACCOUNTS)) + fill_rows(groups)
        rows.append(sell('s','AAA.TO',10,8,'2026-09-10',accountId='account-two',accountType='Second'))
        result = model.match_fifo(rows)
        self.assertFalse(result['unmatched'])
        self.assertFalse(result['open'])
        self.assertAlmostEqual(sum(t['pnl'] for t in result['closed']),30)
        query.reset_mock()
        self.assertEqual(ws.enrich([],ACCOUNTS,app.map_activity_rows,query,cache),(groups,warnings))
        query.assert_not_called()
        rebuilt = ws.enrich([purchase,raw],ACCOUNTS,app.map_activity_rows,query,{})
        self.assertEqual(rebuilt,(groups,warnings))

    def test_transfer_alias_requires_same_identity_currency_and_unambiguous_past_symbol(self):
        history = {('sec-one','CAD'):{'2026-01-01':{'AAA.TO'},'2026-09-10':{'NEW'}}}
        self.assertEqual(ws.transfer_symbol('AAA','sec-one','CAD','2026-09-09',history),'AAA.TO')
        for symbol,sid,currency,date in [('AAA','sec-other','CAD','2026-09-09'),
                ('AAA','sec-one','USD','2026-09-09'), ('AAA','sec-one','CAD','2025-12-31'),
                ('NEW','sec-one','CAD','2026-09-09'), ('AAA','sec-one','CAD','2026-09-11')]:
            self.assertEqual(ws.transfer_symbol(symbol,sid,currency,date,history),symbol)
        history[('sec-one','CAD')]['2026-01-01'].add('AAA.V')
        self.assertEqual(ws.transfer_symbol('AAA','sec-one','CAD','2026-09-09',history),'AAA')

    def test_option_crypto_and_cancelled_summaries_cannot_supply_stock_aliases(self):
        cache = {}
        rows = [item(canonicalId=str(n),securityId='sec-one',assetSymbol='AAA.TO',assetQuantity=10,
                     currency='CAD',**changes) for n,changes in enumerate([
            {'type':'CRYPTO_SELL'}, {'type':'DIY_BUY','contractType':'CALL'},
            {'type':'DIY_BUY','status':'CANCELLED'}])]
        self.assertEqual(ws.symbol_history(rows,ACCOUNTS,app.map_activity_rows,cache),{})

    def test_one_feed_side_supplies_both_transfer_legs_and_carries_original_basis(self):
        group = ws.transfer_group([transfer()], ACCOUNTS, app.map_activity_rows, transfer_detail(), {})
        parent, legs = group
        self.assertEqual(parent['netCashAmount'], 0)
        self.assertEqual([l['quantity'] for l in legs], [-10,10])
        rows = [buy('b','AAA',10,5,'2026-09-01',accountId='account-one',accountType='First'),
                *fill_rows([group]),
                sell('s','AAA',10,8,'2026-09-10',accountId='account-two',accountType='Second')]
        r = model.match_fifo(rows)
        self.assertFalse(r['open'])
        self.assertFalse(r['unmatched'])
        self.assertAlmostEqual(sum(t['pnl'] for t in r['closed']),30)
        self.assertEqual(r['closed'][0]['entryDate'],'2026-09-01')

    def test_multiple_same_day_transfers_are_paired_by_event(self):
        groups=[]
        for n in ('one','two'):
            groups.append(ws.transfer_group([transfer(canonicalId=n,externalCanonicalId='funding_intent-'+n)],
                ACCOUNTS,app.map_activity_rows,transfer_detail(),{}))
        r=model.match_fifo([buy('b','AAA',20,5,'2026-09-01',accountId='account-one',accountType='First'),
            *fill_rows(groups), sell('s','AAA',20,8,'2026-09-10',accountId='account-two',accountType='Second')])
        self.assertFalse(r['unmatched'])
        self.assertAlmostEqual(sum(t['pnl'] for t in r['closed']),60)

    def test_explicit_noninteger_reverse_split_preserves_entire_basis(self):
        raw,nodes=corporate()
        group=ws.corporate_group(raw,ACCOUNTS,app.map_activity_rows,nodes)
        r=model.match_fifo([buy('b','AAA',2000,4,'2026-09-01',accountId='account-one',accountType='First'),
            *fill_rows([group]),sell('s','AAA',66.666667,10,'2026-09-10',accountId='account-one',accountType='First')])
        self.assertFalse(r['open'])
        self.assertFalse(r['unmatched'])
        self.assertAlmostEqual(sum(t['pnl'] for t in r['closed']), 666.66667-8000)

    def test_hold_plus_receive_and_rename_use_explicit_quantities(self):
        for before,after,dest,ent,total in [('10','30','AAA','HOLD',40),('10','10','NEW','SUBMIT',10)]:
            raw,nodes=corporate(before,after,dest=dest,entitlement=ent)
            group=ws.corporate_group(raw,ACCOUNTS,app.map_activity_rows,nodes)
            r=model.match_fifo([buy('b','AAA',10,10,'2026-09-01',accountId='account-one',accountType='First'),*fill_rows([group])])
            self.assertEqual(sum(l['qty'] for l in r['open']),total)
            self.assertAlmostEqual(sum(l['qty']*l['price'] for l in r['open']),100)
            self.assertEqual({l['symbol'] for l in r['open']},{dest})

    def test_spinoff_cannot_move_all_basis_and_incomplete_details_retry(self):
        raw,nodes=corporate('10','3',dest='OTHER',entitlement='HOLD')
        query=mock.Mock(return_value={'corporateActionChildActivities':{'nodes':nodes}})
        cache={}
        for _ in range(2):
            groups,warnings=ws.enrich([raw],ACCOUNTS,app.map_activity_rows,query,cache)
            self.assertEqual({m['symbol'] for m in groups[0][1]},{'AAA','OTHER'})
            self.assertTrue(all(m['quantity']==0 and m['netCashAmount']==0 for m in groups[0][1]))
            self.assertEqual(groups[0][0]['rawType'],'WS_DETAIL_MISSING_INVENTORY')
            self.assertIn('cost allocation',warnings[0]['reason'])
            self.assertEqual(warnings[0]['symbols'],['AAA','OTHER'])
            self.assertEqual(warnings[0]['date'],'2026-09-09')
        self.assertEqual(query.call_count,2)
        r=model.match_fifo([buy('b','AAA',10,5,'2026-09-01',accountId='account-one',accountType='First'), *fill_rows(groups), sell('s','AAA',10,10,'2026-09-10',accountId='account-one',accountType='First')])
        self.assertFalse(model.trade_basis_known(r['closed'][0]))

    def test_cancelled_event_is_not_queried(self):
        query=mock.Mock()
        self.assertEqual(ws.enrich([transfer(status='CANCELLED')],ACCOUNTS,app.map_activity_rows,query,{}),([],[]))
        query.assert_not_called()

    def test_exercise_creates_stock_at_strike_and_fee_without_option_sale_proceeds(self):
        for right,qty,cash,side in [('CALL',100,-5020,'BUY'),('PUT',-100,4980,'SELL')]:
            a=act(id='exercise',symbol='XYZ 18SEP26 50.00 '+right,rawType='OPTIONS_EXERCISE',
                activityType='EXERCISE',category='option_event',quantity=1,unitPrice=50.20,netCashAmount=cash)
            normalized=model.normalize_activity(a)
            self.assertEqual(normalized['netCashAmount'],0)
            self.assertEqual(normalized['unitPrice'],0)
            row=model.synthesize_assignment_shares([normalized],model.Securities([]))[0]
            self.assertEqual((row['quantity'],row['unitPrice'],row['commission'],row['netCashAmount']), (qty,50,20,cash))
            self.assertEqual(row['activitySubType'],side)

    def test_equal_timestamp_matching_is_stable_after_record_ids_change(self):
        rows=[buy('z','AAA',1,1,'2026-09-01',canonicalId='a'),buy('a','AAA',1,2,'2026-09-01',canonicalId='z'),
              sell('s','AAA',1,5,'2026-09-02',canonicalId='s')]
        before=model.match_fifo(rows)['closed'][0]['pnl']
        rows[0]['id'],rows[1]['id']='a','z'
        self.assertEqual(model.match_fifo(rows)['closed'][0]['pnl'],before)


class ReportTest(unittest.TestCase):
    def test_paginated_total_reconciles_and_keeps_currency_and_scope(self):
        q=mock.Mock(side_effect=[report_page(value='10',more=True,cursor='next'),report_page(value='20',sid='sec-two')])
        r=ws.fetch_report('identity',q)
        self.assertEqual(r['total'],30)
        self.assertEqual(len(r['securities']),2)
        self.assertEqual(q.call_args_list[1].args[1]['cursor'],'next')
        self.assertEqual(r['currency'],'CAD')

    def test_partial_duplicate_changed_or_missing_totals_never_look_reconciled(self):
        for pages in ([report_page(value='20')], [report_page(value=None)],
            [report_page(value='10',more=True,cursor='next'),report_page(value='20')],
            [report_page(value='10',more=True,cursor='next'),report_page(amount='40',value='30',sid='other')],
            [report_page(value='10',more=True,cursor='same'),report_page(value='20',sid='other',more=True,cursor='same')]):
            with self.subTest(pages=pages), self.assertRaises(ValueError):
                ws._fetch_report_once('identity',mock.Mock(side_effect=pages))

    def test_inconsistent_report_restarts_without_reusing_partial_pages(self):
        q=mock.Mock(side_effect=[report_page(value='10',more=True,cursor='next'), report_page(value='20'), report_page()])
        r=ws.fetch_report('identity',q)
        self.assertEqual(r['total'],30)
        self.assertEqual(len(r['securities']),1)
        self.assertIsNone(q.call_args_list[2].args[1]['cursor'])

    def test_zero_is_valid_but_whole_account_report_is_hidden_for_filters(self):
        report=ws.fetch_report('identity',mock.Mock(return_value=report_page(amount='0',value='0')))
        base={'brokerPerformance':report}
        self.assertEqual(model.broker_performance_view(base,model.clean_filters({}))['total'],0)
        for f in [{'search':'AAA'},{'lists':{'symbol':['AAA']}},{'from':'2026-09-01'},{'preset':'ytd'}]:
            self.assertEqual(model.broker_performance_view(base,model.clean_filters(f))['status'],'filtered')


class SyncReplayTest(unittest.TestCase):
    def setUp(self):
        self.tmp=tempfile.TemporaryDirectory()
        app.set_home(self.tmp.name);store.set_home(self.tmp.name);store.ensure()
        self.stack=ExitStack()
        fixtures={'load_session':{'access_token':'fake','identity_canonical_id':'identity'},'save_session':None,
            'fetch_all_accounts':list(ACCOUNTS.values()),'fetch_balances':[],'fetch_margin':[],
            'fetch_nav_history':[],'fetch_nickname_nav_history':([],[]),'fill_listings':None}
        for name,result in fixtures.items():
            self.stack.enter_context(mock.patch.object(app,name,return_value=result))
        self.raw=transfer()
        self.stack.enter_context(mock.patch.object(app,'fetch_activities_for_account',side_effect=lambda s,aid,**kw:[self.raw] if aid=='account-one' else []))
        self.query=mock.Mock(side_effect=self.graphql)
        self.stack.enter_context(mock.patch.object(app,'graphql',self.query))
        app._state['syncing']=False

    def tearDown(self):
        self.stack.close();self.tmp.cleanup()

    def graphql(self,s,op,v,q):
        if op=='FetchFundingIntentStatusSummary':return {'fundingIntentStatusSummary':transfer_detail()}
        if op=='DetailSecurities':return {'securities':[dict(id='sec-one',currency='CAD',stock={'symbol':'AAA'})]}
        if op=='FetchIdentityRealizedReturns':return report_page()
        raise AssertionError(op)

    def test_repeat_and_clear_resync_produce_same_economic_rows_and_return(self):
        store.save_journal_entry('existing',{'thesis':'Keep this'})
        self.assertTrue(app.run_sync())
        first=store.snapshot()
        self.assertFalse(app.activity_sync_bounds()['full_history'])
        count=self.query.call_count
        self.assertTrue(app.run_sync())
        self.assertEqual(self.query.call_count,count+1,'only the changing return report is reread')
        self.assertEqual(first['activities'],store.snapshot()['activities'])
        store.clear_synced_data()
        for key in (ws.CACHE_KEY,ws.VERSION_KEY,ws.REPORT_KEY):self.assertEqual(store.get_meta(key),'')
        self.assertIn('existing',store.journal())
        self.assertTrue(app.activity_sync_bounds()['full_history'])
        self.assertTrue(app.run_sync())
        def economic(rows):return sorted([{k:v for k,v in r.items() if k!='id'} for r in rows],key=lambda r:r['canonicalId'])
        self.assertEqual(economic(first['activities']),economic(store.snapshot()['activities']))
        self.assertEqual(first['brokerPerformance']['total'],store.snapshot()['brokerPerformance']['total'])

    def test_synthetic_corporate_legs_do_not_trigger_impossible_feed_backfill(self):
        raw,nodes=corporate()
        store.apply_wealthsimple_detail_groups([ws.corporate_group(raw,ACCOUNTS,app.map_activity_rows,nodes)])
        self.assertFalse(store.needs_security_id_backfill())

    def test_failed_report_keeps_previous_total_explicitly_stale(self):
        self.assertTrue(app.run_sync())
        original=self.graphql
        def changed(s,op,v,q):
            if op=='FetchIdentityRealizedReturns':raise ValueError('Incomplete response')
            return original(s,op,v,q)
        self.query.side_effect=changed
        self.assertTrue(app.run_sync())
        r=store.snapshot()['brokerPerformance']
        self.assertEqual((r['status'],r['total']),('stale',30))
