"""Differential test for disclosures.py, edgar.py and sedar.py against their
Rust ports.

The parsers are compared on real pages and answers: the SEDAR+ pages a
Python session received walking two issuers (captured by recording its
session), EDGAR's ticker list, five issuers' submissions, accession listings
and Schedule 13 XML. The helpers get random input besides. With `--live`
both run the whole pipeline -- SEDAR+ through each side's own browser
session, EDGAR directly -- for a handful of listings, and download the same
documents, compared by hash.
"""
import json
import os
import random
import subprocess
import sys

sys.path.insert(0, "/Users/md/dev/Bagholder")
import disclosures  # noqa: E402
import edgar  # noqa: E402
import sedar  # noqa: E402

HERE = os.path.dirname(os.path.abspath(__file__))
TOOL = os.path.join(HERE, "..", "..", "target", "release", "filingtool")
FIX = os.environ.get("FILINGS_DIR") or "/tmp/filings"


def run_rust(payload, mode=""):
    p = subprocess.run([TOOL] + ([mode] if mode else []), input=json.dumps(payload), capture_output=True, text=True)
    if p.returncode != 0:
        raise SystemExit("filingtool failed: %s" % p.stderr[-3000:])
    return json.loads(p.stdout)


def diff(label, want, got, bad):
    if want != got:
        bad.append("%s\n  python: %s\n  rust:   %s" % (label, json.dumps(want)[:700], json.dumps(got)[:700]))


