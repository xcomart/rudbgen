//! The abbreviation dictionary of the `abbr` processor.
//!
//! jdbgen keeps this in a pair of static maps rebuilt from the configuration;
//! here it is a value the caller hands to the renderer, so that a preview may
//! use a draft dictionary while the generator uses the saved one.

use std::collections::HashMap;

/// One rule of the abbreviation table, as the configuration stores it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AbbrRule {
    /// Whether this rule takes part in the replacement.
    pub enabled: bool,
    /// `true` to match the whole identifier, `false` to match one word of it.
    pub whole_name: bool,
    /// The abbreviation to look for, matched ignoring case.
    pub abbr: String,
    /// What it is replaced with.
    pub replace_to: String,
}

impl AbbrRule {
    /// A rule that is turned on, matching one word of an identifier.
    pub fn word(abbr: impl Into<String>, replace_to: impl Into<String>) -> Self {
        AbbrRule {
            enabled: true,
            whole_name: false,
            abbr: abbr.into(),
            replace_to: replace_to.into(),
        }
    }

    /// A rule that is turned on, matching a whole identifier.
    pub fn whole(abbr: impl Into<String>, replace_to: impl Into<String>) -> Self {
        AbbrRule {
            enabled: true,
            whole_name: true,
            abbr: abbr.into(),
            replace_to: replace_to.into(),
        }
    }
}

/// The rules of [`AbbrRule`] in the shape the `abbr` processor looks them up
/// in: keyed by the lower cased abbreviation, whole names apart from words.
#[derive(Clone, Debug, Default)]
pub struct Abbreviations {
    whole: HashMap<String, String>,
    words: HashMap<String, String>,
}

impl Abbreviations {
    /// A dictionary without any rule, which hands every name through.
    pub fn new() -> Self {
        Abbreviations::default()
    }

    /// Build the dictionary out of the configured rules, leaving out the ones
    /// that are turned off.
    pub fn from_rules<I: IntoIterator<Item = AbbrRule>>(rules: I) -> Self {
        let mut res = Abbreviations::new();
        for rule in rules {
            if !rule.enabled {
                continue;
            }
            if rule.whole_name {
                res.add_whole(rule.abbr, rule.replace_to);
            } else {
                res.add_word(rule.abbr, rule.replace_to);
            }
        }
        res
    }

    /// Add a rule that matches a whole identifier.
    pub fn add_whole(&mut self, abbr: impl AsRef<str>, replace_to: impl Into<String>) {
        self.whole
            .insert(abbr.as_ref().to_lowercase(), replace_to.into());
    }

    /// Add a rule that matches one word of an identifier.
    pub fn add_word(&mut self, abbr: impl AsRef<str>, replace_to: impl Into<String>) {
        self.words
            .insert(abbr.as_ref().to_lowercase(), replace_to.into());
    }

    /// Whether there is no rule at all.
    pub fn is_empty(&self) -> bool {
        self.whole.is_empty() && self.words.is_empty()
    }

    /// Apply the dictionary to one identifier.
    ///
    /// A whole name rule wins and ends the replacement; otherwise the name is
    /// split at `_` and `-`, every word is looked up on its own and the
    /// separators stay where they were.
    ///
    /// Word rules match **ignoring case** - decision D10 of the architecture
    /// document, and the one deliberate behavioural break from jdbgen, whose
    /// word lookup used the raw segment against a lower cased map and
    /// therefore never fired on `TB_USR`.
    pub fn apply(&self, item: &str) -> String {
        if let Some(replaced) = self.whole.get(&item.to_lowercase()) {
            return replaced.clone();
        }
        let mut out = String::with_capacity(item.len());
        let mut word = String::new();
        for c in item.chars() {
            if c == '_' || c == '-' {
                out.push_str(self.word_of(&word));
                out.push(c);
                word.clear();
            } else {
                word.push(c);
            }
        }
        out.push_str(self.word_of(&word));
        out
    }

    /// One word, replaced when a rule knows it.
    fn word_of<'a>(&'a self, word: &'a str) -> &'a str {
        match self.words.get(&word.to_lowercase()) {
            Some(replaced) => replaced.as_str(),
            None => word,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dict() -> Abbreviations {
        Abbreviations::from_rules([
            AbbrRule::word("usr", "user"),
            AbbrRule::word("acct", "account"),
            AbbrRule {
                enabled: false,
                ..AbbrRule::word("tb", "table")
            },
            AbbrRule::whole("tb_sys", "system"),
        ])
    }

    #[test]
    fn a_whole_name_rule_wins_over_the_word_rules() {
        assert_eq!(dict().apply("TB_SYS"), "system");
    }

    #[test]
    fn words_are_replaced_and_the_separators_stay() {
        assert_eq!(dict().apply("tb-usr_acct"), "tb-user_account");
    }

    #[test]
    fn a_name_without_a_known_word_is_handed_through() {
        assert_eq!(dict().apply("other_name"), "other_name");
        assert_eq!(dict().apply(""), "");
    }

    #[test]
    fn word_rules_match_ignoring_case() {
        // D10: jdbgen only ever matched lower case segments
        assert_eq!(dict().apply("TB_USR"), "TB_user");
        assert_eq!(dict().apply("Usr"), "user");
    }
}
