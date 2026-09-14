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
def html_text(data):
    """Readable text from a SEC filing's HTML."""
    s = data.decode("utf-8", "replace") if isinstance(data, (bytes, bytearray)) else (data or "")
    s = re.sub(r"(?is)<(script|style|head)[^>]*>.*?</\1>", " ", s)
    s = _TAGS.sub(" ", s)
    return _WS.sub(" ", _html.unescape(s)).strip()


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
_PROMPT = ("You are labelling a regulatory filing for an investor's dashboard. In one plain "
           "sentence under 25 words, state what this filing announces or contains. No preamble, "
           "no 'This filing', just the substance.\n\nFILING:\n%s\n\nONE SENTENCE:")


def summary_available():
    """Whether a local model is up and ready right now."""
    return localmodel.available()


def summary_status():
    """Provisioning state for the UI and for retry decisions: while either the model
    or the PDF engine is still being fetched, report a not-ready state so a row is
    tried again once both are up. off / downloading / starting / ready / failed."""
    if pdftext.pending():
        return "downloading"
    return localmodel.status()


def summarize(text):
    """One-sentence summary of a filing's text from the app's local model, or "" (no
    text, or the model is not up yet — asking kicks it off in the background). Never
    raises."""
    text = (text or "").strip()
    if not text:
        return ""
    out = localmodel.chat(_PROMPT % text[:MAX_TEXT], max_tokens=90)
    out = re.sub(r"<\|[^>]*\|>", " ", out)             # drop any chat-template special tokens
    out = _WS.sub(" ", out).strip().strip('"').strip()
    m = re.match(r"(.+?[.!?])(\s|$)", out)             # keep it to one sentence
    return (m.group(1) if m else out)[:240]


def enrich_document(source, data, content_type=""):
    """{subject, summary} for one downloaded document. The subject is always
    attempted (no model needed); the summary is attempted too — it comes back empty
    until the local model is ready, and asking starts it provisioning."""
    subject = extract_pdf_subject(data) if (data[:5] == b"%PDF-" if isinstance(data, (bytes, bytearray)) else False) else ""
    summary = summarize(document_text(data, content_type))
    return {"subject": subject, "summary": summary}
