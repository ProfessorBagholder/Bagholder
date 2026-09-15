"""Reading a filing: its subject, and a concise summary, both optional and local.

Two enrichments turn the disclosures list from an index into something you can act
on without opening every document:

- **Subject** — the document's own title, which SEDAR+ hides behind a generic file
  name. Pulled from the PDF's metadata with the standard library alone, so it works
  with no setup. "News release" becomes "News release · Closing 2nd Drawdown".
- **Summary** — one plain sentence of what the filing announces, from a language
  model running locally (Ollama, or any compatible endpoint), so nothing leaves the
  machine and there is no key or bill. It needs the document's text: trivial for a
  SEC filing (HTML), and for a SEDAR+ PDF it needs `pdftotext` (poppler) on the
  path. Where the model or the text is unavailable the summary is simply empty; the
  subject still shows.

Everything here degrades to "" rather than raising, so a missing tool never breaks
the list.
"""
from __future__ import annotations

import html as _html
import re

import localmodel
import pdftext

MAX_TEXT = 8000            # characters of the filing fed to the model
_TAGS = re.compile(r"<[^>]+>")
_WS = re.compile(r"\s+")


# --------------------------------------------------------------------------- #
# Subject — the document's own title (standard library only)
# --------------------------------------------------------------------------- #
_TITLE_LIT = re.compile(rb"/Title\s*\(((?:[^()\\]|\\.)*)\)")
_TITLE_HEX = re.compile(rb"/Title\s*<([0-9A-Fa-f]+)>")
_LANG_TAIL = re.compile(r"[_\-\s]*(FINAL|DRAFT|REVISED|v\d+|EN|FR|ENG?|FRE?|English|French)\b", re.I)
_DATE = re.compile(r"\d{4}[-_]\d{2}[-_]\d{2}")


def extract_pdf_subject(data):
    """The cleaned /Title from a PDF's metadata, or "". Authoring tools leave the
    source document's name here ("Microsoft Word - CHARBONE - Closing 2nd Drawdown
    PR_FINAL_EN_2026-09-04_v6"), which carries the real subject; strip the tooling,
    the language/version/date tail and the extension down to the subject itself."""
    m = _TITLE_LIT.search(data or b"") or _TITLE_HEX.search(data or b"")
    if not m:
        return ""
    raw = m.group(1)
    if _TITLE_HEX.search(data or b"") and all(c in b"0123456789abcdefABCDEF" for c in raw) and len(raw) % 2 == 0:
        try:
            raw = bytes.fromhex(raw.decode())
        except ValueError:
            pass
    if raw[:2] == b"\xfe\xff":
        s = raw[2:].decode("utf-16-be", "replace")
    elif raw[:2] == b"\xff\xfe":
        s = raw[2:].decode("utf-16-le", "replace")
    else:
        s = raw.decode("latin-1", "replace")
    return _clean_subject(s)


def _clean_subject(s):
    s = re.sub(r"^\s*(Microsoft Word|Microsoft PowerPoint|Adobe \w+|Acrobat)\s*-\s*", "", s, flags=re.I)
    s = re.sub(r"\.(pdf|docx?|pptx?|rtf|txt)\s*$", "", s, flags=re.I)
    s = s.replace("_", " ")           # first, so word boundaries below actually fire
    s = _DATE.sub(" ", s)
    s = _LANG_TAIL.sub(" ", s)
    s = re.sub(r"\b(PR|FINAL|NR|DRAFT|REVISED|v\d+)\b", " ", s, flags=re.I)
    s = _WS.sub(" ", s).strip(" -–—·")
    # a bare file-code with no letters, or a title that is only the generic name, is no subject
    if not re.search(r"[A-Za-z]{3,}", s):
        return ""
    if s.lower() in ("news release", "press release", "document"):
        return ""
    return s


# --------------------------------------------------------------------------- #
# Text — for the model
# --------------------------------------------------------------------------- #
_SEC_HEADER = re.compile(r"^\s*[\w.\-]{1,12}\s+\d+\s+\S+\.(?:htm|html|txt|xml)\s+", re.I)
_SEC_EXLABEL = re.compile(r"^\s*(?:form\s+\S+\s+)?(?:exhibit\s+[\d.]+\s+){1,3}", re.I)


