//! What a run reports while it is running.

use std::path::PathBuf;

use crate::outcome::{Outcome, SkipReason};

/// What became of one table × template pair.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FileStatus {
    /// The file was written.
    Written,
    /// The file was left alone.
    Skipped(SkipReason),
    /// Nothing was written; the message is the one in the
    /// [`Failure`](crate::Failure).
    Failed(String),
}

/// One step of a run.
///
/// Every field is owned rather than borrowed so that a callback can put the
/// event straight onto a channel — which is what the application does, since
/// the job runs on a background thread and the progress dialog does not.
///
/// The sequence is always [`Progress::Started`], then either
/// [`Progress::Parsed`] followed by one [`Progress::File`] per pair the run
/// reached, or nothing at all when parsing failed, and finally exactly one
/// [`Progress::Finished`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Progress {
    /// The run began. `total` is tables × templates, the denominator of the
    /// progress bar.
    Started {
        /// How many files the run would write if nothing were in its way.
        total: usize,
    },
    /// Every template parsed, so no parse error can stop the run any more.
    Parsed {
        /// How many templates were parsed, bodies and output names counting as
        /// one.
        templates: usize,
    },
    /// One pair was handled.
    File {
        /// One-based position of this pair among `total`.
        index: usize,
        /// The same `total` as [`Progress::Started`].
        total: usize,
        /// The table.
        table: String,
        /// The template.
        template: String,
        /// The destination, or `None` when the output name itself failed.
        path: Option<PathBuf>,
        /// What became of it.
        status: FileStatus,
    },
    /// The run ended, for any reason. Carries the same value the call answers
    /// with, so a listener that only sees the channel still gets the summary.
    Finished(Outcome),
}
