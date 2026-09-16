"""Filings whose facts sit on the page in a fixed shape, read exactly rather than summarized.

A regulator's fill-in form is mostly its own instructions: a Report of Exempt Distribution
runs ten pages of guidance around a handful of filled boxes, and a model handed that text
summarizes the guidance — which is how a report of a $1,500,000 placement came back as a
sentence about National Instrument 81-106, the rule the form's own instructions cite. Two
rules keep that from happening again:

  a form this module knows      is read here, value by value, and never reaches a model;
  a form it does not know       is not summarized at all. The row shows what the document
                                is, which its type already says, rather than a guess.

Everything here is the document's own text. Nothing is inferred, and a value that is not
found is left out of the sentence rather than filled in.
"""
from __future__ import annotations

import re

# what only a fill-in form says: its own instructions, in the words regulators write them in
FORM_MARKS = (re.compile(r"\(YYYY\s*-\s*MM\s*-\s*DD\)", re.I),
              re.compile(r"\brefer to (?:part|section|item)\b", re.I),
              re.compile(r"\bselect (?:only )?one\b", re.I),
              re.compile(r"\bcomplete (?:schedule|item|part)\b", re.I),
              re.compile(r"\bif applicable\b", re.I),
              re.compile(r"\bcheck (?:the )?box\b", re.I),
              re.compile(r"\bdo not complete\b", re.I),
              re.compile(r"\bof the [Ii]nstructions\b"))
FORM_MARK_MIN = 3          # three of the eight, so prose that happens to say "if applicable" is not a form

_MONEY = r"\$?\s*([\d,]+(?:\.\d+)?)"


def is_form(text):
    """Whether a document is a regulator's fill-in form rather than something written."""
    body = text or ""
    return sum(1 for m in FORM_MARKS if m.search(body)) >= FORM_MARK_MIN


def _num(s):
    try:
        return float(str(s).replace(",", ""))
    except (TypeError, ValueError):
        return None


def _money(n):
    """A dollar amount as the app writes one, without cents it does not have."""
    if n is None:
        return ""
    return "$" + ("{:,.0f}".format(n) if float(n).is_integer() or n >= 1000 else "{:,.2f}".format(n))


def _date(text, label):
    """A form's date boxes: `Start date 2026 YYYY 09 08 MM DD`."""
    m = re.search(label + r"\s*(\d{4})\s*YYYY\s*(\d{1,2})\s*(\d{1,2})\s*MM", text, re.I)
    return "%s-%02d-%02d" % (m.group(1), int(m.group(2)), int(m.group(3))) if m else ""


def _day(iso):
    """`2026-09-08` as `8 September 2026`, the way the app writes a date in a sentence."""
    months = ("January", "February", "March", "April", "May", "June", "July", "August", "September", "October", "November", "December")
    try:
        y, m, d = (int(x) for x in iso.split("-"))
        return "%d %s %d" % (d, months[m - 1], y)
    except (ValueError, IndexError):
        return ""


def read_45_106f1(text):
    """Form 45-106F1, Report of Exempt Distribution: what was raised, from how many
    purchasers, on what date, under which exemption. Each value is the form's own."""
    amount = None
    m = re.search(r"Total dollar amount of securities distributed\s*" + _MONEY, text, re.I)
    if m:
        amount = _num(m.group(1))
    buyers = None
    m = re.search(r"Total number of unique\s*(?:purchasers)?\s*(\d[\d,]*)", text, re.I)
    if m:
        buyers = _num(m.group(1))
    when = _date(text, r"Start date") or _date(text, r"End date")
    exemption = ""
    m = re.search(r"NI\s*45-106\s*([\d.]+)\s*\[([^\]]{3,60})\]", text)
    if m:
        exemption = "NI 45-106 %s (%s)" % (m.group(1), m.group(2).strip().lower())
    if amount is None and buyers is None:
        return {}
    parts = []
    if amount is not None:
        parts.append(_money(amount) + " distributed")
    if buyers is not None:
        parts.append("%d purchaser%s" % (int(buyers), "" if buyers == 1 else "s"))
    head = " from ".join(parts) if len(parts) == 2 else parts[0]
    if when:
        head += " on " + _day(when)
    if exemption:
        head += ", under " + exemption
    # the title says what the document is about, not what it is: the row's Document column
    # already carries the form's name, and a title repeating it tells a reader nothing
    subject = "Exempt distribution of " + _money(amount) if amount is not None else ""
    return {"subject": subject, "summary": head + "."}


# a form is claimed by the words on its own first page, not by the type a source gave the row
READERS = ((re.compile(r"Form\s*45-106F1|Report of Exempt Distribution", re.I), read_45_106f1),)


def read(text):
    """The document read exactly where this module knows its form, {} otherwise."""
    body = text or ""
    for mark, reader in READERS:
        if mark.search(body[:4000]):
            try:
                out = reader(body)
            except Exception:
                out = {}
            if out.get("summary"):
                return out
    return {}
