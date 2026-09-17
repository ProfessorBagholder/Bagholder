"""Differential test for forms.py and enrich.py against their Rust ports.

The form reader is given the shapes its own tests use and random variations
of them; the subject reader real issuer PDFs and PDFs built around every
/Title encoding (literal, escaped, hex, UTF-16 both ways, binary that merely
decodes); the HTML reader real EDGAR documents; and the reading of a model's
answer -- the first sentence, the preamble, the hedge, the junk title --
several thousand generated answers, run through Python's own `summarize`
and `title_from_model` with the model's reply fixed. The PDF text engine is
not compared: pdfminer.six and the Rust reader lay text out differently.
"""
import json
import os
import random
import subprocess
import sys

sys.path.insert(0, os.path.abspath(os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "..")))
import enrich  # noqa: E402
import forms  # noqa: E402
import localmodel  # noqa: E402

HERE = os.path.dirname(os.path.abspath(__file__))
TOOL = os.path.join(HERE, "..", "..", "target", "release", "enrichtool")
DOCS = os.path.join(os.environ.get("FILINGS_DIR") or "/tmp/filings", "docs")
sys.path.insert(0, os.path.abspath(os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "..", "tests")))


def diff(label, want, got, bad):
    if want != got:
        bad.append("%s\n  python: %s\n  rust:   %s" % (label, json.dumps(want)[:600], json.dumps(got)[:600]))


