//! The fourteen conditions of an `${if}` statement.
//!
//! The ten jdbgen has compare text, every one of them ignoring case except
//! `matches`, which is a regular expression the **whole** value has to match.
//! The four rudbgen adds - `gt`, `ge`, `lt` and `le` - compare whole numbers.

use crate::error::RenderError;
use crate::model::Value;
use crate::strutil;
use regex::Regex;

/// The condition attributes an `if` accepts, in the order jdbgen registers
/// them and rudbgen's numeric ones behind. `value` is a second name of
/// `equals`.
pub(crate) const COND_NAMES: [&str; 15] = [
    "equals",
    "value",
    "notequals",
    "contains",
    "notcontains",
    "startswith",
    "notstartswith",
    "endswith",
    "notendswith",
    "matches",
    "notmatches",
    "gt",
    "ge",
    "lt",
    "le",
];

/// One kind of condition.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum CondKind {
    Equals,
    NotEquals,
    Contains,
    NotContains,
    StartsWith,
    NotStartsWith,
    EndsWith,
    NotEndsWith,
    Matches,
    NotMatches,
    Gt,
    Ge,
    Lt,
    Le,
}

impl CondKind {
    /// The condition an attribute name stands for, already lower cased by the
    /// attribute parser.
    pub(crate) fn from_name(name: &str) -> Option<CondKind> {
        Some(match name {
            "equals" | "value" => CondKind::Equals,
            "notequals" => CondKind::NotEquals,
            "contains" => CondKind::Contains,
            "notcontains" => CondKind::NotContains,
            "startswith" => CondKind::StartsWith,
            "notstartswith" => CondKind::NotStartsWith,
            "endswith" => CondKind::EndsWith,
            "notendswith" => CondKind::NotEndsWith,
            "matches" => CondKind::Matches,
            "notmatches" => CondKind::NotMatches,
            "gt" => CondKind::Gt,
            "ge" => CondKind::Ge,
            "lt" => CondKind::Lt,
            "le" => CondKind::Le,
            _ => return None,
        })
    }

    /// The attribute name of a numeric comparison, or nothing for a condition
    /// that compares text.
    fn numeric_name(self) -> Option<&'static str> {
        match self {
            CondKind::Gt => Some("gt"),
            CondKind::Ge => Some("ge"),
            CondKind::Lt => Some("lt"),
            CondKind::Le => Some("le"),
            _ => None,
        }
    }

    /// Whether this condition is the negation of another one.
    fn negated(self) -> bool {
        matches!(
            self,
            CondKind::NotEquals
                | CondKind::NotContains
                | CondKind::NotStartsWith
                | CondKind::NotEndsWith
                | CondKind::NotMatches
        )
    }
}

/// One condition of a parsed `if`, with its regular expression already
/// compiled and its number already read.
///
/// jdbgen compiles the pattern on every evaluation; compiling it once keeps a
/// template that is rendered against a hundred tables from paying for it a
/// hundred times. It also moves the verdict on a pattern that does not
/// compile from the render pass, where jdbgen reports it, to the parse: the
/// pattern is the same for every table, and a run must not write the files of
/// the tables rendered before the first one that reached it. A numeric
/// condition that is given no number is the same kind of mistake.
#[derive(Debug)]
pub(crate) struct Cond {
    pub(crate) kind: CondKind,
    pub(crate) value: String,
    pub(crate) regex: Option<Regex>,
    pub(crate) number: Option<i64>,
}

impl Cond {
    /// A condition, or the message a pattern that does not compile or a
    /// numeric condition without a number fails with.
    pub(crate) fn new(kind: CondKind, value: String) -> Result<Cond, String> {
        let regex = match kind {
            CondKind::Matches | CondKind::NotMatches => {
                // Pattern.matches() asks the whole value to match
                Some(
                    Regex::new(&format!("^(?:{value})$"))
                        .map_err(|e| format!("invalid regular expression '{value}': {e}"))?,
                )
            }
            _ => None,
        };
        let number = match kind.numeric_name() {
            Some(name) => Some(strutil::parse_int(&value).ok_or_else(|| {
                format!("'{name}' in if statement requires a whole number, but got '{value}'")
            })?),
            None => None,
        };
        Ok(Cond {
            kind,
            value,
            regex,
            number,
        })
    }

    /// Evaluate this condition against the value the key chain produced.
    pub(crate) fn eval(&self, val: &Value<'_>) -> Result<bool, RenderError> {
        let held = match self.kind {
            CondKind::Contains | CondKind::NotContains => self.contains(val)?,
            CondKind::Matches | CondKind::NotMatches => self
                .regex
                .as_ref()
                .expect("compiled for a matches condition")
                .is_match(val.to_text().as_ref()),
            CondKind::Equals | CondKind::NotEquals => {
                strutil::eq_ignore_case(&self.value, val.to_text().as_ref())
            }
            CondKind::StartsWith | CondKind::NotStartsWith => val
                .to_text()
                .to_lowercase()
                .starts_with(&self.value.to_lowercase()),
            CondKind::EndsWith | CondKind::NotEndsWith => val
                .to_text()
                .to_lowercase()
                .ends_with(&self.value.to_lowercase()),
            CondKind::Gt | CondKind::Ge | CondKind::Lt | CondKind::Le => self.compare(val),
        };
        Ok(held != self.kind.negated())
    }

    /// A numeric comparison. A value that is no whole number - the `keySeq`
    /// of a column outside the key, a text field - fails every one of them,
    /// the way a comparison with NULL fails in SQL: `gt` and `le` are both
    /// false for it, rather than one of them holding by accident.
    fn compare(&self, val: &Value<'_>) -> bool {
        let bound = self.number.expect("read for a numeric condition");
        let number = match val {
            Value::Int(number) => *number,
            Value::Str(text) => match strutil::parse_int(text) {
                Some(number) => number,
                None => return false,
            },
            _ => return false,
        };
        match self.kind {
            CondKind::Gt => number > bound,
            CondKind::Ge => number >= bound,
            CondKind::Lt => number < bound,
            CondKind::Le => number <= bound,
            _ => unreachable!("not a numeric condition"),
        }
    }

    /// `contains` proper: a collection holds the value when one of its
    /// elements is named so, a text when it equals one of the comma separated
    /// alternatives. Anything else is a template bug rather than a false
    /// condition.
    fn contains(&self, val: &Value<'_>) -> Result<bool, RenderError> {
        match val {
            Value::List(items) => Ok(items.iter().any(|item| match item.get("name") {
                Some(name) => strutil::eq_ignore_case(name.to_text().as_ref(), &self.value),
                None => false,
            })),
            Value::Str(text) => Ok(strutil::split_trim(&self.value, ',')
                .iter()
                .any(|alt| strutil::eq_ignore_case(text.as_ref(), alt))),
            _ => Err(RenderError::new(
                "contains/notcontains in if statement item must be a collection object or a ',' separated string.",
            )),
        }
    }
}
