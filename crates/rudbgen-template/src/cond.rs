//! The ten conditions of an `${if}` statement.
//!
//! Every one of them compares ignoring case except `matches`, which is a
//! regular expression the **whole** value has to match.

use crate::error::RenderError;
use crate::model::Value;
use crate::strutil;
use regex::Regex;

/// The condition attributes an `if` accepts, in the order jdbgen registers
/// them. `value` is a second name of `equals`.
pub(crate) const COND_NAMES: [&str; 11] = [
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
            _ => return None,
        })
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
/// compiled.
///
/// jdbgen compiles the pattern on every evaluation; compiling it once keeps a
/// template that is rendered against a hundred tables from paying for it a
/// hundred times. It also moves the verdict on a pattern that does not
/// compile from the render pass, where jdbgen reports it, to the parse: the
/// pattern is the same for every table, and a run must not write the files of
/// the tables rendered before the first one that reached it.
#[derive(Debug)]
pub(crate) struct Cond {
    pub(crate) kind: CondKind,
    pub(crate) value: String,
    pub(crate) regex: Option<Regex>,
}

impl Cond {
    /// A condition, or the message a pattern that does not compile fails with.
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
        Ok(Cond { kind, value, regex })
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
        };
        Ok(held != self.kind.negated())
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
