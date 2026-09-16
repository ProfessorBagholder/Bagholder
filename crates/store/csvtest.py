"""Differential test for csvimport.py against crates/store/src/csvimport.rs.

Every helper is run on hand-picked and random inputs; whole files in all three
layouts -- random, messy, and the edges each layout has -- are parsed by both
and compared report for report, the uuid4 ids aside. Imports are then run
into two copies of the real database and the stored rows compared, and a
watched folder is scanned by both through edits, removals and a forced scan.
"""
import json
import os
import random
import shutil
import subprocess
import sys
import tempfile

sys.path.insert(0, "/Users/md/dev/Bagholder")
import csvimport  # noqa: E402
import store  # noqa: E402

HERE = os.path.dirname(os.path.abspath(__file__))
TOOL = os.path.join(HERE, "..", "..", "target", "release", "csvtool")


def run_rust(payload, mode=""):
    p = subprocess.run([TOOL] + ([mode] if mode else []), input=json.dumps(payload), capture_output=True, text=True)
    if p.returncode != 0:
        raise SystemExit("csvtool failed: %s" % p.stderr[-3000:])
    return json.loads(p.stdout)


def diff(label, want, got, bad):
    if want != got:
        bad.append("%s\n  python: %s\n  rust:   %s" % (label, json.dumps(want)[:700], json.dumps(got)[:700]))


def strip_ids(rep):
    for a in rep.get("activities") or []:
        a["id"] = ""
    return rep


