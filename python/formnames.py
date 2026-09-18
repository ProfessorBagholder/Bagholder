"""What a regulator's form is called, so a filing carries its own name the
moment it is listed.

Every form the SEC accepts has a title on the cover of the form itself. Read
from the code, a row says what it is with no download and no model: "Form 4:
Statement of changes in beneficial ownership". A current report (8-K, 6-K) says
nothing until its items are known, so its name is refined from the document's
own item lines when it is read."""

import re

# The form's own title, by the code EDGAR lists it under.
NAMES = {
    "1-A": "Offering statement",
    "1-K": "Annual report (Regulation A)",
    "1-U": "Current report (Regulation A)",
    "10-D": "Asset-backed issuer distribution report",
    "10-K": "Annual report",
    "10-KT": "Transition annual report",
    "10-Q": "Quarterly report",
    "10-QT": "Transition quarterly report",
    "11-K": "Annual report of an employee stock plan",
    "13F-HR": "Institutional holdings report",
    "13F-NT": "Institutional holdings notice",
    "144": "Notice of proposed sale of securities",
    "15-12B": "Notice of deregistration",
    "15-12G": "Notice of deregistration",
    "20-F": "Annual report of a foreign private issuer",
    "24F-2NT": "Annual notice of securities sold",
    "25": "Notification of delisting",
    "25-NSE": "Notification of delisting",
    "3": "Initial statement of beneficial ownership",
    "305B2": "Designation of a trustee",
    "4": "Statement of changes in beneficial ownership",
    "40-F": "Annual report of a Canadian issuer",
    "425": "Communication about a business combination",
    "5": "Annual statement of beneficial ownership",
    "6-K": "Report of a foreign private issuer",
    "8-A12B": "Registration of a class of securities",
    "8-A12G": "Registration of a class of securities",
    "8-K": "Current report",
    "8-K12B": "Current report of a successor issuer",
    "ABS-EE": "Asset-backed securities exhibits",
    "ARS": "Annual report to shareholders",
    "CERT": "Exchange certification",
    "CORRESP": "Correspondence with the Commission",
    "D": "Notice of an exempt offering",
    "DEF 14A": "Proxy statement",
    "DEFA14A": "Additional proxy material",
    "DEFM14A": "Proxy statement for a merger",
    "DEFR14A": "Revised proxy statement",
    "DEFS14A": "Proxy statement for a special meeting",
    "EFFECT": "Notice that a registration is effective",
    "F-1": "Registration statement of a foreign private issuer",
    "F-3": "Registration statement of a foreign private issuer",
    "F-4": "Registration statement for a business combination",
    "FWP": "Free writing prospectus",
    "NT 10-K": "Notification of a late annual report",
    "NT 10-Q": "Notification of a late quarterly report",
    "NT 20-F": "Notification of a late annual report",
    "PRE 14A": "Preliminary proxy statement",
    "PREM14A": "Preliminary proxy statement for a merger",
    "POS AM": "Post-effective amendment to a registration statement",
    "RW": "Withdrawal of a registration statement",
    "S-1": "Registration statement",
    "S-3": "Registration statement",
    "S-4": "Registration statement for a business combination",
    "S-8": "Registration statement for an employee plan",
    "S-8 POS": "Post-effective amendment for an employee plan",
    "SC 13D": "Beneficial ownership report",
    "SC 13E3": "Going-private transaction statement",
    "SC 13G": "Beneficial ownership report",
    "SC 14D9": "Recommendation on a tender offer",
    "SC TO-C": "Communication about a tender offer",
    "SC TO-I": "Issuer tender offer statement",
    "SC TO-T": "Third-party tender offer statement",
    "SD": "Specialized disclosure report",
    "SCHEDULE 13D": "Beneficial ownership report",
    "SCHEDULE 13G": "Beneficial ownership report",
    "UPLOAD": "Letter from the Commission's staff",
}


def _prospectus(code):
    """A prospectus rule the code spells out: 424B1 through 424B8 and 424A."""
    return "Prospectus" if code.startswith("424") else None


def name_of(code):
    """The name of the form a code stands for, or None for a code that is not
    known. `4/A` is the amendment of `4`, and says so."""
    code = str(code or "").strip().upper()
    if not code:
        return None
    amended = code.endswith("/A")
    base = code[:-2].strip() if amended else code
    name = NAMES.get(base) or _prospectus(base)
    if not name:
        return None
    return "%s (amended)" % name if amended else name


def title_of(code):
    """`Form 4: Statement of changes in beneficial ownership`, the title a listed
    filing carries before anything has been read. None when the code is not one
    of the regulator's own, and for a current report, whose items say what it is
    (see `items_title`)."""
    code = str(code or "").strip().upper()
    if code.startswith("8-K") or code.startswith("6-K"):
        return None
    return any_title(code)


def any_title(code):
    """The form's name whatever the form, current reports included: what a row
    says when its items are not known."""
    code = str(code or "").strip().upper()
    name = name_of(code)
    if not name:
        return None
    shown = code
    while shown.endswith("/A"):
        shown = shown[:-2]
    return "Form %s: %s" % (shown.strip(), name)


# What the items of a current report are called, by their numbers.
ITEM_NAMES = {
    "1.01": "Entry into a material agreement",
    "1.02": "Termination of a material agreement",
    "1.03": "Bankruptcy or receivership",
    "1.04": "Mine safety",
    "1.05": "Material cybersecurity incident",
    "2.01": "Completion of an acquisition or disposition",
    "2.02": "Results of operations and financial condition",
    "2.03": "Creation of a direct financial obligation",
    "2.04": "Triggering of a financial obligation",
    "2.05": "Costs of exit or disposal activities",
    "2.06": "Material impairments",
    "3.01": "Delisting or failure to satisfy a listing rule",
    "3.02": "Unregistered sale of equity securities",
    "3.03": "Change to the rights of security holders",
    "4.01": "Change of accountants",
    "4.02": "Statements no longer to be relied upon",
    "5.01": "Change in control",
    "5.02": "Departure or election of directors or officers",
    "5.03": "Change to the articles or by-laws",
    "5.04": "Suspension of trading under an employee plan",
    "5.05": "Change to the code of ethics",
    "5.07": "Submission of matters to a vote of security holders",
    "5.08": "Shareholder nominations",
    "6.01": "ABS informational and computational material",
    "7.01": "Regulation FD disclosure",
    "8.01": "Other events",
    "9.01": "Financial statements and exhibits",
}

_ITEM = re.compile(r"\bitem\s+(\d\.\d\d)\b", re.I)


def items_in(text):
    """The items a current report's own text lists, in the order it lists them,
    ignoring the exhibit item every report carries."""
    out = []
    for m in _ITEM.finditer(str(text or "")):
        n = m.group(1)
        if n != "9.01" and n not in out:
            out.append(n)
    return out


def items_title(code, text):
    """`8-K: Results of operations and financial condition`, from the report's own
    items; two items are both named, more than two leave the count. None when the
    text lists none."""
    code = str(code or "").strip().upper()
    shown = code
    while shown.endswith("/A"):
        shown = shown[:-2]
    shown = shown.strip()
    named = [ITEM_NAMES[n] for n in items_in(text) if n in ITEM_NAMES]
    if not named:
        return None
    if len(named) == 1:
        what = named[0]
    elif len(named) == 2:
        what = "%s and %s" % (named[0], named[1])
    else:
        what = "%s and %d other items" % (named[0], len(named) - 1)
    return "Form %s: %s" % (shown, what)
