"""Keep retries restricted to reads and prevent DNS fallback request storms."""
import socket
import unittest
from unittest import mock
from urllib.error import URLError, HTTPError
import network_reads
import bagholder as app


class NetworkReadTest(unittest.TestCase):
    def fake_gate(self):
        self.now=100.0
        def sleep(seconds):self.now+=seconds
        return network_reads.ReadGate(clock=lambda:self.now,sleep=sleep)

    def test_successful_reads_are_spaced_across_callers(self):
        g=self.fake_gate(); times=[]
        for _ in range(5):g.call(lambda:times.append(self.now))
        self.assertEqual(times,[100,100.25,100.5,100.75,101])

    def test_dns_retries_are_bounded_and_cooldown_applies_to_the_next_reader(self):
        for error in [socket.gaierror(socket.EAI_AGAIN,'temporary'),URLError(socket.gaierror(socket.EAI_AGAIN,'temporary'))]:
            g=self.fake_gate(); times=[]
            def fail():times.append(self.now);raise error
            with self.assertRaises((socket.gaierror,URLError)):g.call(fail)
            self.assertEqual(times,[100,145,190])
            self.assertEqual(g.call(lambda:self.now),235)

    def test_recovery_retries_only_the_failed_read(self):
        g=self.fake_gate(); read=mock.Mock(side_effect=[URLError(socket.gaierror(-3,'temporary')),{'ok':True}])
        self.assertEqual(g.call(read),{'ok':True})
        self.assertEqual(read.call_count,2)
        self.assertEqual(self.now,145)

    def test_other_failures_are_not_replayed(self):
        for error in [URLError('timed out'),HTTPError('https://example.test',429,'slow',{},None),PermissionError('no')]:
            read=mock.Mock(side_effect=error)
            with self.assertRaises(type(error)):self.fake_gate().call(read)
            self.assertEqual(read.call_count,1)

    def test_graphql_mutations_bypass_the_read_retryer(self):
        with mock.patch.object(app,'_http_json',return_value={'data':{'ok':True}}) as http, mock.patch.object(network_reads,'call',side_effect=lambda read:read()) as retry:
            app.graphql({},'Read',{},'query Read { value }')
            self.assertEqual(retry.call_count,1)
            app.graphql({},'Place',{},'mutation Place { value }')
            self.assertEqual(retry.call_count,1)
            self.assertEqual(http.call_count,2)

    def test_market_reads_use_shared_gate_and_preserve_pool(self):
        import market
        gate = self.fake_gate()
        with mock.patch.object(network_reads, 'gate', gate), mock.patch.object(market, '_fetch', return_value=b'{}') as fetch:
            self.assertEqual(market._get_text('https://example.test/quote'), '{}')
            self.assertEqual(market._post_json('https://example.test/quote', {}), {})
        self.assertEqual(fetch.call_count, 2)
        self.assertEqual(self.now, 100.25)

    def test_pooled_dns_failure_defers_to_shared_gate_without_immediate_retry(self):
        import market
        connection = mock.Mock()
        connection.request.side_effect = socket.gaierror(-3, 'temporary')
        with mock.patch.object(market, '_proxied', return_value=False), mock.patch.object(market, '_take_connection', return_value=connection) as take:
            with self.assertRaises(socket.gaierror):
                market._fetch_once('https://example.test/quote', None, {}, 1)
        self.assertEqual(take.call_count, 1)
        connection.close.assert_called_once()