def main():
    rnd = random.Random(20260916)
    bad, n = [], 0
    pick = rnd.choice

    numbers = ["", " ", "-", "—", "N/A", "n/a", "1", "1,234.50", "$1,234.50", "(12.5)", "( 3 )", "CAD 5", "5 USD", "usd7", "£3",
               "€2", "1e3", "nan", "inf", "(inf)", "abc", "12-", "1.2.3", "()", "(", "١٢", "１２", " 7 ", "1_000", "--1", "+5",
               "$(5)", "(5", "5)", "CADUSD", "0x1F", "  (1,000.00)  "]
    for _ in range(300):
        numbers.append("".join(pick(["1", "2", ".", ",", "$", "(", ")", "-", " ", "CAD", "e", "9", "0"]) for _ in range(rnd.randint(0, 7))))
    dates = ["", "2026-01-02", "2026-01-02T10:00:00Z", "2026-01-02 10:00", "2026-1-2", "2026/01/02", "2026.1.2 x", "02-Jan-2026",
             "2 jan 2026", "Jan 02, 2026", "jan-2-2026", "Feb 30 2026", "13/01/2026", "01/13/2026", "01/02/2026", "1.2.2026 9:00",
             "45000", "45000.5", "45001.5", "20000", "80000", "79999.9", "4500", "123456", "2026-01-02x", "Foo 01, 2026",
             "٢٠٢٦-٠١-٠٢", "2026-13-40", "32/13/2026", "2026-01-02\tx", "  2026-01-02  ", "44927.5", "44926.5"]
    for _ in range(300):
        dates.append("".join(pick(["2026", "-", "/", ".", "01", "1", "13", "Jan", " ", ",", "45000", "T", "x"]) for _ in range(rnd.randint(1, 6))))
    headers = ["Transaction Date", "﻿date", "'Net-Cash Amount'", '"unit price"', "  ACTIVITY_sub-type ", "a  - b", "", " x "]
    formats = [["transaction_date", "x"], ["Date", "Transaction", "Description", "Amount"], ["Date", "Action", "Symbol", "Quantity", "Price"],
               ["date", "action"], ["action"], ["Activity Type"], ["unit_price", "net_cash_amount", "activity sub type"], [], ["foo"]]
    words = ["fx", "FXEXCHANGE", "Trade", "buy", "SELL", "dividend", "Deposit", "withdrawal", "interest", "fee", "transfer", "expiry",
             "exercise", "assignment", "", "other", "Fx Exchange", "fx-exchange"]
    categories = [[pick(words), pick(words)] for _ in range(200)]
    descriptions = ["AAPL - Apple Inc.: Bought 10 shares at $150.25 per share (executed at 2026-01-02)",
                    "SHOP - Shopify: Sold -5 shares at 100 per share", "AAPL 260117C00100000: Bought 2 contracts (executed at 2026-01-05)",
                    "Apple Inc: 1 share", "XIU: Dividend", "no colon here", "AAPL - Apple", "A B  C: x", "1abc: 3 shares", "",
                    "VFV - Vanguard S&P 500 Index ETF: Bought 1,000.5 shares at $1,234.56 per share", "x: -3 contracts", "(executed at 2026-02-30)",
                    "TSLA - Tesla: Sold 3 SHARES AT $2 PER SHARE ( Executed At 2026-03-04 )", "é - thing: 3 shares", "BRK.B - Berkshire: 1 share"]
    type_codes = ["", "BUY", "SELL", "buy", "DIV", "Dividend", "CONT", "WD", "INT", "INTCHARGED", "TRFIN", "FXCONVERSION", "FEE", "FCHRG",
                  "STKDIS", "SPIN OFF", "ROC", "LOAN", "RECALL", "EXPIRY", "ASSIGN", "EXERCISE", "MARKET BUY", "SHORT SELL", "OTHER", "comm"]
    type_descs = ["", "Return of capital", "Bought 3", "sold 2 FX", "distribution", "interest paid", "transfer out", "fx conversion",
                  "fee", "commission", "deposit", "withdrawal", "sell", "roc payment", "selling"] + descriptions[:4]
    types = [[pick(type_codes), pick(type_descs)] for _ in range(300)] + [[c, ""] for c in type_codes]
    books = ["activities-HQ1234567CAD-2026-01-02.csv", "/tmp/x/hq1234567usd.csv", "short12CAD.csv", "plain.csv", "a/b/c", "", "ABCDEFGH12CAD-2026-1-2"]
    footers = ["As of 2026-01-02", "  as OF 2026-01-02 x", "as of 2026-1-2", "Total as of 2026-01-02", "AS  OF\t2026-01-02"]

    def math_ok(x):
        import math
        return math.isfinite(csvimport.parse_number(x))

    def cell(v):
        v = str(v)
        return '"%s"' % v.replace('"', '""') if any(ch in v for ch in ',"\n') or rnd.random() < 0.1 else v

    def csv_text(header, rows, nl="\n"):
        return nl.join(",".join(cell(c) for c in r) for r in [header] + rows) + pick(["", nl, nl + nl])

    finite = [x for x in numbers if math_ok(x)]
    csvs = []
    for k in range(60):
        kind = k % 3
        rows = []
        for _ in range(rnd.randint(0, 25)):
            if kind == 0:
                rows.append([pick(dates), pick(dates + [""]), pick(words), pick(words), pick(["AAPL", "", "XIU"]), pick(finite), pick(finite),
                             pick(finite), pick(["cad", "USD", ""]), pick(["", "hq1"]), pick(descriptions)])
            elif kind == 1:
                rows.append([pick(dates), pick(type_codes), pick(descriptions + type_descs), pick(finite), pick(finite + [""]), pick(["", "usd", "EUR"])])
            else:
                rows.append([pick(dates), pick(words + ["BUY", "sell", "Reinvest"]), pick(["AAPL", ""]), pick(finite), pick(finite), pick(finite)])
            if rnd.random() < 0.08:
                rows.append([""] * rnd.randint(0, 3))
            if rnd.random() < 0.05:
                rows.append(["as of 2026-01-02"])
        header = [["transaction_date", "settlement_date", "activity_type", "activity_sub_type", "symbol", "quantity", "unit_price",
                   "net_cash_amount", "currency", "account_id", "description"],
                  ["Date", "Transaction", "Description", "Amount", "Balance", "Currency"],
                  ["Date", "Action", "Symbol", "Quantity", "Price", "Amount"]][kind]
        name = pick(["export.csv", "activities-HQ1234567CAD-2026-01-02.csv", "statement-ABC12345USD.csv"])
        csvs.append([name, pick(["", "﻿"]) + csv_text(header, rows, pick(["\n", "\r\n"])) + pick(["", "\nAs of 2026-09-01 12:00"])])
    csvs += [["e.csv", ""], ["e.csv", "\n\n,,\n"], ["u.csv", "foo,bar\n1,2"], ["q.csv", "Date,Action\n\"2026-01-01\"x\ry,buy"],
             ["s.csv", "Date,Transaction,Description,Amount\n2026-01-02,BUY,\"AAPL - Apple: Bought 1 share\",-100\n2026-01-03,,,\n"],
             ["d.csv", "Date,Date,Action\n2026-01-01,bad,buy\n"], ["x.csv", "Date,Action\n" + "2026-01-01,buy\n" * 3 + "as of 2026-01-01\n,\n"]]

    payload = {"numbers": numbers, "dates": dates, "headers": headers, "formats": formats, "categories": categories,
               "descriptions": descriptions, "types": types, "books": books, "footers": footers, "csvs": csvs}
    got = run_rust(payload)

    import math
    for i, v in enumerate(numbers):
        want = csvimport.parse_number(v)
        # the one deliberate difference: a number that is not one reads as 0
        if not math.isfinite(want):
            want = 0.0
        diff("parse_number(%r)" % v, want, got["numbers"][i], bad)
    for i, v in enumerate(dates):
        diff("parse_date(%r)" % v, csvimport.parse_date(v), got["dates"][i], bad)
    for i, v in enumerate(headers):
        diff("normalize_header(%r)" % v, csvimport.normalize_header(v), got["headers"][i], bad)
    for i, v in enumerate(formats):
        diff("detect_format(%r)" % v, csvimport.detect_format(v), got["formats"][i], bad)
    for i, v in enumerate(categories):
        diff("categorize%r" % v, csvimport.categorize(*v), got["categories"][i], bad)
    for i, v in enumerate(descriptions):
        diff("description(%r)" % v, {"instrument": list(csvimport.extract_instrument(v)), "parsed": csvimport.parse_statement_description(v)},
             got["descriptions"][i], bad)
    for i, v in enumerate(types):
        diff("map_statement_type%r" % v, list(csvimport.map_statement_type(*v)), got["types"][i], bad)
    for i, v in enumerate(books):
        diff("book_id(%r)" % v, csvimport.book_id_from_file_name(v), got["books"][i], bad)
    for i, v in enumerate(footers):
        diff("is_footer_line(%r)" % v, csvimport.is_footer_line(v), got["footers"][i], bad)
    for i, (name, text) in enumerate(csvs):
        try:
            want = strip_ids(csvimport.parse_csv(text, name))
        except Exception:
            want = "raised"
        diff("parse_csv[%d] %s" % (i, name), json.loads(json.dumps(want)), got["csvs"][i], bad)
    n += sum(len(v) for v in payload.values())

    # imports into two copies of the real database
    live = os.environ.get("BAGHOLDER_DB") or os.path.expanduser("~/.bagholder/bagholder.db")
    work = tempfile.mkdtemp(prefix="csvtest-")
    try:
        py_home, rs_home = os.path.join(work, "py"), os.path.join(work, "rs")
        os.makedirs(py_home)
        os.makedirs(rs_home)
        shutil.copy(live, os.path.join(py_home, "bagholder.db"))
        shutil.copy(live, os.path.join(rs_home, "bagholder.db"))
        store.set_home(py_home)
        # each file twice, so the second pass is all duplicates
        files = [c for c in csvs if c[1]] + [c for c in csvs[:10] if c[1]]
        py_reports = []
        for name, text in files:
            try:
                py_reports.append(csvimport.import_text(name, text))
            except Exception:
                py_reports.append("raised")
        rs_reports = run_rust({"db": os.path.join(rs_home, "bagholder.db"), "files": files}, "import")
        for i, (a, b) in enumerate(zip(py_reports, rs_reports)):
            diff("import_text[%d] %s" % (i, files[i][0]), json.loads(json.dumps(a)), b, bad)
        n += len(files)
        import sqlite3

        def local_rows(path):
            con = sqlite3.connect(path)
            con.row_factory = sqlite3.Row
            cols = [r[1] for r in con.execute("PRAGMA table_info(activities)")]
            rows = [dict(r) for r in con.execute("SELECT * FROM activities ORDER BY rowid")]
            con.close()
            for r in rows:
                for k in ("id", "raw", "payload", "created_at", "updated_at", "imported_at"):
                    if k in r:
                        r[k] = None
                if isinstance(r.get("data"), str):
                    try:
                        d = json.loads(r["data"])
                        d.pop("id", None)
                        r["data"] = d
                    except ValueError:
                        pass
            return cols, rows
        store.close_all() if hasattr(store, "close_all") else None
        pc, pr = local_rows(os.path.join(py_home, "bagholder.db"))
        rc, rr = local_rows(os.path.join(rs_home, "bagholder.db"))
        diff("activities columns", pc, rc, bad)
        diff("activities count", len(pr), len(rr), bad)
        for i, (a, b) in enumerate(zip(pr, rr)):
            if a != b:
                diff("stored activity %d" % i, a, b, bad)
                break
        n += len(pr)
        print("  imports: %d files, %d stored activities compared" % (len(files), len(pr)))

        # a watched folder, both scanning their own copy of the same files
        folders = {}
        steps_for = {}
        for side in ("py", "rs"):
            folder = os.path.join(work, "watch-" + side)
            os.makedirs(folder)
            folders[side] = folder
        def steps(folder):
            return [
                {"op": "status"},
                {"op": "scan"},
                {"op": "set", "path": os.path.join(folder, "nope")},
                {"op": "set", "path": folder},
                {"op": "write", "path": os.path.join(folder, "a.csv"), "text": csvs[0][1]},
                {"op": "write", "path": os.path.join(folder, "b.CSV"), "text": csvs[1][1]},
                {"op": "write", "path": os.path.join(folder, "._junk.csv"), "text": csvs[2][1]},
                {"op": "write", "path": os.path.join(folder, "empty.csv"), "text": ""},
                {"op": "write", "path": os.path.join(folder, "notes.txt"), "text": "x"},
                {"op": "scan"},
                {"op": "scan"},
                {"op": "status"},
                {"op": "write", "path": os.path.join(folder, "a.csv"), "text": csvs[0][1] + "\n" + csvs[3][1].split("\n", 1)[-1]},
                {"op": "scan"},
                {"op": "remove", "path": os.path.join(folder, "b.CSV")},
                {"op": "scan", "force": True},
                {"op": "status"},
                {"op": "clear"},
            ]
        py_out = []
        for s in steps(folders["py"]):
            op = s["op"]
            if op == "set":
                py_out.append(csvimport.set_watch_folder(s["path"]))
            elif op == "scan":
                py_out.append(csvimport.scan_folder(s.get("folder"), s.get("force", False)))
            elif op == "status":
                py_out.append(csvimport.status())
            elif op == "clear":
                csvimport.clear_watch_folder()
                py_out.append(csvimport.status())
            elif op == "write":
                open(s["path"], "w").write(s["text"])
                py_out.append(None)
            elif op == "remove":
                os.remove(s["path"])
                py_out.append(None)
        rs_steps = steps(folders["rs"])
        # the two sides write their files at the same moment, so mtimes agree
        rs_out = run_rust({"db": os.path.join(rs_home, "bagholder.db"), "steps": rs_steps}, "scan")

        def norm(v, side):
            text = json.dumps(v).replace(folders[side], "<folder>")
            v = json.loads(text)
            def walk(x):
                if isinstance(x, dict):
                    return {k: ("<t>" if k in ("scannedAt", "lastScan") and x[k] else walk(y)) for k, y in x.items()}
                if isinstance(x, list):
                    return [walk(y) for y in x]
                return x
            return walk(v)
        for i, (a, b) in enumerate(zip(py_out, rs_out)):
            diff("watch step %d %s" % (i, rs_steps[i]["op"]), norm(a, "py"), norm(b, "rs"), bad)
        n += len(py_out)
    finally:
        shutil.rmtree(work, ignore_errors=True)

    print("%d comparisons, %d differing" % (n, len(bad)))
    for b in bad[:25]:
        print(b)
    return 1 if bad else 0


if __name__ == "__main__":
    sys.exit(main())