def main():
    rnd = random.Random(20260916)
    pick = rnd.choice
    import test_forms
    F1, RELEASE = test_forms.F1, test_forms.RELEASE
    bad, n = [], 0

    texts = [F1, RELEASE, "", F1.replace("Total number of unique 1", "Total number of unique 14"),
             "Form 45-106F1 Report of Exempt Distribution Total dollar amount of securities distributed $250,000.0000",
             "Form 45-106F1 Report of Exempt Distribution and nothing else", "x" * 4100 + F1, F1.replace("$1,500,000.0000", "$999.5"),
             F1.replace("2026 YYYY 09 08 MM", "2026 YYYY 00 08 MM"), F1.replace("2026 YYYY 09 08 MM", "2026 YYYY 13 08 MM"),
             F1.replace("NI 45-106 2.10 [Minimum amount investment]", "NI 45-106 2.3 [Accredited investor]"),
             F1.replace("Total dollar amount of securities distributed $1,500,000.0000", ""),
             "report of exempt distribution total NUMBER of unique purchasers 3 start date 2026 yyyy 1 2 mm",
             "if applicable check the box do not complete", "IF APPLICABLE. Check box. refer to Part 3."]
    for _ in range(300):
        chunks = [pick(["Form 45-106F1", "Report of Exempt Distribution", "Total dollar amount of securities distributed $%s" % pick(["1,000", "12.5", "0", "7", "1,234,567.891", "999.995"]),
                        "Total number of unique %s" % pick(["purchasers 2", "1", "0", "1,000"]), "Start date %d YYYY %d %d MM" % (rnd.randint(2000, 2030), rnd.randint(0, 14), rnd.randint(0, 40)),
                        "End date 2026 YYYY 1 1 MM", "NI 45-106 %s [%s]" % (pick(["2.3", "2.10", "x"]), pick(["ab", "Accredited investor", "Family, friends and business associates"])),
                        "(YYYY-MM-DD)", "refer to item", "select one", "complete part", "if applicable", "check the box", "do not complete", "of the Instructions", "of the instructions",
                        RELEASE]) for _ in range(rnd.randint(0, 9))]
        texts.append(" ".join(chunks))

    def pdf_with(title_bytes):
        return b"%PDF-1.4\n1 0 obj << " + title_bytes + b" /Author (x) >> endobj\n%%EOF"
    titles = [b"/Title (Microsoft Word - CHARBONE - Closing 2nd Drawdown PR_FINAL_EN_2026-09-04_v6.docx)",
              b"/Title (News release)", b"/Title ()", b"/Title <FEFF0041006E006E00750061006C0020005200650070006F00720074>",
              b"/Title <FFFE41006E006E00750061006C00>", b"/Title (Paren \\( escaped \\) text_DRAFT)", b"/Title (\x12 \xf0,0 \xbfO\xf9\xff\xaf\xe2U\xc0<w\xb0)",
              b"/Title (ABCD)\n/Title <4142434445>", b"/Title(Adobe Acrobat - Q3 Results FR.pdf)", b"/Title  <414>", b"/Title (\xfe\xff\x00A\x00B\x00C)",
              b"/Title (MD&A \xe9t\xe9 2026)", b"/Title (12345 67890)", b"/Title (Exhibit 99.1 press release.htm)", b"nothing here"]
    for _ in range(200):
        words = [pick(["Q3", "Results", "Annual", "Report", "FINAL", "v2", "EN", "FR", "2026-09-04", "PR", "_", "-", "Microsoft Word -", "Acrobat -", "é", "’s", "#1", "(", ")", "\\)", "NR", "Draft"]) for _ in range(rnd.randint(0, 7))]
        s = " ".join(words)
        enc = pick(["lit", "hex", "u16"])
        if enc == "lit":
            titles.append(b"/Title (" + s.encode("latin-1", "replace") + b")")
        elif enc == "hex":
            titles.append(b"/Title <" + s.encode("latin-1", "replace").hex().encode() + b">")
        else:
            titles.append(b"/Title <" + ("feff" + s.encode("utf-16-be").hex()).encode() + b">")
    pdfs = [pdf_with(t) for t in titles]

    index = json.load(open(os.path.join(DOCS, "index.json"))) if os.path.exists(os.path.join(DOCS, "index.json")) else []
    files = [{"path": os.path.join(DOCS, x["file"])} for x in index]

    readable_in = ["", "  ", "Annual Report", "\x12 \xf0,0 \xbfO\xf9", "Q3 2026", "123 456", "Résumé des résultats", "a�", "MD&A — Q3",
                   "¿Oùÿ¯âUÀ<w°", "Ⅻ report", "² squared", "Σύνοψη", "日本語のタイトル", "ábc", "x_y_z"]
    for _ in range(400):
        readable_in.append("".join(chr(rnd.choice([rnd.randint(32, 126), rnd.randint(160, 700), rnd.randint(0x370, 0x3ff), rnd.randint(0x2000, 0x206f), rnd.randint(0, 31)])) for _ in range(rnd.randint(0, 20))))

    answers = ["Quantum eMotion Corp. announces the closing of its financing.", "Sure, here is the title: **Q3 Results and MD&A**",
               "Title: Annual information form", "The company reports J.P. Morgan as agent. It also notes more.", "Shopify Inc.",
               "<|eot_id|>Here's a summary: The company likely filed a report.", "No stop at all in this answer here",
               "It closed a deal on Sept. 3 with Acme Inc. and raised $5M. Next sentence.", "Results were strong... and more followed. End.",
               "\"Quoted\" start. “Next” one.", "- * # > Here are the details: Annual report 2026", "4 · FORM 4", "Exhibit 99.1 press release",
               "Report on form 10-Q for the quarter ended June 30, 2026.", "ok. lower continues. Upper Ends.", "Approx. 5,000 shares were sold. Then.",
               "It seems to be a report.", "The filing may be amended.", "Mr. Smith resigned as CEO effective today.", "U.S. listing approved by the NYSE. More."]
    parts = ["The company", "announces", "Corp.", "Inc.", "Ltd.", "J.", "P.", "a.b.c.", "approx.", "St.", "the closing of", "$1.5M", "financing",
             "It", "likely", "may be", "Here is the summary:", "Title:", "**", "<|im_end|>", "Sure,", ".", "!", "?", "...", "\n", "(", "\"", "“",
             "exhibit", "EX-99", "12345", "report.htm", "Quarterly results", "2026", "ÉTATS", "étape", "NYSE", "q3"]
    for _ in range(3000):
        answers.append(" ".join(pick(parts) for _ in range(rnd.randint(0, 14))))

    htmls = ["<html><head><title>x</title></head><body><p>Hello &amp; <b>world</b></p><script>var a=1;</script></body></html>",
             "EX-99.2 3 tm2615535d1_ex99-2.htm EXHIBIT 99.2 Exhibit 99.2 Press Release", "<HEADER>kept?</head> rest", "<style>x</style><Script type=x>y</SCRIPT>z",
             "<p>a</p>\n\n<p>b&nbsp;c</p>", "form 6-K exhibit 1 exhibit 2 The body", "<head>unclosed", ""]

    payload = {"texts": texts, "pdf_hex": [p.hex() for p in pdfs], "files": files, "readable": readable_in, "answers": answers, "htmls": htmls}
    p = subprocess.run([TOOL], input=json.dumps(payload), capture_output=True, text=True)
    if p.returncode != 0:
        raise SystemExit("enrichtool failed: %s" % p.stderr[-3000:])
    got = json.loads(p.stdout)

    for i, t in enumerate(texts):
        diff("forms[%d]" % i, {"is_form": forms.is_form(t), "read": forms.read(t)}, got["texts"][i], bad)
    for i, pdf in enumerate(pdfs):
        diff("extract_pdf_subject[%d] %r" % (i, titles[i][:60]), enrich.extract_pdf_subject(pdf), got["subjects"][i], bad)
    for i, x in enumerate(index):
        data = open(files[i]["path"], "rb").read()
        want = {"subject": enrich.extract_pdf_subject(data), "html_text": None if data[:5] == b"%PDF-" else enrich.html_text(data)}
        diff("document %s (%s)" % (x["file"], x["row"]["type"]), want, got["files"][i], bad)
    for i, t in enumerate(readable_in):
        diff("readable(%r)" % t, enrich.readable(t), got["readable"][i], bad)
    real_chat = localmodel.chat
    try:
        for i, a in enumerate(answers):
            localmodel.chat = lambda prompt, max_tokens=90, _a=a: _a
            want = {"first_sentence": enrich.first_sentence(a), "strip_preamble": enrich._strip_preamble(a),
                    "summary": enrich.summarize("some filing text"), "title": enrich.title_from_model("some filing text"),
                    "hedged": enrich.hedged(a), "junk": bool(enrich._is_junk_title(a))}
            diff("answer[%d] %r" % (i, a), want, got["answers"][i], bad)
    finally:
        localmodel.chat = real_chat
    for i, h in enumerate(htmls):
        diff("html_text(%r)" % h[:40], enrich.html_text(h.encode()), got["htmls"][i], bad)
    n = len(texts) + len(pdfs) + len(index) + len(readable_in) + len(answers) * 6 + len(htmls)
    print("  %d real documents (%d PDFs)" % (len(index), sum(1 for x in index if x["file"].endswith(".pdf"))))
    print("%d comparisons, %d differing" % (n, len(bad)))
    for b in bad[:25]:
        print(b)
    return 1 if bad else 0


if __name__ == "__main__":
    sys.exit(main())
