//! What the import has to say for itself.
//!
//! A [`Note`] is *data*, never a sentence: this crate carries no user-facing
//! strings, because the layer that has the eight translations is the app
//! (`docs/status.md`, "How work is done here"). The [`Display`] impls below
//! exist so a note can be logged, and are English on purpose — a log is read by
//! whoever is debugging, not by the user.
//!
//! Notes are not errors. Every one of them describes something the import did
//! anyway, and the wizard's job is to show them beside the checklist so that
//! nothing changes silently.
//!
//! [`Display`]: std::fmt::Display

use std::fmt;

/// One thing the import wants the wizard to say out loud.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Note {
    /// Word-level abbreviation rules now match case-insensitively (D10).
    ///
    /// The one deliberate behavioural break from jdbgen, and the reason it is
    /// emitted **always**, whether or not the configuration holds a single
    /// rule: a user who adds one later still needs to have been told, and the
    /// architecture document names the import wizard as one of the two places
    /// it is called out.
    AbbreviationCaseRule,

    /// At least one value was written in the superseded encryption format.
    ///
    /// Informational: it was read all the same. rudbgen does not rewrite the
    /// jdbgen configuration — the file is left exactly as it was found — so
    /// there is nothing for the user to do beyond knowing that the file is old.
    LegacyEncryption,

    /// A stock driver was recognised and rudbgen's own definition is used.
    ///
    /// The JAR paths, the four SQL overrides and the properties come across
    /// from jdbgen; the id, the URL template and the dialect are rudbgen's, so
    /// that a connection created afterwards gets the placeholders the
    /// connection dialog knows how to fill.
    StockDriverMatched {
        /// jdbgen's name for the driver.
        driver: String,
        /// Id of the built-in definition it was matched to.
        builtin: String,
    },

    /// A driver marked stock by jdbgen matches no built-in definition.
    ///
    /// It is imported as a driver of its own, which is the right answer for a
    /// stock entry whose class the user changed and for a product rudbgen does
    /// not ship a definition for.
    StockDriverUnknown {
        /// jdbgen's name for the driver.
        driver: String,
        /// The driver class that matched nothing.
        class: String,
    },

    /// A connection names a driver the configuration does not define.
    ///
    /// The connection is imported with no driver, which is a state the
    /// connection dialog already has to render: the user picks one before
    /// connecting.
    UnknownDriver {
        /// Name of the connection.
        connection: String,
        /// The `driverType` that matched no driver.
        driver_type: String,
    },

    /// A keep-alive interval could not be read as a number of seconds.
    ///
    /// jdbgen stores it as a string, so `""`, `"30 sec"` and `"-1"` all occur.
    /// The probe is imported switched off rather than at a guessed interval.
    KeepAliveNotANumber {
        /// Name of the connection.
        connection: String,
        /// The value that is not a number.
        value: String,
    },

    /// A relative path was found in neither of jdbgen's two directories.
    ///
    /// It is carried across unchanged — an absolute path could only be a guess,
    /// and the file may simply not have been copied to this machine yet.
    UnresolvedPath {
        /// What the path was for.
        kind: PathKind,
        /// Which entry carried it.
        owner: String,
        /// The path, as the file spells it.
        path: String,
    },

    /// An icon locator has no rudbgen equivalent.
    ///
    /// rudbgen names an icon by asset stem; jdbgen also allows a file path, a
    /// Font Awesome glyph, a colour swatch and a URL. Those come across as *no
    /// icon*, which is what the driver picker draws for a driver the user
    /// added.
    IconDropped {
        /// Which entry carried it.
        owner: String,
        /// The locator that was dropped.
        icon: String,
    },
}

/// What a path in a jdbgen configuration points at.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PathKind {
    /// A JDBC driver JAR (`jdbcJar`).
    DriverJar,
    /// A template body (`templateFile`).
    TemplateFile,
    /// A generation output directory (`outputDir`).
    OutputDir,
}

impl fmt::Display for PathKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let text = match self {
            Self::DriverJar => "driver JAR",
            Self::TemplateFile => "template file",
            Self::OutputDir => "output directory",
        };
        f.write_str(text)
    }
}

impl fmt::Display for Note {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::AbbreviationCaseRule => f.write_str(
                "word abbreviation rules now match case-insensitively, which jdbgen did not do",
            ),
            Self::LegacyEncryption => {
                f.write_str("some values were written in jdbgen's superseded encryption format")
            }
            Self::StockDriverMatched { driver, builtin } => {
                write!(
                    f,
                    "stock driver '{driver}' maps onto the built-in '{builtin}'"
                )
            }
            Self::StockDriverUnknown { driver, class } => write!(
                f,
                "stock driver '{driver}' ({class}) matches no built-in definition and is imported as its own"
            ),
            Self::UnknownDriver {
                connection,
                driver_type,
            } => write!(
                f,
                "connection '{connection}' names the driver '{driver_type}', which this configuration does not define"
            ),
            Self::KeepAliveNotANumber { connection, value } => write!(
                f,
                "connection '{connection}' has a keep-alive interval of '{value}', which is not a number of seconds"
            ),
            Self::UnresolvedPath { kind, owner, path } => write!(
                f,
                "the {kind} '{path}' of '{owner}' was not found in either of jdbgen's directories"
            ),
            Self::IconDropped { owner, icon } => {
                write!(f, "the icon '{icon}' of '{owner}' has no equivalent here")
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_note_renders_something_a_log_can_carry() {
        let notes = [
            Note::AbbreviationCaseRule,
            Note::LegacyEncryption,
            Note::StockDriverMatched {
                driver: "H2 Embedded".into(),
                builtin: "h2-embedded".into(),
            },
            Note::StockDriverUnknown {
                driver: "Whatever".into(),
                class: "com.example.Driver".into(),
            },
            Note::UnknownDriver {
                connection: "staging".into(),
                driver_type: "Gone".into(),
            },
            Note::KeepAliveNotANumber {
                connection: "staging".into(),
                value: "30 sec".into(),
            },
            Note::UnresolvedPath {
                kind: PathKind::DriverJar,
                owner: "Whatever".into(),
                path: "drivers/x.jar".into(),
            },
            Note::IconDropped {
                owner: "Whatever".into(),
                icon: "fa:database".into(),
            },
        ];
        for note in notes {
            assert!(!note.to_string().is_empty(), "{note:?}");
        }
    }
}
