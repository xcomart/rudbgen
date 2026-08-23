//! What can go wrong before a run is even a run.
//!
//! [`generate`](crate::generate) and [`dry_run`](crate::dry_run) never answer
//! with one of these — everything that goes wrong there is a
//! [`Failure`](crate::Failure) in the summary, because a run that failed on
//! one of two hundred pairs is not a failed call. [`preview`](crate::preview)
//! is the other shape: one pair, one answer.

use std::path::PathBuf;

use rudbgen_template::{ParseError, RenderError};
use thiserror::Error;

use crate::outcome::TemplatePart;

/// A single pair that could not be rendered.
#[derive(Clone, Debug, PartialEq, Eq, Error)]
pub enum Error {
    /// The plan has no table at that index.
    #[error("no table at index {0}")]
    NoSuchTable(usize),
    /// The plan has no template at that index.
    #[error("no template at index {0}")]
    NoSuchTemplate(usize),
    /// The plan names no output directory.
    #[error("no output directory is set")]
    NoOutputDir,
    /// The template file could not be read, or is not UTF-8.
    #[error("cannot read template '{template}' from {}: {message}", path.display())]
    Read {
        /// The template's name in the profile.
        template: String,
        /// The file it should have come from.
        path: PathBuf,
        /// What the filesystem, or the UTF-8 decoder, said.
        message: String,
    },
    /// The template could not be parsed.
    #[error("{template} ({}): {error}", part.label())]
    Parse {
        /// The template's name in the profile.
        template: String,
        /// Which half of it failed.
        part: TemplatePart,
        /// The engine's error, with the line.
        #[source]
        error: ParseError,
    },
    /// The template parsed but could not be rendered against this table.
    #[error("{template} ({}) on {table}: {error}", part.label())]
    Render {
        /// The template's name in the profile.
        template: String,
        /// Which half of it failed.
        part: TemplatePart,
        /// The table it was rendered against.
        table: String,
        /// The engine's error, with the line when it has one.
        #[source]
        error: RenderError,
    },
    /// The rendered output name cannot be used.
    #[error("{template} on {table}: {message}")]
    OutputPath {
        /// The template's name in the profile.
        template: String,
        /// The table whose name rendered it.
        table: String,
        /// Why the name was refused.
        message: String,
    },
}