def _strip_sec_header(text):
    """Drop the EDGAR document-type header that prefixes an exhibit's text (e.g.
    'EX-99.2 3 tm2615535d1_ex99-2.htm EXHIBIT 99.2 Exhibit 99.2 ...'), so it is not
    mistaken for the document's title."""
    text = _SEC_HEADER.sub("", text or "", count=1)
    text = _SEC_EXLABEL.sub("", text, count=1)
    return text.strip()


def html_text(data):
    """Readable text from a SEC filing's HTML."""
    s = data.decode("utf-8", "replace") if isinstance(data, (bytes, bytearray)) else (data or "")
    s = re.sub(r"(?is)<(script|style|head)[^>]*>.*?</\1>", " ", s)
    s = _TAGS.sub(" ", s)
    return _strip_sec_header(_WS.sub(" ", _html.unescape(s)).strip())


def pdf_text(data):
    """Readable text from a PDF, via the pdftext engine (a system pdftotext if the
    user has one, else the auto-provisioned pdfminer.six). "" while the engine is
    still installing or if the PDF has no recoverable text; asking kicks provisioning
    in the background."""
    return _WS.sub(" ", pdftext.text(data)).strip()


def document_text(data, content_type=""):
    ct = (content_type or "").lower()
    if "pdf" in ct or (isinstance(data, (bytes, bytearray)) and data[:5] == b"%PDF-"):
        return pdf_text(data)
    return html_text(data)


# --------------------------------------------------------------------------- #
# Summary — a local model, optional
# --------------------------------------------------------------------------- #
_PROMPT = ("Below is the text of a company regulatory filing. In ONE short sentence, at most 20 words, "
           "say what it contains or announces \u2014 name the actual documents, events, or figures, not the "
           "company. If it is a cover form listing exhibits, name those exhibits. Do not restate the form "
           "type or begin with 'This filing'.\n\nFILING TEXT:\n%s\n\nSUMMARY (one sentence):")

_TITLE_PROMPT = ("Give a short, specific title for this company filing: a noun phrase of at most 8 words naming "
                 "what it is \u2014 the documents, event, or figures it contains. Not a form code, not the company "
                 "name alone, no quotes, no preamble.\n\nFILING TEXT:\n%s\n\nTitle:")


def summary_available():
    """Whether a local model is up and ready right now."""
    return localmodel.available()


SUMMARY_WAIT_SEC = 25      # how long a document read waits for a model that is starting


def wait_for_summary(seconds=SUMMARY_WAIT_SEC):
    """Give a model that is coming up the moment it needs, so the first document read in a
    session — the read that starts the model — is not the one that comes back without a
    sentence. Returns whether a summary can be made now."""
    return localmodel.wait_ready(seconds)


def summary_status():
    """Provisioning state for the UI and for retry decisions: while either the model
    or the PDF engine is still being fetched, report a not-ready state so a row is
    tried again once both are up. off / downloading / starting / ready / failed."""
    if pdftext.pending():
        return "downloading"
    return localmodel.status()


# Full stops that end an abbreviation rather than a sentence. A filing's summary nearly always
# opens by naming the issuer, and a company's name ends in one of these far more often than a
# sentence does.
_ABBREV = {"corp", "inc", "ltd", "co", "llc", "llp", "plc", "lp", "sa", "nv", "ag", "cie", "pte",
           "jr", "sr", "mr", "mrs", "ms", "dr", "prof", "st", "no", "nos", "vs", "etc", "approx", "al"}
_STOP = re.compile(r"[.!?]+(?=\s|$)")
_INITIAL = re.compile(r"(?:[a-z]\.)*[a-z]")
_LAST_WORD = re.compile(r"[\s(\[\"']")


