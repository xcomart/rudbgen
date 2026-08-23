//! What the wizard shows before anything is written.
//!
//! The import dialog of §4.6 "shows what it found (connections, drivers, sets,
//! rules) with checkboxes, then writes the stores and the keychain". This is
//! the *what it found*: one flat row per thing, already carrying the answers
//! the user needs to tick a box — which built-in a stock driver landed on, how
//! many templates a set holds — and never a secret.
//!
//! It is derived from a [`Mapped`], so the preview cannot drift from what the
//! import will actually do: the same function produced both.

use crate::map::{Decrypted, MapOptions, Mapped, map};
use crate::notes::Note;

/// The checklist the import wizard draws.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Preview {
    /// One row per saved connection.
    pub connections: Vec<ConnectionPreview>,
    /// One row per driver definition.
    pub drivers: Vec<DriverPreview>,
    /// One row per template set.
    pub sets: Vec<SetPreview>,
    /// One row per abbreviation rule.
    pub rules: Vec<RulePreview>,
    /// Everything the import wants said out loud.
    pub notes: Vec<Note>,
}

/// A connection as the checklist shows it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConnectionPreview {
    /// Name jdbgen gave it.
    pub name: String,
    /// Name of the driver it will use, or the unresolved `driverType`.
    pub driver: String,
    /// The URL with any inline credentials taken out of it.
    pub url: String,
}

/// A driver as the checklist shows it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DriverPreview {
    /// Name jdbgen gave it.
    pub name: String,
    /// Whether jdbgen shipped it rather than the user writing it.
    pub stock: bool,
    /// Id of the built-in definition it maps onto, when it maps onto one.
    ///
    /// `None` means the import adds a definition of its own, which is the row
    /// a user is most likely to want to look at.
    pub matched_builtin: Option<String>,
}

/// A template set as the checklist shows it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SetPreview {
    /// Name of the set.
    pub name: String,
    /// How many templates it holds.
    pub templates: usize,
}

/// An abbreviation rule as the checklist shows it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RulePreview {
    /// Whether the rule is on.
    pub enabled: bool,
    /// Whether it matches a whole identifier rather than a word inside one.
    pub whole_name: bool,
    /// What it looks for.
    pub abbreviation: String,
    /// What it puts in its place.
    pub replacement: String,
}

/// Everything an import would do, as a checklist.
///
/// Runs the mapping and reads the answer back, so the preview is the import.
pub fn preview(dec: &Decrypted, opts: &MapOptions) -> Preview {
    let mapped = map(dec, opts);
    from_mapped(dec, &mapped)
}

/// The checklist for a mapping that has already been computed.
///
/// Separate from [`preview`] so the wizard can hold one [`Mapped`] from the
/// moment the password is accepted to the moment the boxes are ticked, rather
/// than mapping twice and showing the user ids that the second run invented
/// afresh.
pub fn from_mapped(dec: &Decrypted, mapped: &Mapped) -> Preview {
    Preview {
        connections: dec
            .config
            .connections
            .iter()
            .map(|conn| ConnectionPreview {
                name: conn.name.clone(),
                driver: conn.driver_type.clone(),
                url: mask_url(&conn.connection_url),
            })
            .collect(),
        drivers: dec
            .config
            .drivers
            .iter()
            .map(|driver| DriverPreview {
                name: driver.name.clone(),
                stock: driver.stock_item,
                matched_builtin: crate::map::matched_builtin_id(driver),
            })
            .collect(),
        sets: mapped
            .sets
            .iter()
            .map(|set| SetPreview {
                name: set.name.clone(),
                templates: set.templates.len(),
            })
            .collect(),
        rules: mapped
            .rules
            .iter()
            .map(|rule| RulePreview {
                enabled: rule.enabled,
                whole_name: rule.whole_name,
                abbreviation: rule.abbreviation.clone(),
                replacement: rule.replacement.clone(),
            })
            .collect(),
        notes: mapped.notes.clone(),
    }
}

/// The URL with anything that could be a credential replaced.
///
/// The same rule `rudbgen-core`'s `MaskedUrl` applies to a log line, applied
/// here to a screen: the user info before an `@` goes, and so does every
/// parameter value after the first `?` or `;`. What is left is enough to
/// recognise which database a row is about, which is the whole job of the
/// column. jdbgen encrypted the URL, so a wizard that printed it in full would
/// be showing more than the file ever did.
fn mask_url(url: &str) -> String {
    const REDACTED: &str = "<redacted>";
    let (base, params) = match url.find(['?', ';']) {
        Some(index) => (&url[..index], Some(&url[index..])),
        None => (url, None),
    };

    let mut out = String::with_capacity(url.len());
    match base.find('@') {
        Some(at) => {
            let start = base[..at].rfind(':').map_or(0, |colon| colon + 1);
            if start == at {
                // Oracle's `thin:@//host:1521/service` has an `@` and no
                // credentials in front of it.
                out.push_str(base);
            } else {
                out.push_str(&base[..start]);
                out.push_str(REDACTED);
                out.push_str(&base[at..]);
            }
        }
        None => out.push_str(base),
    }

    let Some(params) = params else {
        return out;
    };
    out.push_str(&params[..1]);
    for (index, token) in params[1..].split(['?', ';', '&']).enumerate() {
        if index > 0 {
            out.push(';');
        }
        match token.find('=') {
            Some(equals) => {
                out.push_str(&token[..=equals]);
                out.push_str(REDACTED);
            }
            None => out.push_str(token),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_url_with_nothing_to_hide_is_shown_as_it_is() {
        assert_eq!(
            mask_url("jdbc:postgresql://db.example.com:5432/app"),
            "jdbc:postgresql://db.example.com:5432/app"
        );
        assert_eq!(
            mask_url("jdbc:oracle:thin:@//host:1521/service"),
            "jdbc:oracle:thin:@//host:1521/service"
        );
    }

    #[test]
    fn inline_credentials_are_taken_out() {
        assert_eq!(
            mask_url("jdbc:mysql://alice:hunter2@db:3306/app"),
            "jdbc:mysql://alice:<redacted>@db:3306/app"
        );
    }

    #[test]
    fn every_parameter_value_goes_whatever_it_is_called() {
        assert_eq!(
            mask_url("jdbc:sqlserver://db:1433;databaseName=app;password=hunter2"),
            "jdbc:sqlserver://db:1433;databaseName=<redacted>;password=<redacted>"
        );
    }

    #[test]
    fn a_parameter_with_no_value_is_left_alone() {
        assert_eq!(
            mask_url("jdbc:h2:mem:test;DB_CLOSE_DELAY"),
            "jdbc:h2:mem:test;DB_CLOSE_DELAY"
        );
    }
}
