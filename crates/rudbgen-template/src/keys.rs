//! Key chains and the string processors they are made of.
//!
//! `${name.replace('_','-').camel}` is **not** a nested path: the first step
//! names a member of the model and every following step rewrites the value the
//! previous one produced, left to right.

use crate::abbr::Abbreviations;
use crate::error::RenderError;
use crate::strutil;

/// The processor names a chain may use, in the order jdbgen lists them in its
/// error message.
const PROCESSORS: [&str; 12] = [
    "prefix",
    "suffix",
    "camel",
    "pascal",
    "snake",
    "screaming",
    "skewer",
    "kebab",
    "lower",
    "upper",
    "replace",
    "abbr",
];

/// One step of a key chain: a member name in the first step, a processor name
/// with its arguments in every following one.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct KeyStep {
    pub(crate) name: String,
    pub(crate) params: Vec<String>,
}

/// A whole key chain, as it was written in the template.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct KeyChain {
    pub(crate) steps: Vec<KeyStep>,
}

impl KeyChain {
    /// Split a key chain into its steps.
    ///
    /// Arguments may be quoted or bare; white space outside of a quoted
    /// argument is dropped. This is jdbgen's `parseKeys` without the
    /// abbreviation step, which depends on a render time setting and is
    /// therefore inserted by the renderer instead.
    pub(crate) fn parse(mkey: &str) -> KeyChain {
        // jdbgen appends the separator so that the last step needs no special
        // case; the loop below closes the last step itself
        let mut steps: Vec<KeyStep> = Vec::new();
        let mut buf = String::new();
        let mut curr_name: Option<String> = None;
        let mut params: Vec<String> = Vec::new();
        let mut is_param = false;
        let mut open: Option<char> = None;

        for c in mkey.chars() {
            if let Some(openchar) = open {
                if c == openchar {
                    params.push(std::mem::take(&mut buf));
                    open = None;
                } else {
                    buf.push(c);
                }
            } else if c == '\'' || c == '"' {
                open = Some(c);
                buf.clear();
            } else if c == '.' {
                if curr_name.is_none() {
                    curr_name = Some(std::mem::take(&mut buf));
                }
                steps.push(KeyStep {
                    name: curr_name.take().unwrap_or_default(),
                    params: std::mem::take(&mut params),
                });
                buf.clear();
            } else if c == '(' {
                curr_name = Some(std::mem::take(&mut buf));
                is_param = true;
            } else if is_param {
                if c == ')' || c == ',' {
                    // a quoted argument was collected when its quote closed,
                    // which leaves the buffer empty here
                    if !buf.is_empty() {
                        params.push(std::mem::take(&mut buf));
                    }
                    buf.clear();
                    if c == ')' {
                        is_param = false;
                    }
                } else if !strutil::is_space(c) {
                    buf.push(c);
                }
            } else if !strutil::is_space(c) {
                buf.push(c);
            }
        }
        // the trailing step jdbgen's appended '.' would have closed
        if curr_name.is_some() || !buf.is_empty() || !params.is_empty() {
            steps.push(KeyStep {
                name: curr_name.unwrap_or(buf),
                params,
            });
        }
        KeyChain { steps }
    }

    /// The member of the model this chain reads, already trimmed.
    pub(crate) fn key(&self) -> &str {
        match self.steps.first() {
            Some(step) => strutil::trim(&step.name),
            None => "",
        }
    }

    /// Whether the automatic `abbr` step applies to this chain, that is
    /// whether it reads a member called `name`.
    pub(crate) fn takes_auto_abbr(&self) -> bool {
        match self.steps.first() {
            Some(step) => strutil::trim(&step.name).eq_ignore_ascii_case("name"),
            None => false,
        }
    }
}

/// Run one processor over a value.
///
/// The name is matched ignoring case, as jdbgen lower cases it before the
/// lookup.
pub(crate) fn apply(
    name: &str,
    value: &str,
    params: &[String],
    abbr: &Abbreviations,
) -> Result<String, RenderError> {
    let proc = strutil::trim(name).to_lowercase();
    Ok(match proc.as_str() {
        "prefix" => match value.rfind('_') {
            Some(idx) => value[..idx].to_string(),
            None => value.to_string(),
        },
        "suffix" => match value.find('_') {
            Some(idx) => value[idx + 1..].to_string(),
            None => value.to_string(),
        },
        "camel" => strutil::to_camel_case(value),
        "pascal" => strutil::to_pascal_case(value),
        "snake" => strutil::to_snake_case(value),
        "screaming" => strutil::to_screaming_snake_case(value),
        "skewer" | "kebab" => strutil::to_skewer_case(value),
        "lower" => value.to_lowercase(),
        "upper" => value.to_uppercase(),
        "replace" => {
            if params.len() < 2 {
                return Err(RenderError::new(format!(
                    "'replace' processor requires 2 arguments - replace(find, replacement), but got {}: {:?}",
                    params.len(),
                    params
                )));
            }
            value.replace(params[0].as_str(), params[1].as_str())
        }
        "abbr" => abbr.apply(value),
        _ => {
            return Err(RenderError::new(format!(
                "cannot find '{proc}' in string processors, valid values are: [{}]",
                PROCESSORS.join(", ")
            )));
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(chain: &KeyChain) -> Vec<&str> {
        chain.steps.iter().map(|s| s.name.as_str()).collect()
    }

    #[test]
    fn a_chain_is_split_at_the_dots() {
        let chain = KeyChain::parse("name.suffix.camel");
        assert_eq!(names(&chain), vec!["name", "suffix", "camel"]);
        assert!(chain.steps.iter().all(|s| s.params.is_empty()));
    }

    #[test]
    fn arguments_are_collected_quoted_or_bare() {
        let quoted = KeyChain::parse("name.replace('_','-')");
        assert_eq!(
            quoted.steps[1].params,
            vec!["_".to_string(), "-".to_string()]
        );

        let bare = KeyChain::parse("name.replace(_, -)");
        assert_eq!(bare.steps[1].params, vec!["_".to_string(), "-".to_string()]);

        let mixed = KeyChain::parse("name.replace(ghi, 'xyz')");
        assert_eq!(
            mixed.steps[1].params,
            vec!["ghi".to_string(), "xyz".to_string()]
        );
    }

    #[test]
    fn a_single_key_is_a_chain_of_one_step() {
        let chain = KeyChain::parse("name");
        assert_eq!(names(&chain), vec!["name"]);
        assert_eq!(chain.key(), "name");
    }
}