def main():
    rnd = random.Random(20260916)
    pages = json.load(open(os.path.join(FIX, "sedar_pages.json")))
    fx = json.load(open(os.path.join(FIX, "edgar.json")))
    bad, n = [], 0

    menu_names = [None, "", "Shopify Inc.", "Royal Bank of Canada", "shopify", "Nothing Like It", "Banque Royale"]
    all_filings = [f for p in pages for f in sedar.parse_filings(p["text"])]
    submitted = sorted({f["submitted"] for f in all_filings}) + ["", "13 Sep 2026", "1 Sept 2026 9:05", "13 sep 2026 20:42 EDT", "x 13 Sep 2026", "٣ Sep ٢٠٢٦"]
    files = sorted({f["file"] for f in all_filings}) + ["Interim MD&A - English.PDF", "News release (French)", "Material change report – French ",
                                                      "Early warning report", "45-106F1", "Proxy circular -English", "x.pdf.pdf", "", " - English"]
    ranks = []
    issuers = [r for p in pages for r in sedar.parse_reporting_issuers(p["text"])]
    for q in ["Shopify Inc.", "000037100", "royal", "Bank", "zzz"]:
        rows = [dict(r) for r in issuers]
        rnd.shuffle(rows)
        ranks.append([rows, q])
    covers = [[s, e, c] for s in ["X"] for e in ["", "TSX", "tsx-v", "NYSE", "CNSX", "NEO EXCHANGE", "LSE"] for c in ["", "CAD", "usd", "EUR"]]
    encode = [[["a b", "c&d=e"], ["é", "~._-"], ["QueryString", "Royal Bank / Banque"], ["", "+%"]]]
    subs = [{"cik": v["cik"], "sub": v["sub"]} for v in fx["subs"].values()] + [{"cik": 1, "sub": {}}, {"cik": 1, "sub": {"filings": {"recent": {"form": ["4"], "filingDate": ["2026-01-01"], "accessionNumber": ["0001-26-000001"]}}}},
                                                                                 {"cik": 2, "sub": []}]
    forms = []
    codes = ["10-K", "10-K/A", "10-Q", "8-K", "8-K/A", "20-F", "40-F", "6-K", "DEF 14A", "DEFA14A", "PRE 14A", "S-1", "F-1", "F-10", "F-X", "F-N",
             "424B4", "POS AM", "3", "4", "4/A", "5", "144", "SC 13D", "SC 13G/A", "SCHEDULE 13G", "13F-HR", "25", "25-NSE", "425", "ARS", "N-CSR",
             "CORRESP", "UPLOAD", "", "d-e-f 14a", "fef 14c", "EFFECT", "DRS"]
    for c in codes:
        for desc in ["", c, "FORM " + c, "form " + c.lower(), "<b>Annual</b>   report", "Something else"]:
            forms.append([c, desc])
    bares = ["brk.b", " shop.to ", "X.V", "A.U.TO", "RY.NE.CN", "BF.B.U", "", "ab.cn.v"]
    indexes = [{"listing": v, "primary": k.rsplit("/", 1)[-1]} for k, v in fx["indexes"].items()]
    indexes.append({"listing": {"directory": {"item": [{"name": "a.htm", "size": 10}, {"name": "b.htm", "size": 10}, {"name": "R1.htm", "size": 99},
                                                        {"name": "0001234567-26-000001.txt", "size": 999}, {"name": "x-index.html", "size": 5},
                                                        {"name": "data.xml", "size": ""}, {"name": "img.jpg", "size": 1000}]}}, "primary": "b.htm"})
    thirteen = fx["thirteen"] + [{"type": "SCHEDULE 13G/A", "xml": "<submissionType>SCHEDULE 13G/A</submissionType><reportingPersonName> A </reportingPersonName><reportingPersonName>B</reportingPersonName><issuerName>X Corp</issuerName>"},
                                 {"type": "SCHEDULE 13D", "xml": "<reportingPersonName></reportingPersonName>"}, {"type": "SCHEDULE 13D", "xml": "nothing"}]
    words = ["Shopify", "Inc.", "Royal", "Bank", "of", "Canada", "The", "Holdings", "Corp", "Trust", "Fund", "Apple", "Co", "A", "Banque", "du", "&", "Ltd", "Company", "é"]
    names = [[" ".join(rnd.choice(words) for _ in range(rnd.randint(0, 4))), " ".join(rnd.choice(words) for _ in range(rnd.randint(0, 4)))] for _ in range(300)]
    cleans = ["<p>a  <b>b</b>\n c</p>", "", "  x  ", "<not closed", "a b", "<br/>"]
    ids = [[f["url"], f["profileNo"], f["file"], f["submitted"]] for f in all_filings[:10]] + [["https://x/?drmKey=abc123&x", "", "", ""], ["no key", "000001161", "Ré", "1 Jan 2026"], ["", "", "", ""]]

    payload = {"pages": pages, "menu_names": menu_names, "isos": submitted, "files": files, "raw_filings": [[f, rnd.choice(["", "000037100"])] for f in all_filings[:40]],
               "ranks": ranks, "covers": covers, "encode": encode, "subs": subs, "forms": forms, "bares": bares, "indexes": indexes,
               "thirteen": thirteen, "names": names, "cleans": cleans, "ids": ids}
    got = run_rust(payload)

    from urllib.parse import urlencode
    for i, p in enumerate(pages):
        g = got["pages"][i]
        html = p["text"]
        diff("form_fields page %d" % i, [list(x) for x in sedar._form_fields(html)], g["form_fields"], bad)
        diff("vi_params page %d" % i, [list(x) for x in sedar._vi_params(html)], g["vi_params"], bad)
        sa = sedar._search_action(html)
        diff("search_action page %d" % i, list(sa) if sa else None, g["search_action"], bad)
        diff("issuer_menu_node page %d" % i, [sedar._issuer_menu_node(html, nm) for nm in menu_names], g["issuer_menu_node"], bad)
        diff("docs_menu_node page %d" % i, sedar._docs_menu_node(html), g["docs_menu_node"], bad)
        diff("parse_filings page %d" % i, sedar.parse_filings(html), g["filings"], bad)
        diff("parse_reporting_issuers page %d" % i, sedar.parse_reporting_issuers(html), g["issuers"], bad)
        diff("_text page %d" % i, sedar._text(html[:5000]), g["text"], bad)
        n += 8
    print("  SEDAR+ pages: %d, %d filings, %d issuer rows" % (len(pages), len(all_filings), len(issuers)))
    for i, v in enumerate(submitted):
        diff("_iso(%r)" % v, sedar._iso(v), got["isos"][i], bad)
    for i, v in enumerate(files):
        diff("split/category(%r)" % v, {"split": list(sedar._split_type_title(v)), "category": sedar._sedar_category(v)}, got["files"][i], bad)
    for i, (raw, pno) in enumerate(payload["raw_filings"]):
        diff("_to_item[%d]" % i, sedar._to_item(raw, pno), got["items"][i], bad)
    for i, (rows, q) in enumerate(ranks):
        want = [dict(r) for r in rows]
        ql = q.lower()
        want.sort(key=lambda r: (r["profileNo"] != q, ql not in r["name"].lower(), not r["name"].lower().startswith(ql)))
        diff("rank %r" % q, want, got["ranks"][i], bad)
    for i, (s_, e_, c_) in enumerate(covers):
        diff("sedar.covers%r" % ((s_, e_, c_),), [sedar.covers(s_, e_, c_)], got["covers"][i], bad)
    for i, pairs in enumerate(encode):
        diff("urlencode[%d]" % i, urlencode([tuple(p) for p in pairs]), got["encode"][i], bad)
    n += len(submitted) + len(files) + len(payload["raw_filings"]) + len(ranks) + len(covers) + len(encode)

    for i, x in enumerate(subs):
        # Python's own fetch body, from the submissions answer on
        real = edgar._get_json, edgar._ticker_map
        edgar._get_json = lambda url, _s=x["sub"]: _s
        edgar._ticker_map = lambda _c=x["cik"]: {"T": (_c, "T")}
        try:
            want = edgar.fetch("T", exchange="NYSE", limit=200)
        except Exception as ex:
            want = {"error": "%s: %s" % (type(ex).__name__, ex)}
        finally:
            edgar._get_json, edgar._ticker_map = real
        diff("edgar.fetch submissions[%d]" % i, want, got["subs"][i], bad)
        n += len(want) if isinstance(want, list) else 1
    for i, (c, desc) in enumerate(forms):
        diff("edgar category/title %r" % ((c, desc),), [edgar._category(c), edgar._title(c, desc)], got["forms"][i], bad)
    for i, v in enumerate(bares):
        diff("edgar._bare(%r)" % v, edgar._bare(v), got["bares"][i], bad)
    for i, x in enumerate(indexes):
        items = (x["listing"].get("directory", {}) or {}).get("item", []) or []
        cands = []
        try:
            for it in items:
                nm = str(it.get("name", ""))
                low = nm.lower()
                if not low.endswith((".htm", ".html", ".txt", ".xml")):
                    continue
                if "index" in low or edgar._SKIP_DOC.search(low):
                    continue
                cands.append((nm, int(it.get("size") or 0)))
            cands.sort(key=lambda c: (-c[1], c[0] != x["primary"]))
            want = None if not cands or cands[0][0] == x["primary"] else cands[0][0]
        except ValueError:
            want = None
        diff("edgar content pick[%d]" % i, want, got["picks"][i], bad)
    for i, x in enumerate(thirteen):
        real = edgar.document
        edgar.document = lambda row, _x=x["xml"]: (_x.encode(), "text/xml")
        try:
            want = edgar.enrichment({"type": x["type"], "url": "https://www.sec.gov/x"})
        finally:
            edgar.document = real
        diff("edgar enrichment[%d]" % i, want, got["thirteen"][i], bad)
    for i, (a, b) in enumerate(names):
        diff("names_match(%r, %r)" % (a, b), disclosures.names_match(a, b), got["names"][i], bad)
    for i, v in enumerate(cleans):
        diff("clean(%r)" % v, disclosures.clean(v), got["cleans"][i], bad)
    for i, r in enumerate(ids):
        diff("filing_id%r" % (r,), sedar.filing_id(*r), got["ids"][i], bad)
    n += len(forms) + len(bares) + len(indexes) + len(thirteen) + len(names) + len(cleans) + len(ids)

    if "--live" in sys.argv:
        listings = [["SHOP", "Shopify Inc.", "TSX", "CAD", ""], ["RY", "Royal Bank of Canada", "TSX", "CAD", "000001161"],
                    ["AAPL", "Apple Inc.", "NASDAQ", "USD", ""], ["TD", "Toronto-Dominion Bank", "TSX", "CAD", ""], ["ZZZQ", "", "", "", ""]]
        py = [disclosures.fetch(l[0], l[1], l[2], l[3], 200, l[4]) for l in listings]
        docs = []
        for res in py[:3]:
            for it in res["items"][:1]:
                docs.append(it)
        sec = [it for it in py[2]["items"] if it["type"] in ("8-K", "10-Q")][:1]
        docs += sec
        import hashlib
        py_docs = []
        for row in docs:
            try:
                data, ct = disclosures.document(row)
                py_docs.append({"len": len(data), "sha1": hashlib.sha1(data).hexdigest(), "ct": ct})
            except Exception as ex:
                py_docs.append({"error": str(ex)})
        rust = run_rust({"listings": listings, "documents": docs}, "live")
        for i, l in enumerate(listings):
            a, b = py[i], rust["fetch"][i]
            diff("live sources %s" % l[0], a["sources"], b["sources"], bad)
            # a SEDAR+ document link is minted per session (its node, drr and
            # view id); the drmKey, and so the item's id, is the document's own
            import re
            def unsession(items):
                return [dict(it, url=re.sub(r"node=W\d+|&drr=[^&]*|&id=[0-9a-f]+", "", it["url"])) if it["source"] == "SEDAR+" else it for it in items]
            diff("live items %s" % l[0], unsession(a["items"]), unsession(b["items"]), bad)
            n += 1 + len(a["items"])
            print("  live %-5s %s" % (l[0], {k: (v["count"], v["error"][:40]) for k, v in a["sources"].items()}))
        for i, row in enumerate(docs):
            diff("live document %s" % row["id"], py_docs[i], rust["documents"][i], bad)
            n += 1
            print("  document %-30s %s" % (row["id"][:30], py_docs[i]))

    print("%d comparisons, %d differing" % (n, len(bad)))
    for b in bad[:25]:
        print(b)
    return 1 if bad else 0


if __name__ == "__main__":
    sys.exit(main())
