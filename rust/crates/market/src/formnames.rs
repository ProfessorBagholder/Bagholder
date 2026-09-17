//! What a regulator's form is called, so a filing carries its own name the
//! moment it is listed.
//!
//! Every form the SEC accepts has a title on the cover of the form itself.
//! Read from the code, a row says what it is with no download and no model:
//! "Form 4: Statement of changes in beneficial ownership". A current report
//! (8-K, 6-K) says nothing until its items are known, so its name is refined
//! from the document's own item lines when it is read.

use std::collections::HashMap;
use std::sync::OnceLock;

/// The form's own title, by the code EDGAR lists it under.
fn names() -> &'static HashMap<&'static str, &'static str> {
    static M: OnceLock<HashMap<&'static str, &'static str>> = OnceLock::new();
    M.get_or_init(|| {
        [
            ("1-A", "Offering statement"),
            ("1-K", "Annual report (Regulation A)"),
            ("1-U", "Current report (Regulation A)"),
            ("10-D", "Asset-backed issuer distribution report"),
            ("10-K", "Annual report"),
            ("10-KT", "Transition annual report"),
            ("10-Q", "Quarterly report"),
            ("10-QT", "Transition quarterly report"),
            ("11-K", "Annual report of an employee stock plan"),
            ("13F-HR", "Institutional holdings report"),
            ("13F-NT", "Institutional holdings notice"),
            ("144", "Notice of proposed sale of securities"),
            ("15-12B", "Notice of deregistration"),
            ("15-12G", "Notice of deregistration"),
            ("20-F", "Annual report of a foreign private issuer"),
            ("24F-2NT", "Annual notice of securities sold"),
            ("25", "Notification of delisting"),
            ("25-NSE", "Notification of delisting"),
            ("3", "Initial statement of beneficial ownership"),
            ("305B2", "Designation of a trustee"),
            ("4", "Statement of changes in beneficial ownership"),
            ("40-F", "Annual report of a Canadian issuer"),
            ("425", "Communication about a business combination"),
            ("5", "Annual statement of beneficial ownership"),
            ("6-K", "Report of a foreign private issuer"),
            ("8-A12B", "Registration of a class of securities"),
            ("8-A12G", "Registration of a class of securities"),
            ("8-K", "Current report"),
            ("8-K12B", "Current report of a successor issuer"),
            ("ABS-EE", "Asset-backed securities exhibits"),
            ("ARS", "Annual report to shareholders"),
            ("CERT", "Exchange certification"),
            ("CORRESP", "Correspondence with the Commission"),
            ("D", "Notice of an exempt offering"),
            ("DEF 14A", "Proxy statement"),
            ("DEFA14A", "Additional proxy material"),
            ("DEFM14A", "Proxy statement for a merger"),
            ("DEFR14A", "Revised proxy statement"),
            ("DEFS14A", "Proxy statement for a special meeting"),
            ("EFFECT", "Notice that a registration is effective"),
            ("F-1", "Registration statement of a foreign private issuer"),
            ("F-3", "Registration statement of a foreign private issuer"),
            ("F-4", "Registration statement for a business combination"),
            ("FWP", "Free writing prospectus"),
            ("NT 10-K", "Notification of a late annual report"),
            ("NT 10-Q", "Notification of a late quarterly report"),
            ("NT 20-F", "Notification of a late annual report"),
            ("PRE 14A", "Preliminary proxy statement"),
            ("PREM14A", "Preliminary proxy statement for a merger"),
            ("POS AM", "Post-effective amendment to a registration statement"),
            ("RW", "Withdrawal of a registration statement"),
            ("S-1", "Registration statement"),
            ("S-3", "Registration statement"),
            ("S-4", "Registration statement for a business combination"),
            ("S-8", "Registration statement for an employee plan"),
            ("S-8 POS", "Post-effective amendment for an employee plan"),
            ("SC 13D", "Beneficial ownership report"),
            ("SC 13E3", "Going-private transaction statement"),
            ("SC 13G", "Beneficial ownership report"),
            ("SC 14D9", "Recommendation on a tender offer"),
            ("SC TO-C", "Communication about a tender offer"),
            ("SC TO-I", "Issuer tender offer statement"),
            ("SC TO-T", "Third-party tender offer statement"),
            ("SD", "Specialized disclosure report"),
            ("SCHEDULE 13D", "Beneficial ownership report"),
            ("SCHEDULE 13G", "Beneficial ownership report"),
            ("UPLOAD", "Letter from the Commission's staff"),
        ]
        .into_iter()
        .collect()
    })
}

