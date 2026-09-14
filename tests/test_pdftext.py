"""The PDF text engine: it never installs or shells out during tests (disabled),
routes non-PDF bytes to nothing, and uses the extractor when one is available. No
real pip install, poppler, or network."""
from __future__ import annotations

import unittest

import pdftext


class DisabledTest(unittest.TestCase):
    def setUp(self):
        self._d = pdftext.DISABLED
        pdftext.DISABLED = True

    def tearDown(self):
        pdftext.DISABLED = self._d

    def test_disabled_returns_empty_and_never_provisions(self):
        self.assertEqual(pdftext.text(b"%PDF-1.7 hello"), "")

    def test_ensure_is_a_noop_when_disabled(self):
        pdftext.ensure()   # must not raise or start a thread/install
        self.assertTrue(True)


class RouteTest(unittest.TestCase):
    def setUp(self):
        self._d = pdftext.DISABLED
        pdftext.DISABLED = False
        self._ex = pdftext._extract_text
        self._which = pdftext.shutil.which
        pdftext.shutil.which = lambda name: None   # pretend no system pdftotext

    def tearDown(self):
        pdftext.DISABLED = self._d
        pdftext._extract_text = self._ex
        pdftext.shutil.which = self._which

    def test_non_pdf_bytes_yield_no_text(self):
        pdftext._extract_text = lambda: (_ for _ in ()).throw(AssertionError("should not be called"))
        self.assertEqual(pdftext.text(b"<html>not a pdf</html>"), "")

    def test_uses_the_extractor_when_available(self):
        pdftext._extract_text = lambda: (lambda fh: "Extracted body text.")
        self.assertEqual(pdftext.text(b"%PDF-1.7 ..."), "Extracted body text.")

    def test_a_broken_extractor_is_swallowed(self):
        def boom():
            def _x(fh):
                raise ValueError("bad pdf")
            return _x
        pdftext._extract_text = boom
        self.assertEqual(pdftext.text(b"%PDF-1.7 ..."), "")


if __name__ == "__main__":
    unittest.main()
