import unittest
import disclosures as D


class RecategorizeTest(unittest.TestCase):
    """A stored row's category is re-derived from its type on read, so a stale stored
    category (an F-X saved as Offerings by older logic) is corrected without re-fetch."""
    def test_stale_category_is_corrected(self):
        import disclosures as D
        row = {"source": "SEC", "type": "F-X", "category": "Offerings"}
        self.assertEqual(D.categorize(row), D.OTHER)
        self.assertEqual(D.categorize({"source":"SEC","type":"F-1","category":"Other"}), D.OFFERINGS)
    def test_unknown_source_keeps_stored(self):
        import disclosures as D
        self.assertEqual(D.categorize({"source":"???","type":"X","category":"Financials"}), "Financials")


if __name__ == "__main__":
    unittest.main()
