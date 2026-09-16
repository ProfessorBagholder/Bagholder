"""Notifications: what is told, once, and how it reaches the page."""
import http.client
import json
import os
import subprocess
import sys
import tempfile
import threading
import time
import unittest
from datetime import datetime, timedelta, timezone
from http.server import ThreadingHTTPServer
from unittest import mock

sys.path.insert(0, os.path.dirname(os.path.dirname(os.path.abspath(__file__))))

import bagholder  # noqa: E402
import disclosures  # noqa: E402
import model  # noqa: E402
import news  # noqa: E402
import notify  # noqa: E402
import store  # noqa: E402

OFF = {"fills": False, "problems": False, "connection": False, "updates": False,
       "releasesHeld": False, "releasesWatched": False, "releasesAll": False,
       "disclosuresHeld": False, "disclosuresWatched": False, "disclosuresAll": False}

ORDER = {"id": "o1", "symbol": "QNC", "account": "🚀 Trading", "side": "BUY", "type": "LIMIT", "quantity": 5.0, "limitPrice": 1.75, "status": "pending", "role": "entry", "source": "bagholder", "securityId": "sec-1"}


class NotifyTest(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.home = self.tmp.name
        os.environ["BAGHOLDER_HOME"] = self.home
        os.environ["BAGHOLDER_NOTIFY"] = "browser"   # this process must never post on the machine running the suite
        store.set_home(self.home)
        bagholder.set_home(self.home)
        store.ensure()
        with bagholder._lock:
            bagholder._state.update({"connected": False, "error": "", "syncing": False, "capturing": False, "email": "", "syncFails": 0, "syncFirstFail": ""})
        # a new filing is read for its title before it is told; here nothing is read, so no test
        # reaches a regulator or a model and what is told does not depend on either being up
        self.reads = mock.patch.object(bagholder, "filings_enrich", return_value={})
        self.reads.start()
        # and no test asks whether a model is up, which would start one downloading here
        self.naming = mock.patch.object(bagholder, "_can_name_documents", return_value=True)
        self.naming.start()

    def tearDown(self):
        self.naming.stop()
        self.reads.stop()
        self.tmp.cleanup()
        os.environ.pop("BAGHOLDER_HOME", None)
        os.environ.pop("BAGHOLDER_NOTIFY", None)

    def test_every_kind_is_off_until_turned_on_and_the_settings_round_trip(self):
        self.assertEqual(notify.settings(), OFF)
        out = notify.set_settings({"fills": True, "bogus": True, "updates": "yes"})
        self.assertEqual(out, dict(OFF, fills=True), "unknown keys and non-booleans are ignored")
        self.assertEqual(notify.settings(), out)
        self.assertEqual(bagholder.status_payload()["notify"], dict(out, native="", unread=0), "the status payload carries the kinds, the channel and the unread count; told to stand aside, the page is the channel")
        with mock.patch.dict(os.environ, {"BAGHOLDER_NOTIFY": ""}), mock.patch.object(sys, "platform", "darwin"), mock.patch.object(notify.shutil, "which", lambda n: "/usr/bin/" + n):
            self.assertEqual(notify.native_channel(), "mac")
        with mock.patch.dict(os.environ, {"BAGHOLDER_NOTIFY": ""}), mock.patch.object(sys, "platform", "win32"), mock.patch.object(notify.shutil, "which", lambda n: "C:/ps.exe" if n == "powershell" else None):
            self.assertEqual(notify.native_channel(), "windows")
        with mock.patch.dict(os.environ, {"BAGHOLDER_NOTIFY": "", "DISPLAY": ":0"}), mock.patch.object(sys, "platform", "linux"), mock.patch.object(notify.shutil, "which", lambda n: "/usr/bin/" + n if n == "notify-send" else None):
            self.assertEqual(notify.native_channel(), "linux")
        with mock.patch.dict(os.environ, {"BAGHOLDER_NOTIFY": "", "DISPLAY": "", "WAYLAND_DISPLAY": ""}), mock.patch.object(sys, "platform", "linux"), mock.patch.object(notify.shutil, "which", lambda n: None):
            self.assertEqual(notify.native_channel(), "", "no desktop: the page is the channel")

    def test_a_kind_that_is_off_is_not_told_and_a_key_is_told_once(self):
        self.assertIsNone(notify.emit("fills", "order:1:filled", "Order filled · QNC", "Bought 5 at 1.75"))
        notify.set_settings({"fills": True})
        row = notify.emit("fills", "order:1:filled", "Order filled · QNC", "Bought 5 at 1.75")
        self.assertEqual((row["kind"], row["title"], row["body"], row["seenAt"], row["readAt"]), ("fills", "Order filled · QNC", "Bought 5 at 1.75", "", ""))
        self.assertIsNone(notify.emit("fills", "order:1:filled", "Order filled · QNC", "again"), "the same event is never told twice")
        self.assertIsNone(notify.emit("bogus", "x", "t", "b"), "an unknown kind is nothing")
        self.assertIsNone(notify.emit("disclosures", "f1", "t", "b"), "no set of tickers chosen: disclosures are not told")
        notify.set_settings({"disclosuresWatched": True})
        self.assertEqual(notify.disclosure_scopes(), {"watched"})
        self.assertIsNotNone(notify.emit("disclosures", "f1", "t", "b"), "any set on: the kind is told")
        self.assertGreater(notify.test_notification()["id"], row["id"], "the test goes out whatever the kinds say")
        self.assertEqual(len(store.list_notifications()), 3)

    def test_seen_rows_are_not_listed_again_and_the_oldest_are_pruned(self):
        notify.set_settings({"fills": True})
        ids = [notify.emit("fills", "k%d" % i, "t", "b")["id"] for i in range(3)]
        self.assertEqual(store.mark_notifications_seen([ids[0], "x", None]), 1)
        self.assertEqual([r["id"] for r in store.list_notifications(unseen=True)], ids[1:])
        self.assertEqual([r["id"] for r in store.list_notifications(after_id=ids[1])], ids[2:])
        with mock.patch.object(store, "NOTIFICATIONS_KEPT", 2):
            notify.emit("fills", "k9", "t", "b")
        self.assertEqual(len(store.list_notifications()), 2, "the newest are kept")
        self.assertEqual([r["key"] for r in store.list_notifications(newest=True)], ["k9", "k2"], "the history reads newest first")
        # the history: unread until looked at, read when the panel closes, cleared on Clear
        self.assertEqual(store.unread_notifications(), 2)
        self.assertEqual(store.mark_notifications_read([ids[2], "x"]), 1)
        self.assertEqual(store.unread_notifications(), 1)
        self.assertEqual(store.mark_notifications_read(), 1, "no ids: every unread one")
        self.assertEqual((store.unread_notifications(), store.mark_notifications_read()), (0, 0))
        self.assertTrue(all(r["readAt"] for r in store.list_notifications()))
        self.assertEqual(store.clear_notifications(), 2)
        self.assertEqual((store.list_notifications(), store.latest_notification_id()), ([], 0))

    def test_the_stream_sends_every_row_after_the_id_the_page_brings_with_pings_between(self):
        notify.set_settings({"fills": True})
        old = notify.emit("fills", "old", "Old", "b")
        store.mark_notifications_seen([old["id"]])
        first = notify.emit("fills", "first", "First", "b")
        ticks = iter([True, True, True, False])
        with mock.patch.object(notify, "HEARTBEAT_SEC", 0.05):
            gen = notify.stream(after=old["id"], alive=lambda: next(ticks))
            hello, row1, ping = next(gen), next(gen), next(gen)
            second = notify.emit("fills", "second", "Second", "b")
            row2 = next(gen)
            self.assertEqual(list(gen), [], "alive said no: the stream ends")
        self.assertEqual(hello, ": bagholder\n\n")
        self.assertTrue(row1.startswith("id: %d\ndata: " % first["id"]) and '"title": "First"' in row1, row1)
        self.assertEqual(ping, ": ping\n\n", "nothing new by the heartbeat: a comment keeps the connection")
        self.assertTrue(row2.startswith("id: %d\ndata: " % second["id"]), row2)
        with mock.patch.object(notify, "HEARTBEAT_SEC", 0.05), mock.patch.object(notify, "native_channel", return_value="mac"), mock.patch.object(notify, "deliver", return_value=True):
            gen = notify.stream()
            self.assertEqual((next(gen), next(gen)), (": bagholder\n\n", ": ping\n\n"), "no id: only what is made after the stream opens")
            third = notify.emit("fills", "third", "Third", "b")
            chunk = next(gen)
        self.assertTrue(chunk.startswith("id: %d\n" % third["id"]) and '"seenAt": "20' in chunk, "a row the server posts itself still reaches the history, already seen")

    def test_a_fill_read_back_is_told_by_its_role(self):
        self.assertEqual(bagholder.order_notice(ORDER, {"status": "filled", "filledQty": 5.0, "avgFill": 1.75}), ("fills", "order:o1:filled", "Order filled · QNC", "Bought 5 at 1.75 · 🚀 Trading"))
        stop = dict(ORDER, id="o2", side="SELL", type="STOP", stopPrice=1.66, role="stop")
        self.assertEqual(bagholder.order_notice(stop, {"status": "filled", "filledQty": 5.0, "avgFill": 1.6374}), ("fills", "order:o2:filled", "Stopped out · QNC", "Sold 5 at 1.64 · 🚀 Trading"))
        target = dict(ORDER, id="o3", side="SELL", limitPrice=1.93, role="target")
        self.assertEqual(bagholder.order_notice(target, {"status": "filled", "filledQty": 5.0, "avgFill": 1.93})[2], "Target hit · QNC")
        self.assertIsNone(bagholder.order_notice(dict(ORDER, status="filled"), {"status": "filled", "filledQty": 5.0, "avgFill": 1.75}), "read back filled again: nothing new")
        self.assertEqual(bagholder.order_notice(dict(ORDER, quantity=100.0), {"status": "pending", "filledQty": 40.0, "avgFill": 64.5}), ("fills", "order:o1:partial:40", "Partly filled · QNC", "40 of 100 at 64.50 · 🚀 Trading"))
        self.assertIsNone(bagholder.order_notice(dict(ORDER, quantity=100.0, filledQty=40.0), {"status": "pending", "filledQty": 40.0, "avgFill": 64.5}), "the same partial fill again: nothing new")

    def test_problems_are_told_but_not_the_persons_own_cancel_nor_a_legs_expiry(self):
        self.assertEqual(bagholder.order_notice(ORDER, {"status": "rejected", "error": "Limit price has too many decimal places. Max allowed: 2"}),
                         ("problems", "order:o1:rejected", "Order rejected · QNC", "Buy 5 at 1.75 limit · Limit price has too many decimal places. Max allowed: 2"))
        self.assertEqual(bagholder.order_notice(ORDER, {"status": "failed"})[2], "Order not sent · QNC")
        self.assertEqual(bagholder.order_notice(ORDER, {"status": "expired"}), ("problems", "order:o1:expired", "Order expired · QNC", "Buy 5 at 1.75 limit · 🚀 Trading"))
        self.assertEqual(bagholder.order_notice(ORDER, {"status": "cancelled"}), ("problems", "order:o1:cancelled", "Order cancelled · QNC", "Buy 5 at 1.75 limit · 🚀 Trading"))
        self.assertIsNone(bagholder.order_notice(dict(ORDER, status="cancelling"), {"status": "cancelled"}), "a cancel asked for here passes through cancelling: not told")
        stop = dict(ORDER, id="o2", side="SELL", type="STOP", stopPrice=1.66, role="stop")
        self.assertIsNone(bagholder.order_notice(stop, {"status": "expired"}), "a leg's expiry is the engine's to place again")
        self.assertIsNone(bagholder.order_notice(stop, {"status": "cancelled"}))
        self.assertEqual(bagholder.order_notice(stop, {"status": "rejected", "error": "no shares"})[3], "Sell 5 stop 1.66 · no shares")
        self.assertEqual(bagholder._order_words(dict(ORDER, type="MARKET")), "Buy 5 at market")
        self.assertEqual(bagholder._order_words(dict(ORDER, type="STOP_LIMIT", stopPrice=1.6, limitPrice=1.55, side="SELL", quantity=2.5)), "Sell 2.5 stop 1.60 · limit 1.55")
        self.assertEqual([bagholder._price_words(p) for p in (1.6374, 0.625, 0.54, 12, None)], ["1.64", "0.625", "0.54", "12.00", "—"])

    def test_a_status_read_back_by_the_refresh_is_told_once(self):
        notify.set_settings({"fills": True, "problems": True})
        upd = {"wsStatus": "FILLED", "status": "filled", "filledQty": 5.0, "avgFill": 1.75}
        with mock.patch.object(bagholder, "_ticket_session", return_value={"access_token": "t"}), \
             mock.patch.object(bagholder, "fetch_extended_order", return_value=upd), \
             mock.patch.object(bagholder, "_identity_from", return_value=None), \
             mock.patch.object(bagholder, "book_order_fill"), \
             mock.patch.object(store, "list_orders", return_value=[dict(ORDER)]), \
             mock.patch.object(store, "update_order"), mock.patch.object(store, "get_order", return_value=dict(ORDER)):
            bagholder.refresh_orders()
            bagholder.refresh_orders()
        self.assertEqual([(r["kind"], r["title"], r["body"]) for r in store.list_notifications()], [("fills", "Order filled · QNC", "Bought 5 at 1.75 · 🚀 Trading")])

    def test_a_bracket_that_ends_for_a_reason_of_its_own_is_told_and_the_persons_doing_is_not(self):
        notify.set_settings({"problems": True})
        b = {"id": "b1", "orderId": "o1", "symbol": "QNC", "status": "armed", "slOrderId": "", "tpOrderId": "", "attempts": 0}
        with mock.patch.object(bagholder, "_own_exit_rows", return_value=[]), mock.patch.object(store, "update_bracket"), \
             mock.patch.object(store, "get_order", return_value={"account": "🚀 Trading"}), mock.patch.object(sys, "stderr"):
            bagholder._end_bracket(b, "stop cancelled at Wealthsimple by hand")
            bagholder._end_bracket(b, "stop cancelled at Wealthsimple by hand")
            bagholder._end_bracket(dict(b, id="b2"), "cancelled by the user")
            bagholder._end_bracket(dict(b, id="b3"), "both legs removed")
            bagholder._end_bracket(dict(b, id="b4"), "sold from the ticket")
            bagholder._end_bracket(dict(b, id="b5"), "stopped")
            bagholder._fail(b, "stop not placed: Insufficient shares")
            bagholder._fail(dict(b, attempts=1), "stop not placed: Insufficient shares")
        self.assertEqual([(r["title"], r["body"]) for r in store.list_notifications()], [
            ("Bracket off · QNC", "Stop cancelled at Wealthsimple by hand · 🚀 Trading"),
            ("Bracket · QNC", "Stop not placed: Insufficient shares · trying again in a minute · 🚀 Trading"),
        ], "the fill is the order's to tell, the person's own doing is nothing, a refusal is told on the first attempt")

    def test_the_session_expiring_is_told_on_the_transition_and_a_failing_sync_on_the_third_time(self):
        notify.set_settings({"connection": True})
        bagholder.note_session_expired()
        self.assertEqual(store.list_notifications(), [], "not connected: nothing expired")
        with bagholder._lock:
            bagholder._state["connected"] = True
        bagholder.note_session_expired()
        bagholder.note_session_expired()
        self.assertFalse(bagholder._state["connected"])
        for _ in range(4):
            bagholder.note_sync_failed("Wealthsimple did not answer")
        self.assertEqual([(r["title"], r["body"]) for r in store.list_notifications()], [
            ("Sign in needed", "The Wealthsimple session expired. Connect again from the menu."),
            ("Sync failing", "Wealthsimple did not answer"),
        ])

    def test_a_newer_release_is_told_once(self):
        notify.set_settings({"updates": True})
        with mock.patch.object(bagholder, "_http_json", return_value={"tag_name": "v99.0.0", "html_url": "https://x", "assets": []}):
            bagholder.check_for_update()
            bagholder.check_for_update()
        self.assertEqual([(r["title"], r["body"]) for r in store.list_notifications()], [("Bagholder v99.0.0 is available", "Update from the header.")])
        with mock.patch.object(bagholder, "_http_json", return_value={"tag_name": "v0.0.1", "html_url": "https://x", "assets": []}):
            bagholder.check_for_update()
        self.assertEqual(len(store.list_notifications()), 1, "an older release never tells")

    def test_the_endpoints_serve_the_settings_the_stream_the_test_and_the_seen_mark(self):
        srv = ThreadingHTTPServer(("127.0.0.1", 0), bagholder.Handler)
        threading.Thread(target=srv.serve_forever, daemon=True).start()
        port = srv.server_address[1]
        write = {"X-Bagholder": "1", "Content-Type": "application/json"}
        try:
            with mock.patch.object(notify, "HEARTBEAT_SEC", 0.2):
                c = http.client.HTTPConnection("127.0.0.1", port, timeout=5)
                c.request("POST", "/api/notifications/settings", body=json.dumps({"fills": True}), headers=write)
                self.assertEqual(json.loads(c.getresponse().read())["settings"]["fills"], True)
                s = http.client.HTTPConnection("127.0.0.1", port, timeout=5)
                s.request("GET", "/api/notifications/stream")
                resp = s.getresponse()
                self.assertEqual((resp.status, resp.getheader("Content-Type")), (200, "text/event-stream; charset=utf-8"))
                self.assertEqual(resp.readline(), b": bagholder\n")
                c.request("POST", "/api/notifications/test", body="{}", headers=write)
                self.assertTrue(json.loads(c.getresponse().read())["ok"])
                got = None
                deadline = time.time() + 5
                while time.time() < deadline and got is None:
                    line = resp.readline()
                    if line.startswith(b"data: "):
                        got = json.loads(line[6:])
                self.assertEqual((got["kind"], got["title"], got["body"]), ("test", "Bagholder", "Notifications reach you here."))
                s.close()
                c.request("POST", "/api/notifications/seen", body=json.dumps({"ids": [got["id"]]}), headers=write)
                self.assertEqual(json.loads(c.getresponse().read())["seen"], 1)
                c.request("GET", "/api/notifications")
                out = json.loads(c.getresponse().read())
                self.assertEqual((out["settings"]["fills"], out["rows"][0]["seenAt"] != "", out["unread"]), (True, True, 1))
                c.request("POST", "/api/notifications/read", body="{}", headers=write)
                self.assertEqual(json.loads(c.getresponse().read())["read"], 1)
                c.request("GET", "/api/notifications")
                self.assertEqual(json.loads(c.getresponse().read())["unread"], 0)
                c.request("POST", "/api/notifications/clear", body="{}", headers=write)
                self.assertEqual(json.loads(c.getresponse().read())["cleared"], 1)
                c.request("GET", "/api/notifications")
                self.assertEqual(json.loads(c.getresponse().read())["rows"], [])
                c.request("POST", "/api/notifications/settings", body="{}", headers={"Content-Type": "application/json"})
                self.assertEqual(c.getresponse().status, 403, "a write without the page's own header is refused")
        finally:
            srv.shutdown()
            srv.server_close()


    def test_posted_by_the_server_a_row_is_stored_seen_and_handed_to_the_system(self):
        notify.set_settings({"fills": True})
        calls = []
        with mock.patch.object(notify, "native_channel", return_value="mac"), mock.patch.object(notify, "deliver", side_effect=lambda ch, t, b: calls.append((ch, t, b)) or True):
            row = notify.emit("fills", "order:1:filled", "Order filled · QNC", "Bought 5 at 1.75")
            deadline = time.time() + 3
            while time.time() < deadline and not calls:
                time.sleep(0.02)
        self.assertNotEqual(row["seenAt"], "", "the server shows it: no page shows it too")
        self.assertEqual(calls, [("mac", "Order filled · QNC", "Bought 5 at 1.75")])
        self.assertEqual(store.list_notifications(unseen=True), [], "nothing left for a page")

    def test_each_system_is_asked_in_its_own_words(self):
        runs = []
        def fake_run(cmd, **kw):
            runs.append((cmd, kw.get("env") or {}))
            return mock.Mock(returncode=0, stderr=b"")
        notify.configure(url="http://127.0.0.1:8799/", icon=os.path.join(os.path.dirname(os.path.dirname(os.path.abspath(__file__))), "favicon.png"))
        with mock.patch.object(notify.subprocess, "run", side_effect=fake_run), mock.patch.object(notify, "mac_app", return_value="/x/Bagholder.app"):
            self.assertTrue(notify.deliver("mac", "Stopped out · QNC", "Sold 5 at 1.64"))
        self.assertEqual(runs[-1][0], ["open", "-n", "-W", "--env", "BAGHOLDER_TITLE=Stopped out · QNC", "--env", "BAGHOLDER_BODY=Sold 5 at 1.64", "/x/Bagholder.app"], "the applet reads its words from the environment; a click on the banner runs it without any and it opens the app")
        self.assertIn('open location "http://127.0.0.1:8799/"', notify._mac_script())
        with mock.patch.object(notify.subprocess, "run", side_effect=fake_run), mock.patch.object(notify, "mac_app", return_value=None):
            self.assertTrue(notify.deliver("mac", "T", "B"))
        self.assertEqual((runs[-1][0][0], runs[-1][1]["BAGHOLDER_TITLE"], runs[-1][1]["BAGHOLDER_BODY"]), ("osascript", "T", "B"), "without the applet, the system's plain notification")
        with mock.patch.object(notify.subprocess, "run", side_effect=fake_run), mock.patch.object(notify, "_windows_register"), mock.patch.object(notify.shutil, "which", lambda n: "powershell.exe" if n == "powershell" else None):
            self.assertTrue(notify.deliver("windows", "T", "B"))
        cmd, env = runs[-1]
        self.assertEqual((cmd[0], cmd[1:7], env["BAGHOLDER_TITLE"]), ("powershell.exe", ["-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass", "-WindowStyle", "Hidden"], "T"))
        self.assertIn("CreateToastNotifier('Bagholder')", cmd[-1])
        self.assertIn('launch="http://127.0.0.1:8799/"', cmd[-1], "a click on the toast opens the app")
        with mock.patch.object(notify.subprocess, "run", side_effect=fake_run):
            self.assertTrue(notify.deliver("linux", "T", "B"))
        self.assertEqual(runs[-1][0][:2] + runs[-1][0][-2:], ["notify-send", "--app-name=Bagholder", "T", "B"])
        self.assertTrue(runs[-1][0][2].startswith("--icon="))
        self.assertFalse(notify.deliver("", "T", "B"))

    @unittest.skipUnless(sys.platform == "darwin" and os.path.exists("/usr/bin/osacompile"), "the applet is built with macOS's own tools")
    def test_the_mac_applet_is_built_once_under_bagholders_name_and_icon(self):
        notify.configure(url="http://127.0.0.1:8799/", icon=os.path.join(os.path.dirname(os.path.dirname(os.path.abspath(__file__))), "favicon.png"))
        app = notify.mac_app()
        self.assertIsNotNone(app)
        self.assertEqual(str(app), os.path.join(self.home, "Bagholder.app"))
        plist = subprocess.run(["plutil", "-p", str(app / "Contents" / "Info.plist")], capture_output=True, text=True).stdout
        self.assertIn('"CFBundleName" => "Bagholder"', plist)
        self.assertIn('"CFBundleIdentifier" => "com.bagholder.notifier"', plist)
        self.assertNotIn("CFBundleIconName", plist, "the stock asset catalogue gives way to the app's own icon file")
        self.assertTrue((app / "Contents" / "Resources" / "applet.icns").stat().st_size > 10000, "the icon built from the favicon")
        self.assertFalse((app / "Contents" / "Resources" / "Assets.car").exists())
        self.assertIn('open location "http://127.0.0.1:8799/"', (app / "Contents" / "Resources" / "Scripts").exists() and notify._mac_script())
        stamp = (app / "Contents" / "Resources" / "bagholder.stamp").read_text()
        with mock.patch.object(notify, "_mac_build", side_effect=AssertionError("built again")):
            self.assertEqual(notify.mac_app(), app, "already built: not built again")
        notify.configure(url="http://127.0.0.1:8800/")
        self.assertNotEqual(notify._mac_stamp(), stamp, "a new address means a new applet")

    def test_a_new_filing_on_a_chosen_ticker_is_told_but_a_first_read_is_a_baseline(self):
        notify.set_settings({"disclosuresHeld": True, "disclosuresWatched": True, "disclosuresAll": True})
        held = [{"symbol": "QNC", "exchange": "NYSE", "currency": "USD", "kind": "Shares"}, {"symbol": "QNC 20NOV26 3.00 CALL", "exchange": "NYSE", "currency": "USD", "kind": "Options"}, {"symbol": "BTC", "exchange": "", "currency": "CAD", "kind": "Crypto"}]
        watched = [{"symbol": "SHOP", "exchange": "TSX", "name": "Shopify Inc.", "currency": "CAD"}]
        base = {"today": "2026-09-14", "positions": [], "trades": [
            {"symbol": "ENB", "exchange": "TSX", "currency": "CAD", "kind": "Shares", "entryDate": "2026-05-01", "exitDate": "2026-06-01"},
            {"symbol": "OLD", "exchange": "TSX", "currency": "CAD", "kind": "Shares", "entryDate": "2024-05-01", "exitDate": "2024-06-01"},
            {"symbol": "QNC 20NOV26 3.00 CALL", "exchange": "NYSE", "currency": "USD", "kind": "Options", "entryDate": "2026-08-01", "exitDate": "2026-09-01"}]}
        listings = {"QNC": [{"id": "sec:1", "source": "sec", "type": "8-K", "title": "Current report", "date": "2026-09-10"}],
                    "SHOP": [{"id": "sedar:1", "source": "sedar", "type": "Material change report", "title": "x", "date": "2026-09-10"}]}
        def fake_refresh(sym, name=None, exchange=None, currency=None):
            for src in ("sec", "sedar"):
                rows = [r for r in listings[sym] if r["source"] == src]
                if rows:
                    store.replace_filings(sym, src, rows)
            store.mark_filings_fetched(sym)
            return len(listings[sym])
        listings["ENB"] = [{"id": "sedar:9", "source": "sedar", "type": "Annual report", "title": "a", "date": "2026-09-01"}]
        with mock.patch.object(model, "base_model", return_value=base), mock.patch.object(model, "held_symbols", return_value=held), mock.patch.object(store, "list_watchlist", return_value=watched), \
             mock.patch.object(disclosures, "providers_for", return_value=[object()]), mock.patch.object(bagholder, "refresh_filings", side_effect=fake_refresh):
            self.assertEqual([i["symbol"] for i in bagholder.known_filing_symbols()], ["QNC", "ENB", "OLD", "SHOP"], "a contract and a coin have no filer; every trade the book ever closed counts, an option trade for its underlying")
            self.assertEqual([i["symbol"] for i in bagholder.known_filing_symbols(("watched",))], ["SHOP"], "each set is its own choice")
            self.assertEqual([i["symbol"] for i in bagholder.known_filing_symbols(("held",))], ["QNC"])
            listings["OLD"] = []
            self.assertEqual(bagholder.sweep_filings(), 0, "the first read is the baseline")
            self.assertEqual(store.list_notifications(), [])
            listings["QNC"].append({"id": "sec:2", "source": "sec", "type": "8-K", "title": "Another", "date": "2026-09-14"})
            listings["SHOP"].extend([{"id": "sedar:2", "source": "sedar", "type": "News release", "title": "y", "date": "2026-09-14"}, {"id": "sedar:3", "source": "sedar", "type": "Material change report", "title": "z", "date": "2026-09-14"}])
            self.assertEqual(bagholder.sweep_filings(), 0, "read minutes ago: left alone")
            later = datetime.now(timezone.utc) + timedelta(minutes=31)
            self.assertEqual(bagholder.sweep_filings(now=later), 2)
            self.assertEqual(bagholder.sweep_filings(now=later), 0, "told once")
        self.assertEqual([(r["kind"], r["title"], r["body"], r["extra"]) for r in store.list_notifications()], [
            ("disclosures", "New disclosure · QNC", "8-K · SEC EDGAR", {"symbol": "QNC"}),
            ("disclosures", "New disclosure · SHOP", "Material change report · SEDAR+", {"symbol": "SHOP"}),
        ], "a filed news release is the Releases kind's to tell, and that kind is off here")
        notify.set_settings({"disclosuresHeld": False, "disclosuresWatched": False, "disclosuresAll": False})
        with mock.patch.object(bagholder, "known_filing_symbols", side_effect=AssertionError("swept while off")):
            self.assertEqual(bagholder.sweep_filings(), 0, "every set off: the pipeline stays on demand")


    def test_the_feed_merges_the_stored_disclosures_of_a_set_newest_first(self):
        store.replace_filings("QNC", "sec", [{"id": "sec:1", "source": "sec", "type": "8-K", "title": "a", "date": "2026-09-10"}])
        store.replace_filings("SHOP", "sedar", [{"id": "sedar:1", "source": "sedar", "type": "News release", "title": "b", "date": "2026-09-14"}, {"id": "sedar:2", "source": "sedar", "type": "MD&A", "title": "c", "date": "2026-08-01"}])
        held = [{"symbol": "QNC", "exchange": "NYSE", "currency": "USD", "kind": "Shares"}]
        watched = [{"symbol": "SHOP", "exchange": "TSX", "name": "Shopify Inc.", "currency": "CAD"}]
        with mock.patch.object(model, "base_model", return_value={"today": "2026-09-15", "positions": [], "trades": []}), mock.patch.object(model, "held_symbols", return_value=held), \
             mock.patch.object(store, "list_watchlist", return_value=watched), mock.patch.object(disclosures, "providers_for", return_value=[object()]):
            out = bagholder.filings_feed("all")
            self.assertEqual([(r["symbol"], r["exchange"], r["id"]) for r in out["filings"]], [("SHOP", "TSX", "sedar:1"), ("QNC", "NYSE", "sec:1"), ("SHOP", "TSX", "sedar:2")], "every ticker the book knows, newest first, each row naming its listing")
            self.assertEqual([r["id"] for r in bagholder.filings_feed("holdings")["filings"]], ["sec:1"])
            self.assertEqual([r["id"] for r in bagholder.filings_feed("watchlist")["filings"]], ["sedar:1", "sedar:2"])
            self.assertEqual(bagholder.filings_feed("bogus")["scope"], "bogus")

    def test_a_re_read_takes_the_stored_profile_so_sedar_is_asked_once(self):
        calls = []
        def fake_fetch(sym, **kw):
            calls.append((sym, kw.get("profile_no")))
            return {"items": [], "sources": {"SEDAR+": {"available": True, "matched": False, "filer": False, "count": 0, "error": ""}}}
        with mock.patch.object(disclosures, "fetch", side_effect=fake_fetch), mock.patch.object(store, "list_securities", return_value=[]):
            bagholder.refresh_filings("QNC")
            store.mark_filings_fetched("QNC", "000012345")
            bagholder.refresh_filings("QNC")
        self.assertEqual(calls, [("QNC", None), ("QNC", "000012345")], "the first read resolves the issuer; every later one brings its profile")
        # the pipeline hands the profile to SEDAR+ only
        seen = {}
        with mock.patch.object(disclosures.sedar, "available", return_value=True), mock.patch.object(disclosures.sedar, "covers", return_value=True), \
             mock.patch.object(disclosures.sedar, "fetch", side_effect=lambda sym, **kw: seen.setdefault("sedar", kw) and []), \
             mock.patch.object(disclosures.edgar, "available", return_value=True), mock.patch.object(disclosures.edgar, "covers", return_value=True), \
             mock.patch.object(disclosures.edgar, "fetch", side_effect=lambda sym, **kw: seen.setdefault("edgar", kw) and []):
            disclosures.fetch("QNC", name="Quantum eMotion", exchange="TSX-V", currency="CAD", profile_no="000012345")
        self.assertEqual(seen["sedar"].get("profile_no"), "000012345")
        self.assertNotIn("profile_no", seen["edgar"])


    def test_a_re_keyed_list_and_an_empty_answer_tell_nothing_and_a_new_filing_is_known_by_what_it_is(self):
        notify.set_settings({"disclosuresWatched": True})
        watched = [{"symbol": "CH", "exchange": "TSX-V", "name": "Charbone", "currency": "CAD"}]
        docs = [{"id": "sedar:drm:a1", "source": "SEDAR+", "type": "News release", "title": "Closing 2nd Drawdown", "date": "2026-09-08T08:27", "size": "112 KB"},
                {"id": "sedar:drm:b2", "source": "SEDAR+", "type": "Interim MD&A", "title": "MDA June 2026", "date": "2026-08-27T16:49", "size": "1.2 MB"}]
        answer = {"rows": list(docs)}
        def fake_fetch(sym, **kw):
            return {"items": list(answer["rows"]), "sources": {"SEDAR+": {"available": True, "matched": bool(answer["rows"]), "filer": True, "count": len(answer["rows"]), "error": ""}}}
        later = lambda m: datetime.now(timezone.utc) + timedelta(minutes=m)
        with mock.patch.object(store, "list_watchlist", return_value=watched), mock.patch.object(disclosures, "providers_for", return_value=[object()]), \
             mock.patch.object(disclosures, "fetch", side_effect=fake_fetch), mock.patch.object(store, "list_securities", return_value=[]), mock.patch.object(sys, "stderr"):
            self.assertEqual(bagholder.sweep_filings(), 0, "the first read is the baseline")
            self.assertEqual(len(store.filings("CH")), 2)
            # the same two filings under new ids: nothing new
            answer["rows"] = [dict(d, id=d["id"] + "-again") for d in docs]
            self.assertEqual(bagholder.sweep_filings(now=later(31)), 0, "a filing is known by what it is, not by its id")
            # the source answers empty: the stored rows stand, and nothing is told
            answer["rows"] = []
            self.assertEqual(bagholder.sweep_filings(now=later(62)), 0)
            self.assertEqual(len(store.filings("CH")), 2, "a filed document never disappears")
            # a list none of whose rows were there a moment ago: a baseline again, not thirty filings in a morning
            answer["rows"] = [dict(d, id=d["id"] + "-x", title=d["title"] + " (fr)") for d in docs]
            self.assertEqual(bagholder.sweep_filings(now=later(93)), 0)
            # one filing more, the rest as before: that one is told, once
            answer["rows"] = [dict(d, title=d["title"] + " (fr)") for d in docs] + [{"id": "sedar:drm:c3", "source": "SEDAR+", "type": "Material change report", "title": "Financing", "date": "2026-09-15T09:00", "size": "80 KB"}]
            self.assertEqual(bagholder.sweep_filings(now=later(124)), 1)
            self.assertEqual(bagholder.sweep_filings(now=later(155)), 0, "told once")
        self.assertEqual([(r["title"], r["body"]) for r in store.list_notifications()], [("New disclosure · CH", "Material change report · SEDAR+")])


    def test_a_disclosure_is_told_by_the_documents_own_title_and_the_form_code_stands_in(self):
        rows = [{"id": "sec:1", "source": "SEC", "type": "144", "subject": "Proposed sale of 40,000 shares by an officer"}]
        self.assertEqual(bagholder.filings_notice("NBIS", rows),
                         ("New disclosure \u00b7 NBIS", "Proposed sale of 40,000 shares by an officer \u00b7 SEC EDGAR"))
        # no title on the row yet: the document is read for one, and that is what is told
        read = []
        def fake(sym, doc_id):
            read.append((sym, doc_id))
            return {"ok": True, "subject": "Notice of intent to sell"}
        with mock.patch.object(bagholder, "filings_enrich", side_effect=fake):
            self.assertEqual(bagholder.filings_notice("NBIS", [{"id": "sec:2", "source": "SEC", "type": "144"}]),
                             ("New disclosure \u00b7 NBIS", "Notice of intent to sell \u00b7 SEC EDGAR"))
        self.assertEqual(read, [("NBIS", "sec:2")], "read once, for the notice it is naming")
        # nothing could be read from it: the form's code stands in rather than nothing at all
        with mock.patch.object(bagholder, "filings_enrich", return_value={"ok": False}):
            self.assertEqual(bagholder.filings_notice("NBIS", [{"id": "sec:3", "source": "SEC", "type": "6-K"}]),
                             ("New disclosure \u00b7 NBIS", "6-K \u00b7 SEC EDGAR"))
        # several: counted in the title, named in the body, and only the ones it names are read
        many = [{"id": "sec:%d" % i, "source": "SEC", "type": "4", "subject": "Insider report %d" % i} for i in range(4)]
        self.assertEqual(bagholder.filings_notice("NBIS", many),
                         ("4 new disclosures \u00b7 NBIS", "Insider report 0, Insider report 1, Insider report 2 and more \u00b7 SEC EDGAR"))


    def test_a_disclosure_waits_to_be_told_by_name_rather_than_by_its_forms_code(self):
        notify.set_settings({"disclosuresWatched": True})
        watched = [{"symbol": "CH", "exchange": "TSX-V", "name": "Charbone", "currency": "CAD"}]
        old = {"id": "sedar:a0", "source": "SEDAR+", "type": "Material change report", "title": "Old", "date": "2026-09-01T09:00", "size": "1 KB"}
        rows = [old, {"id": "sedar:a1", "source": "SEDAR+", "type": "144", "title": "Notice", "date": "2026-09-08T08:27", "size": "1 KB"}]
        answer = {"rows": [old]}
        def fake_fetch(sym, **kw):
            return {"items": list(answer["rows"]), "sources": {"SEDAR+": {"available": True, "matched": True, "filer": True, "count": len(answer["rows"]), "error": ""}}}
        later = lambda m: datetime.now(timezone.utc) + timedelta(minutes=m)
        with mock.patch.object(store, "list_watchlist", return_value=watched), mock.patch.object(disclosures, "providers_for", return_value=[object()]), \
             mock.patch.object(disclosures, "fetch", side_effect=fake_fetch), mock.patch.object(store, "list_securities", return_value=[]), mock.patch.object(sys, "stderr"):
            self.assertEqual(bagholder.sweep_filings(), 0, "the first read is the baseline")
            answer["rows"] = list(rows)
            # the model that reads a document is still coming up: the list is stored, nothing is told
            with mock.patch.object(bagholder, "_can_name_documents", return_value=False):
                self.assertEqual(bagholder.sweep_filings(now=later(31)), 0)
            self.assertEqual(store.list_notifications(), [], "nothing told by a form's code")
            self.assertEqual([r["id"] for r in store.filings("CH")], ["sedar:a0"], "the ticker is left alone: a filing stored now would read as history")
            # once a document can be read, the filing is told — the stream was never consumed
            with mock.patch.object(bagholder, "filings_enrich", return_value={"subject": "Notice of proposed sale"}):
                self.assertEqual(bagholder.sweep_filings(now=later(62)), 1)
        self.assertEqual([(r["title"], r["body"]) for r in store.list_notifications()],
                         [("New disclosure \u00b7 CH", "Notice of proposed sale \u00b7 SEDAR+")])

    def test_a_hold_is_bounded_so_a_filing_is_told_late_rather_than_never(self):
        notify.set_settings({"disclosuresWatched": True})
        watched = [{"symbol": "CH", "exchange": "TSX-V", "name": "Charbone", "currency": "CAD"}]
        old = {"id": "sedar:a0", "source": "SEDAR+", "type": "Material change report", "title": "Old", "date": "2026-09-01T09:00", "size": "1 KB"}
        rows = [old, {"id": "sedar:a1", "source": "SEDAR+", "type": "144", "title": "Notice", "date": "2026-09-08T08:27", "size": "1 KB"}]
        answer = {"rows": [old]}
        def fake_fetch(sym, **kw):
            return {"items": list(answer["rows"]), "sources": {"SEDAR+": {"available": True, "matched": True, "filer": True, "count": len(answer["rows"]), "error": ""}}}
        later = lambda m: datetime.now(timezone.utc) + timedelta(minutes=m)
        with mock.patch.object(store, "list_watchlist", return_value=watched), mock.patch.object(disclosures, "providers_for", return_value=[object()]), \
             mock.patch.object(disclosures, "fetch", side_effect=fake_fetch), mock.patch.object(store, "list_securities", return_value=[]), \
             mock.patch.object(sys, "stderr"):
            self.assertEqual(bagholder.sweep_filings(), 0, "the first read is the baseline")
            answer["rows"] = list(rows)
            with mock.patch.object(bagholder, "_can_name_documents", return_value=False):
                self.assertEqual(bagholder.sweep_filings(now=later(31)), 0, "held while the document cannot be read")
                # after the hold's bound it is told by what the row already says, rather than never told
                self.assertEqual(bagholder.sweep_filings(now=later(31 + bagholder.FILINGS_HOLD_MAX_MIN + 1)), 1)
        self.assertEqual([r["body"] for r in store.list_notifications()], ["144 \u00b7 SEDAR+"])

    def test_a_wires_release_is_told_and_a_first_read_of_a_listing_is_not(self):
        notify.set_settings({"releasesHeld": True})
        base = {"today": "2026-09-15", "positions": [{"symbol": "QNC", "exchange": "TSX-V", "currency": "CAD", "kind": "Shares"}], "trades": []}
        wire = {"rows": [{"id": "tmx:1", "headline": "Quantum eMotion Reports Record Quarter", "source": "TMX Newsfile", "url": "u", "publishedAt": "2026-09-15T12:00:00Z", "kind": "release"},
                         {"id": "tmx:2", "headline": "Why QNC is up", "source": "The Motley Fool", "url": "u", "publishedAt": "2026-09-15T12:00:00Z", "kind": "story"}]}
        listings = [("QNC", "TSX-V", "CAD")]
        with mock.patch.object(model, "base_model", return_value=base), mock.patch.object(news, "fetch_symbol", side_effect=lambda s, e, c, ctx=None, now=None: ("tmx", list(wire["rows"]))), \
             mock.patch.object(bagholder, "news_listings", return_value=listings), mock.patch.object(bagholder, "_ssl_context", return_value=None):
            bagholder.refresh_news()                      # the listing's first read: what it already carries, not news
            self.assertEqual(store.list_notifications(), [])
            wire["rows"].append({"id": "tmx:3", "headline": "Quantum eMotion Wins Certification", "source": "GlobeNewswire", "url": "u", "publishedAt": "2026-09-15T13:00:00Z", "kind": "release"})
            with mock.patch.object(news, "stale", return_value=listings):
                bagholder.refresh_news()
                rows = store.list_notifications()
                bagholder.refresh_news()               # the same release again: told once
        self.assertEqual([(r["kind"], r["title"], r["body"]) for r in rows],
                         [("releases", "Press release · QNC", "Quantum eMotion Wins Certification")], "the wire's release, not the story beside it")
        self.assertEqual(len(store.list_notifications()), 1)

    def test_a_release_outside_the_chosen_sets_is_not_told(self):
        notify.set_settings({"releasesWatched": True})
        base = {"today": "2026-09-15", "positions": [{"symbol": "QNC", "exchange": "TSX-V", "currency": "CAD", "kind": "Shares"}], "trades": []}
        old = {"id": "tmx:8", "headline": "older", "kind": "release", "publishedAt": "2026-09-14T13:00:00Z"}
        new = {"id": "tmx:9", "headline": "h", "kind": "release", "publishedAt": "2026-09-15T13:00:00Z"}
        with mock.patch.object(model, "base_model", return_value=base), mock.patch.object(store, "list_watchlist", return_value=[]):
            self.assertFalse(bagholder.in_release_scope("QNC"), "held, while only the watchlist is chosen")
            bagholder.note_wire_releases("QNC", "TSX-V", [old], {"tmx:8"})
        self.assertEqual(store.list_notifications(), [])
        notify.set_settings({"releasesHeld": True})
        with mock.patch.object(model, "base_model", return_value=base):
            self.assertTrue(bagholder.in_release_scope("QNC"))
            bagholder.note_wire_releases("QNC", "TSX-V", [old], {"tmx:8"})          # the wire met for the first time: its history
            self.assertEqual(store.list_notifications(), [])
            bagholder.note_wire_releases("QNC", "TSX-V", [old, new], {"tmx:9"})     # what it carries after that
        self.assertEqual([r["title"] for r in store.list_notifications()], ["Press release · QNC"])

    def test_a_filed_release_is_told_only_where_no_wire_carried_one(self):
        notify.set_settings({"releasesWatched": True, "disclosuresWatched": True})
        watched = [{"symbol": "BIGG", "exchange": "CSE", "name": "BIGG", "currency": "CAD"}]
        docs = {"BIGG": [{"id": "sedar:1", "source": "sedar", "type": "News release", "title": "x", "date": "2026-09-10"}]}
        def fake_refresh(sym, name=None, exchange=None, currency=None):
            store.replace_filings(sym, "sedar", docs[sym]); store.mark_filings_fetched(sym); return len(docs[sym])
        later = lambda m: datetime.now(timezone.utc) + timedelta(minutes=m)
        with mock.patch.object(model, "base_model", return_value={"today": "2026-09-15", "positions": [], "trades": []}), \
             mock.patch.object(store, "list_watchlist", return_value=watched), mock.patch.object(disclosures, "providers_for", return_value=[object()]), \
             mock.patch.object(bagholder, "refresh_filings", side_effect=fake_refresh):
            self.assertEqual(bagholder.sweep_filings(), 0, "the first read is the baseline")
            docs["BIGG"].append({"id": "sedar:2", "source": "sedar", "type": "News release", "title": "y", "date": "2026-09-15"})
            self.assertEqual(bagholder.sweep_filings(now=later(31)), 1)
            told = store.list_notifications()
            # the same release once a wire has carried one for this ticker: the wire told it, the record does not again
            docs["BIGG"].append({"id": "sedar:3", "source": "sedar", "type": "News release", "title": "z", "date": "2026-09-15"})
            store.replace_news("BIGG", "CSE", "tmx", [{"id": "tmx:1", "headline": "z", "source": "TMX Newsfile", "url": "u", "publishedAt": "2026-09-15T00:00:00Z", "kind": "release"}])
            self.assertEqual(bagholder.sweep_filings(now=later(62)), 0)
        self.assertEqual([(r["kind"], r["title"]) for r in told], [("releases", "Press release · BIGG")],
                         "a ticker no wire carries is told from the record, under Releases and not Disclosures")


    def test_a_stream_met_for_the_first_time_shows_nothing_and_never_shows_its_past(self):
        """The one rule every feed is told through: what a stream held when it was met is history."""
        at = lambda i: i["at"]
        held = [{"id": "a", "at": "2026-05-01"}, {"id": "b", "at": "2026-06-01"}]
        self.assertEqual(notify.fresh_since("s1", held, at), [], "met for the first time: nothing, whatever it holds")
        self.assertEqual(store.get_meta("notify_seen:s1"), "2026-06-01|b", "and the mark is set from it, with what stood at that moment")
        self.assertEqual(notify.fresh_since("s1", held, at), [], "the same again: still nothing")
        later = held + [{"id": "c", "at": "2026-07-01"}]
        self.assertEqual([i["id"] for i in notify.fresh_since("s1", later, at)], ["c"], "what comes after the mark")
        self.assertEqual(notify.fresh_since("s1", later, at), [], "and never again, without the caller having to remember")
        # a sibling filed at the same moment as the newest is not lost; a backfill dated before the mark is not told
        both = later + [{"id": "d", "at": "2026-07-01"}, {"id": "old", "at": "2026-02-01"}]
        self.assertEqual([i["id"] for i in notify.fresh_since("s1", both, at)], ["d"])
        self.assertEqual(notify.fresh_since("s1", both, at), [])
        # every stream carries its own mark, and it is kept in the store, so a restart does not replay one
        self.assertEqual(notify.fresh_since("s2", held, at), [])
        self.assertEqual(sorted(k for k in ("notify_seen:s1", "notify_seen:s2") if store.get_meta(k)), ["notify_seen:s1", "notify_seen:s2"])

    def test_a_source_read_for_the_first_time_brings_history_not_news(self):
        """QNC had SEC rows alone; SEDAR+ matched the issuer for the first time and brought thirty
        documents going back months. A source's own history is not news, whatever the ticker's is."""
        notify.set_settings({"disclosuresWatched": True})
        watched = [{"symbol": "QNC", "exchange": "TSX-V", "name": "Quantum eMotion", "currency": "CAD"}]
        sec = [{"id": "sec:1", "source": "SEC", "type": "6-K", "title": "a", "date": "2026-08-14"}]
        sedar = [{"id": "sedar:%d" % i, "source": "SEDAR+", "type": "Other Correspondence", "title": "t%d" % i, "date": "2026-0%d-14T10:00" % (5 + i)} for i in range(3)]
        holds = {"SEC": list(sec), "SEDAR+": []}
        def fake_refresh(sym, name=None, exchange=None, currency=None):
            for src, rows in holds.items():
                store.replace_filings(sym, src, rows)
            store.mark_filings_fetched(sym)
            return sum(len(r) for r in holds.values())
        later = lambda m: datetime.now(timezone.utc) + timedelta(minutes=m)
        with mock.patch.object(model, "base_model", return_value={"today": "2026-09-15", "positions": [], "trades": []}), \
             mock.patch.object(store, "list_watchlist", return_value=watched), mock.patch.object(disclosures, "providers_for", return_value=[object()]), \
             mock.patch.object(bagholder, "refresh_filings", side_effect=fake_refresh):
            self.assertEqual(bagholder.sweep_filings(), 0, "the ticker's first read is the baseline")
            holds["SEDAR+"] = list(sedar)          # a second regulator matches for the first time
            self.assertEqual(bagholder.sweep_filings(now=later(31)), 0, "its back catalogue is history, not news")
            self.assertEqual(store.list_notifications(), [])
            holds["SEDAR+"] = sedar + [{"id": "sedar:9", "source": "SEDAR+", "type": "Material change report", "title": "new", "date": "2026-09-15T09:00"}]
            self.assertEqual(bagholder.sweep_filings(now=later(62)), 1, "what it files after that is news")
            holds["SEC"] = sec + [{"id": "sec:0", "source": "SEC", "type": "6-K", "title": "old", "date": "2026-02-01"}]
            self.assertEqual(bagholder.sweep_filings(now=later(93)), 0, "a filing older than what that source already had is not news")
        self.assertEqual([(r["title"], r["body"]) for r in store.list_notifications()],
                         [("New disclosure · QNC", "Material change report · SEDAR+")])


if __name__ == "__main__":
    unittest.main()