def first_sentence(out):
    """The first sentence of the model's answer, which is not the same as the text up to its
    first full stop. `Quantum eMotion Corp. announces ...` is one sentence; cutting it at
    `Corp.` left the bare name `Quantum eMotion Corp.`, which the check below then read as no
    summary at all. So every filing whose summary opened with the issuer's name — every news
    release — showed nothing, and the ones that survived the check showed a sentence chopped
    mid-way. A stop ends a sentence only when the word before it is not an abbreviation or an
    initial and what follows begins a new one."""
    out = (out or "").strip()
    for m in _STOP.finditer(out):
        word = _LAST_WORD.split(out[:m.start()])[-1].lower().strip("\"'([")
        if m.group(0) == "." and (word in _ABBREV or _INITIAL.fullmatch(word)):
            continue
        rest = out[m.end():].lstrip()
        if rest and not (rest[0].isupper() or rest[0].isdigit() or rest[0] in '"\u201c('):
            continue
        return out[:m.end()].strip()
    return out


def summarize(text):
    """One-sentence summary of a filing's text from the app's local model, or "" (no
    text, or the model is not up yet — asking kicks it off in the background). Never
    raises."""
    text = (text or "").strip()
    if not text:
        return ""
    out = first_sentence(_strip_preamble(localmodel.chat(_PROMPT % text[:MAX_TEXT], max_tokens=90)))
    # reject a non-summary: a bare name with no statement (e.g. "Quantum eMotion Corp.").
    # A real sentence carries a lowercase word (a verb/function word); a name is all caps-cased.
    if len(out.split()) < 4 or not re.search(r"\b[a-z]{3,}\b", out):
        return ""
    return out[:240]


_PREAMBLE = re.compile(r"^\s*(sure[,!.]?\s+)?(here(?:'?s| is| are)\b[^:]*:?\s*)", re.I)
_LABEL = re.compile(r"^\s*(title|summary|answer)\s*[:\-]\s*", re.I)


def _strip_preamble(out):
    """Drop a chatty preamble a small model prepends (\"Sure, here is the title:\", \"Title:\")."""
    out = re.sub(r"<\|[^>]*\|>", " ", out or "")
    out = _WS.sub(" ", out).strip()
    out = re.sub(r"^[*#>\-\s]+", "", out)             # leading markdown / bullets
    for _ in range(2):
        out = _PREAMBLE.sub("", out)
        out = _LABEL.sub("", out)
        out = re.sub(r"^[*#>\-\s]+", "", out)
    return out.strip().strip('"').strip("*").strip()


def _is_junk_title(s):
    """True for a title that is really a file name, exhibit label, or document id
    (e.g. a PDF's '/Title' of 'PEO 75744 1'), which is no better than the form type."""
    low = (s or "").lower()
    if not low:
        return True
    return (".htm" in low or ".xml" in low or ".pdf" in low or "exhibit" in low
            or re.search(r"\bex-?\d", low) or re.search(r"\d{5,}", low))


def title_from_model(text):
    """A short title for a filing from the local model, or "" when there is no text or
    model, or the model only echoes a form code (which the row already shows as its
    type). Never raises."""
    text = (text or "").strip()
    if not text:
        return ""
    out = _strip_preamble(localmodel.chat(_TITLE_PROMPT % text[:MAX_TEXT], max_tokens=40))
    out = out.rstrip(".:").strip()
    out = " ".join(out.split()[:9])
    low = out.lower()
    if len(out.split()) < 3 or "title" in low or low.startswith("here") or _is_junk_title(out):
        return ""                        # a preamble echo, a form/file header, or a bare form code: fall back to the type
    return out[:90]


def enrich_document(source, data, content_type=""):
    """A title and a one-sentence summary for one document, both from its readable
    text. The title (`subject`) is the document's own when it exposes one — a PDF's
    metadata — else a short title from the model; the summary is one sentence from the
    model. Pass the document's *content* (edgar.content resolves a cover form to its
    exhibit) so both describe the substance, not boilerplate."""
    is_pdf = isinstance(data, (bytes, bytearray)) and data[:5] == b"%PDF-"
    subject = extract_pdf_subject(data) if is_pdf else ""
    if _is_junk_title(subject):          # a PDF's own /Title can be a file name or doc id
        subject = ""
    text = document_text(data, content_type)
    summary = summarize(text)
    if not subject:
        subject = title_from_model(text)
    return {"subject": subject, "summary": summary}
