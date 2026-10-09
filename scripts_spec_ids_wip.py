"""Give SPEC.md one rule per line, each with a permanent id, changing no word.

  python3 spec_ids.py convert SPEC.md        # rewrites the file in place
  python3 spec_ids.py same OLD.md NEW.md     # the text is the same, ids and line breaks aside

A rule is a sentence of a paragraph, a list item or a table row. Each goes on
its own line beginning with an invisible anchor, <a id="<section>-<n>"></a>,
placed after a list marker or a table's first pipe. Headings, blank lines and
table separators carry none. Sentences split onto their own lines stay in the
same paragraph or list item when rendered (a soft line break), so the page reads
exactly as before.
"""
import re
import sys

ANCHOR = re.compile(r'<a id="[^"]+"></a>')
ABBREV = ("e.g.", "i.e.", "vs.", "etc.", "Inc.", "Corp.", "Ltd.", "No.", "U.S.", "St.", "approx.", "cf.")


def slug(heading: str) -> str:
    t = re.sub(r"^#+\s*", "", heading)
    t = re.sub(r"^\d+\.\s*", "", t)
    t = re.sub(r"[^a-z0-9]+", "-", t.lower()).strip("-")
    return t or "spec"


def sentences(text: str):
    """Split at a sentence's end: '. ' (or ! ?) before a capital, a quote, a
    bracket, a backtick or bold; never inside backticks or after an abbreviation."""
    out, start, i, tick = [], 0, 0, False
    while i < len(text):
        c = text[i]
        if c == "`":
            tick = not tick
        elif not tick and c in ".!?" and i + 1 < len(text) and text[i + 1] == " ":
            j = i + 2
            nxt = text[j] if j < len(text) else ""
            before = text[start : i + 1]
            if (nxt.isupper() or nxt in "(`\"*_[") and not before.endswith(ABBREV) and not re.search(r"(\b[A-Z]\.|§\d+\.|\d\.)$", before):
                out.append(text[start : i + 1])
                start = j
                i = j
                continue
        i += 1
    out.append(text[start:])
    return [s for s in out if s.strip()]


def convert(lines):
    section = "spec"
    counter = {}
    out = []

    def next_id():
        counter[section] = counter.get(section, 0) + 1
        return f"{section}-{counter[section]}"

    for line in lines:
        raw = line.rstrip("\n")
        if not raw.strip():
            out.append(raw)
            continue
        if raw.lstrip().startswith("#"):
            section = slug(raw)
            out.append(raw)
            continue
        if re.match(r"^\s*\|[\s:|-]+\|\s*$", raw):
            out.append(raw)
            continue
        if raw.lstrip().startswith("|"):
            # a table row is one rule: its anchor opens the first cell
            m = re.match(r"^(\s*\|\s*)(.*)$", raw)
            out.append(f'{m.group(1)}<a id="{next_id()}"></a>{m.group(2)}')
            continue
        m = re.match(r"^(\s*(?:[-*+]|\d+\.)\s+)(.*)$", raw)
        lead, body = (m.group(1), m.group(2)) if m else (re.match(r"^\s*", raw).group(0), raw.lstrip())
        parts = sentences(body)
        cont = " " * len(lead)
        for k, p in enumerate(parts):
            out.append(f'{lead if k == 0 else cont}<a id="{next_id()}"></a>{p}')
    return out


def stripped(text: str) -> str:
    text = ANCHOR.sub("", text)
    return re.sub(r"\s+", " ", text).strip()


if __name__ == "__main__":
    if sys.argv[1] == "convert":
        path = sys.argv[2]
        with open(path) as f:
            lines = f.readlines()
        new = convert(lines)
        with open(path, "w") as f:
            f.write("\n".join(new) + "\n")
    elif sys.argv[1] == "same":
        a, b = (open(p).read() for p in sys.argv[2:4])
        if stripped(a) != stripped(b):
            sa, sb = stripped(a), stripped(b)
            n = next(i for i, (x, y) in enumerate(zip(sa, sb)) if x != y)
            print("differs at", n, repr(sa[n - 60 : n + 60]), repr(sb[n - 60 : n + 60]))
            sys.exit(1)
        print("same text: only ids and line breaks differ")