/// A prospectus rule the code spells out: 424B1 through 424B8 and 424A.
fn prospectus(code: &str) -> Option<&'static str> {
    if code.starts_with("424") { Some("Prospectus") } else { None }
}

/// The name of the form a code stands for, or None for a code that is not
/// known. `4/A` is the amendment of `4`, and says so.
pub fn name_of(code: &str) -> Option<String> {
    let code = code.trim().to_uppercase();
    if code.is_empty() {
        return None;
    }
    let (base, amended) = match code.strip_suffix("/A") {
        Some(b) => (b.trim().to_string(), true),
        None => (code.clone(), false),
    };
    let name = names().get(base.as_str()).copied().or_else(|| prospectus(&base))?;
    Some(if amended { format!("{} (amended)", name) } else { name.to_string() })
}

/// `Form 4: Statement of changes in beneficial ownership`, the title a listed
/// filing carries before anything has been read. None when the code is not
/// one of the regulator's own, and for a current report, whose items say what
/// it is (see `items_title`).
pub fn title_of(code: &str) -> Option<String> {
    let code = code.trim().to_uppercase();
    if code.starts_with("8-K") || code.starts_with("6-K") {
        return None;
    }
    any_title(&code)
}

/// The form's name whatever the form, current reports included: what a row
/// says when its items are not known.
pub fn any_title(code: &str) -> Option<String> {
    let code = code.trim().to_uppercase();
    let name = name_of(&code)?;
    let shown = code.trim_end_matches("/A").trim().to_string();
    Some(format!("Form {}: {}", shown, name))
}

/// What the items of a current report are called, by their numbers.
fn item_names() -> &'static HashMap<&'static str, &'static str> {
    static M: OnceLock<HashMap<&'static str, &'static str>> = OnceLock::new();
    M.get_or_init(|| {
        [
            ("1.01", "Entry into a material agreement"),
            ("1.02", "Termination of a material agreement"),
            ("1.03", "Bankruptcy or receivership"),
            ("1.04", "Mine safety"),
            ("1.05", "Material cybersecurity incident"),
            ("2.01", "Completion of an acquisition or disposition"),
            ("2.02", "Results of operations and financial condition"),
            ("2.03", "Creation of a direct financial obligation"),
            ("2.04", "Triggering of a financial obligation"),
            ("2.05", "Costs of exit or disposal activities"),
            ("2.06", "Material impairments"),
            ("3.01", "Delisting or failure to satisfy a listing rule"),
            ("3.02", "Unregistered sale of equity securities"),
            ("3.03", "Change to the rights of security holders"),
            ("4.01", "Change of accountants"),
            ("4.02", "Statements no longer to be relied upon"),
            ("5.01", "Change in control"),
            ("5.02", "Departure or election of directors or officers"),
            ("5.03", "Change to the articles or by-laws"),
            ("5.04", "Suspension of trading under an employee plan"),
            ("5.05", "Change to the code of ethics"),
            ("5.07", "Submission of matters to a vote of security holders"),
            ("5.08", "Shareholder nominations"),
            ("6.01", "ABS informational and computational material"),
            ("7.01", "Regulation FD disclosure"),
            ("8.01", "Other events"),
            ("9.01", "Financial statements and exhibits"),
        ]
        .into_iter()
        .collect()
    })
}

/// The items a current report's own text lists, in the order it lists them,
/// ignoring the exhibit item every report carries.
pub fn items_in(text: &str) -> Vec<String> {
    static R: OnceLock<regex::Regex> = OnceLock::new();
    let re = R.get_or_init(|| regex::Regex::new(r"(?i)\bitem\s+(\d\.\d\d)\b").unwrap());
    let mut out: Vec<String> = Vec::new();
    for c in re.captures_iter(text) {
        let n = c[1].to_string();
        if n != "9.01" && !out.contains(&n) {
            out.push(n);
        }
    }
    out
}

/// `8-K: Results of operations and financial condition`, from the report's own
/// items; two items are both named, more than two leave the count. None when
/// the text lists none.
pub fn items_title(code: &str, text: &str) -> Option<String> {
    let code = code.trim().to_uppercase();
    let shown = code.trim_end_matches("/A").trim().to_string();
    let items = items_in(text);
    let named: Vec<&str> = items.iter().filter_map(|n| item_names().get(n.as_str()).copied()).collect();
    let what = match named.len() {
        0 => return None,
        1 => named[0].to_string(),
        2 => format!("{} and {}", named[0], named[1]),
        n => format!("{} and {} other items", named[0], n - 1),
    };
    Some(format!("Form {}: {}", shown, what))
}
