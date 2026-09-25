//! How a distribution is paid, as far as its source says.

/// Whether the source states the form a distribution is paid in (cash, units
/// or both, as its row says), or states only an amount per unit and not whether
/// it is paid in cash or in units (an exchange's record). An unstated form is
/// found from the record: the account's own payment for it (`SPEC.md` §2,
/// Distribution rate).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Form {
    Stated,
    Unstated,
}

impl Form {
    pub fn as_str(self) -> &'static str {
        match self {
            Form::Stated => "stated",
            Form::Unstated => "unstated",
        }
    }

    pub fn parse(s: &str) -> Option<Form> {
        match s {
            "stated" => Some(Form::Stated),
            "unstated" => Some(Form::Unstated),
            _ => None,
        }
    }
}
