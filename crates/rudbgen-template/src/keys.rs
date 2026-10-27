//! Key chains and the string processors they are made of.
//!
//! `${name.replace('_','-').camel}` is **not** a nested path: the first step
//! names a member of the model and every following step rewrites the value the
//! previous one produced, left to right.

use crate::abbr::Abbreviations;
use crate::error::RenderError;
use crate::strutil;

/// The processor names a chain may use, in the order jdbgen lists them in its
/// error message and rudbgen's own `clamp` behind.
const PROCESSORS: [&str; 13] = [
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
    "clamp",
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

    /// Make sure every processor of the chain exists and is given what it
    /// needs, so that a misspelled `${name.pascl}` is a parse error rather than
    /// a render error on the first table that reaches it.
    ///
    /// The answer is the message [`apply`] would have failed with.
    pub(crate) fn check(&self) -> Result<(), String> {
        for step in self.steps.iter().skip(1) {
            let proc = strutil::trim(&step.name).to_lowercase();
            if !PROCESSORS.contains(&proc.as_str()) {
                return Err(unknown_processor(&proc));
            }
            if proc == "replace" && step.params.len() < 2 {
                return Err(replace_arity(&step.params));
            }
            if proc == "clamp" {
                clamp_bounds(&step.params)?;
            }
        }
        Ok(())
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
                return Err(RenderError::new(replace_arity(params)));
            }
            value.replace(params[0].as_str(), params[1].as_str())
        }
        "abbr" => abbr.apply(value),
        "clamp" => {
            let (min, max) = clamp_bounds(params).map_err(RenderError::new)?;
            // Only a value past a bound is rewritten, so that a number in
            // range keeps the very text it came with. A value that is no
            // whole number is not the processor's to judge - the `keySeq` of
            // a column outside the key renders empty - and passes unchanged.
            match strutil::parse_int(value) {
                Some(number) if number < min => min.to_string(),
                Some(number) if number > max => max.to_string(),
                _ => value.to_string(),
            }
        }
        _ => return Err(RenderError::new(unknown_processor(&proc))),
    })
}

/// What a processor nobody knows is reported as.
fn unknown_processor(proc: &str) -> String {
    format!(
        "cannot find '{proc}' in string processors, valid values are: [{}]",
        PROCESSORS.join(", ")
    )
}

/// What a `replace` with fewer than its two arguments is reported as.
fn replace_arity(params: &[String]) -> String {
    format!(
        "'replace' processor requires 2 arguments - replace(find, replacement), but got {}: {:?}",
        params.len(),
        params
    )
}

/// The two bounds of a `clamp(min, max)`, or the message a pair that is not
/// two whole numbers in order fails with.
fn clamp_bounds(params: &[String]) -> Result<(i64, i64), String> {
    let bounds = match params {
        [min, max] => strutil::parse_int(min).zip(strutil::parse_int(max)),
        _ => None,
    };
    let Some((min, max)) = bounds else {
        return Err(format!(
            "'clamp' processor requires 2 whole numbers - clamp(min, max), but got {}: {:?}",
            params.len(),
            params
        ));
    };
    if min > max {
        return Err(format!(
            "'clamp' processor requires min <= max, but got clamp({min}, {max})"
        ));
    }
    Ok((min, max))
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
    fn a_chain_is_checked_step_by_step_behind_the_member() {
        assert!(KeyChain::parse("name.suffix.Pascal").check().is_ok());
        assert!(
            KeyChain::parse("pascl").check().is_ok(),
            "the first step is a member, never a processor"
        );
        let error = KeyChain::parse("name.pascl").check().unwrap_err();
        assert!(error.contains("'pascl'"), "{error}");
        let error = KeyChain::parse("name.replace(a)").check().unwrap_err();
        assert!(error.contains("requires 2 arguments"), "{error}");
        assert!(KeyChain::parse("name.replace(a, b)").check().is_ok());
    }

    #[test]
    fn a_clamp_is_checked_for_two_whole_numbers_in_order() {
        assert!(KeyChain::parse("length.clamp(1, 4000)").check().is_ok());
        assert!(KeyChain::parse("length.clamp('-5', '5')").check().is_ok());
        assert!(KeyChain::parse("length.clamp(7, 7)").check().is_ok());
        for bad in [
            "length.clamp",
            "length.clamp(1)",
            "length.clamp(1, 2, 3)",
            "length.clamp(a, 10)",
            "length.clamp('1.5', 10)",
        ] {
            let error = KeyChain::parse(bad).check().unwrap_err();
            assert!(error.contains("requires 2 whole numbers"), "{bad}: {error}");
        }
        let error = KeyChain::parse("length.clamp(10, 1)").check().unwrap_err();
        assert!(error.contains("min <= max"), "{error}");
    }

    #[test]
    fn a_clamp_rewrites_only_a_number_past_a_bound() {
        let abbr = Abbreviations::default();
        let clamp = |value: &str| {
            apply(
                "clamp",
                value,
                &["1".to_string(), "4000".to_string()],
                &abbr,
            )
            .unwrap()
        };
        assert_eq!(clamp("0"), "1");
        assert_eq!(clamp("-20"), "1");
        assert_eq!(clamp("40"), "40");
        assert_eq!(clamp(" 40 "), " 40 ", "a number in range keeps its text");
        assert_eq!(clamp("2147483647"), "4000");
        assert_eq!(clamp(""), "");
        assert_eq!(clamp("VARCHAR"), "VARCHAR");
        assert_eq!(clamp("1.5"), "1.5", "only whole numbers are numbers here");
    }

    #[test]
    fn a_single_key_is_a_chain_of_one_step() {
        let chain = KeyChain::parse("name");
        assert_eq!(names(&chain), vec!["name"]);
        assert_eq!(chain.key(), "name");
    }
}
