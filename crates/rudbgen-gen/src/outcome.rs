//! What a run produced: the summary D11 asks for.

use std::path::PathBuf;

use rudbgen_template::Warning;

/// Which of a template's two halves something happened in.
///
/// A template is two templates: the body in the file, and the output name in
/// the profile. A parse error or a warning is useless without saying which of
/// the two it came from — they are edited in different places.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TemplatePart {
    /// The template file itself.
    Body,
    /// The output file name template of the profile.
    OutputName,
}

impl TemplatePart {
    /// What to call this part in a message.
    pub fn label(self) -> &'static str {
        match self {
            TemplatePart::Body => "template body",
            TemplatePart::OutputName => "output name",
        }
    }
}

/// Why a pair produced no file.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SkipReason {
    /// The destination was already there and the policy is
    /// [`Overwrite::Skip`](crate::Overwrite::Skip).
    ExistingFile,
    /// The destination was already there and the user chose to keep it.
    UserSkipped,
}

/// One file a run did not write because it was already there.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Skipped {
    /// The file that was left alone.
    pub path: PathBuf,
    /// The table the pair was for.
    pub table: String,
    /// The template the pair was for.
    pub template: String,
    /// Why it was left alone.
    pub reason: SkipReason,
}

/// One thing that went wrong.
///
/// `table` is `None` for a failure that belongs to a template rather than to a
/// pair — a template file that cannot be read or parsed — and `path` is `None`
/// when the run never got as far as having one, which is every failure of the
/// output name itself.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Failure {
    /// The table the pair was for, when the failure belongs to a pair.
    pub table: Option<String>,
    /// The template the failure belongs to.
    pub template: String,
    /// The destination, when the run got as far as resolving one.
    pub path: Option<PathBuf>,
    /// What went wrong, worded for the result summary.
    pub message: String,
}

/// One unknown field, and where it was found.
///
/// The engine renders an empty string for these and carries on (jdbgen does
/// too); collecting them is what turns a silently empty `${nmae}` into a line
/// of the result summary.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Diagnostic {
    /// The table being rendered.
    pub table: String,
    /// The template being rendered.
    pub template: String,
    /// Which half of the template the warning is in.
    pub part: TemplatePart,
    /// The warning, with the span the editor marks.
    pub warning: Warning,
}

/// Everything one [`generate`](crate::generate) run did.
///
/// Nothing here is a failure of the *call*: a run that wrote nothing at all
/// still answers with an `Outcome`, because the summary of what went wrong is
/// the point (D11). [`Outcome::is_ok`] is the one-line verdict.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Outcome {
    /// Every file written, in the order they were written.
    pub written: Vec<PathBuf>,
    /// Every file left alone because it was already there.
    pub skipped: Vec<Skipped>,
    /// Everything that went wrong, in the order it was found.
    pub failed: Vec<Failure>,
    /// Whether the run stopped before it ran out of work.
    ///
    /// Set by the cancel token and by [`Decision::Cancel`](crate::Decision).
    /// The files already written stay written — this crate does not roll back.
    pub cancelled: bool,
    /// Every unknown field the render passes noticed.
    pub diagnostics: Vec<Diagnostic>,
}

impl Outcome {
    /// Whether the run finished with nothing to report but its files.
    pub fn is_ok(&self) -> bool {
        self.failed.is_empty() && !self.cancelled
    }

    /// How many pairs the run accounted for, written, skipped or failed.
    pub fn handled(&self) -> usize {
        self.written.len() + self.skipped.len() + self.failed.len()
    }
}

/// One file a [`dry_run`](crate::dry_run) would have written.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RenderedFile {
    /// The table it was rendered from.
    pub table: String,
    /// The template it was rendered from.
    pub template: String,
    /// Where it would have gone.
    pub path: PathBuf,
    /// What would have been written, as UTF-8 text.
    pub content: String,
    /// Whether something is already at [`RenderedFile::path`].
    ///
    /// The one thing a dry run reads off the disk, and the one thing the user
    /// asked a dry run for: which of these would replace a file.
    pub exists: bool,
}

/// What a [`dry_run`](crate::dry_run) produced. Nothing was written.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DryRun {
    /// The files, in the order the run would have written them.
    pub files: Vec<RenderedFile>,
    /// Everything that would have gone wrong before the write.
    pub failed: Vec<Failure>,
    /// Whether the run stopped early.
    pub cancelled: bool,
    /// Every unknown field the render passes noticed.
    pub diagnostics: Vec<Diagnostic>,
}

/// One table × template pair, rendered for the preview pane.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Preview {
    /// Where a real run would put it.
    pub path: PathBuf,
    /// The rendered text.
    pub content: String,
    /// The unknown fields of both render passes: the output name's first,
    /// then the body's, which is the order they are rendered in.
    pub diagnostics: Vec<Warning>,
}
