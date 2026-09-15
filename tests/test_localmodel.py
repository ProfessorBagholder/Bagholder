"""The local-model manager: endpoint resolution, checksum verification, chat request
shape, and graceful failure. No real download, subprocess, or network — the
provisioning steps are stubbed so nothing is fetched or executed."""
from __future__ import annotations

import hashlib
import os
import tempfile
import unittest
from unittest import mock
from pathlib import Path

import localmodel


class EndpointTest(unittest.TestCase):
    def setUp(self):
        self._detect = localmodel._detect_running
        self._ensure = localmodel.ensure
        localmodel._state.update({"phase": "off", "detail": "", "proc": None, "endpoint": "", "model": ""})

    def tearDown(self):
        localmodel._detect_running = self._detect
        localmodel.ensure = self._ensure
        localmodel._state.update({"phase": "off", "detail": "", "proc": None, "endpoint": "", "model": ""})

    def test_a_running_endpoint_is_used_and_nothing_is_provisioned(self):
        localmodel._detect_running = lambda: ("http://127.0.0.1:11434", "llama3.2")
        kicked = []
        localmodel.ensure = lambda: kicked.append(1)
        self.assertEqual(localmodel.endpoint(), "http://127.0.0.1:11434")
        self.assertEqual(localmodel.status(), "ready")
        self.assertEqual(kicked, [], "a detected endpoint must not trigger a download")

    def test_no_endpoint_kicks_provisioning_and_returns_empty(self):
        localmodel._detect_running = lambda: None
        kicked = []
        localmodel.ensure = lambda: kicked.append(1)
        self.assertEqual(localmodel.endpoint(), "")
        self.assertEqual(kicked, [1], "with nothing running, provisioning is kicked off")

    def test_status_defaults_to_off(self):
        self.assertEqual(localmodel.status(), "off")


class VerifyTest(unittest.TestCase):
    def test_a_file_is_verified_against_the_pinned_sha256(self):
        with tempfile.TemporaryDirectory() as d:
            p = Path(d) / "m.llamafile"
            p.write_bytes(b"hello world")
            saved = localmodel.LLAMAFILE_SHA256
            try:
                localmodel.LLAMAFILE_SHA256 = hashlib.sha256(b"hello world").hexdigest()
                self.assertTrue(localmodel._verified(p))
                localmodel.LLAMAFILE_SHA256 = "0" * 64
                self.assertFalse(localmodel._verified(p), "a wrong checksum is refused")
            finally:
                localmodel.LLAMAFILE_SHA256 = saved

    def test_a_missing_file_is_not_verified(self):
        self.assertFalse(localmodel._verified(Path("/no/such/file")))

    def test_download_refuses_a_host_off_the_allowlist(self):
        saved = localmodel.LLAMAFILE_URL
        try:
            localmodel.LLAMAFILE_URL = "https://evil.example.com/x.llamafile"
            with tempfile.TemporaryDirectory() as d:
                self.assertFalse(localmodel._download(Path(d) / "m"))
        finally:
            localmodel.LLAMAFILE_URL = saved


class ChatTest(unittest.TestCase):
    def setUp(self):
        self._ep = localmodel.endpoint

    def tearDown(self):
        localmodel.endpoint = self._ep

    def test_chat_is_empty_when_no_endpoint(self):
        localmodel.endpoint = lambda: ""
        self.assertEqual(localmodel.chat("hi"), "")

    def test_chat_parses_an_openai_shaped_reply(self):
        localmodel.endpoint = lambda: "http://127.0.0.1:8121"
        saved = localmodel.urlopen

        class FakeResp:
            def read(self_):
                import json
                return json.dumps({"choices": [{"message": {"content": "A concise summary."}}]}).encode()

        try:
            localmodel.urlopen = lambda *a, **k: FakeResp()
            self.assertEqual(localmodel.chat("summarize this"), "A concise summary.")
        finally:
            localmodel.urlopen = saved

    def test_chat_swallows_a_backend_error(self):
        localmodel.endpoint = lambda: "http://127.0.0.1:8121"
        saved = localmodel.urlopen
        try:
            def boom(*a, **k):
                raise OSError("connection refused")
            localmodel.urlopen = boom
            self.assertEqual(localmodel.chat("x"), "")
        finally:
            localmodel.urlopen = saved


if __name__ == "__main__":
    unittest.main()


class WaitReadyTest(unittest.TestCase):
    """Asking for a model is what starts one, so the first caller of a session would always
    get nothing. It waits for a model that is coming up — never for a download."""

    def wait(self, seconds, available, status):
        with mock.patch.object(localmodel, "endpoint", return_value=""), \
             mock.patch.object(localmodel, "available", side_effect=available), \
             mock.patch.object(localmodel, "status", side_effect=status), \
             mock.patch.object(localmodel.time, "sleep", lambda s: None):
            return localmodel.wait_ready(seconds)

    def test_it_waits_for_a_model_that_is_starting(self):
        tries = iter([False, False, True, True])
        self.assertTrue(self.wait(30, lambda: next(tries), lambda: "starting"))

    def test_it_waits_while_one_is_being_detected(self):
        tries = iter([False, True, True])
        self.assertTrue(self.wait(30, lambda: next(tries), lambda: "detecting"))

    def test_it_does_not_wait_for_a_download(self):
        self.assertFalse(self.wait(30, lambda: False, lambda: "downloading"))

    def test_it_does_not_wait_when_there_is_nothing_coming(self):
        for phase in ("off", "failed"):
            self.assertFalse(self.wait(30, lambda: False, lambda p=phase: p), phase)

    def test_a_model_already_up_is_not_waited_for_at_all(self):
        self.assertTrue(self.wait(30, lambda: True, lambda: "ready"))

    def test_no_time_to_wait_means_no_wait(self):
        self.assertFalse(self.wait(0, lambda: False, lambda: "starting"))
