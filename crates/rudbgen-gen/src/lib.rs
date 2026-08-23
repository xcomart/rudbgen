//! The generation job: a [`Plan`] of tables and templates, rendered and
//! written to disk (architecture document, §9 and D11).
//!
//! ```no_run
//! use rudbgen_gen::{CancelToken, Overwrite, Plan, TemplateSpec, generate};
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! # let tables: Vec<rudbgen_meta::Table> = Vec::new();
//! let plan = Plan::new(
//!     tables,
//!     vec![TemplateSpec::new(
//!         "Java Model",
//!         "/home/me/.config/rudbgen/templates/java_model.java",
//!         "${name.suffix.pascal}Model.java",
//!     )],
//!     "/home/me/out/src",
//! );
//!
//! let cancel = CancelToken::new();
//! let outcome = generate(&plan, Overwrite::Skip, &cancel, &|progress| {
//!     println!("{progress:?}");
//! });
//! println!("{} files written", outcome.written.len());
//! # Ok(()) }
//! ```
//!
//! # Where this sits
//!
//! It knows `rudbgen-meta` (the model a template renders against),
//! `rudbgen-template` (the engine) and `rudbgen-core` (the saved profile, the
//! overwrite policy and the atomic write). It knows neither gpui nor JNI, and
//! must not learn either: the tables arrive **already loaded**, because
//! reading them needs a JDBC session and a session needs a JVM, and the whole
//! point of this crate is that every rule below is testable in a temporary
//! directory.
//!
//! The three entry points render the same way and differ only in where the
//! text goes: [`generate`] writes it, [`dry_run`] keeps it in memory, and
//! [`preview`] renders exactly one table × template pair.
//!
//! # The rules of §9
//!
//! 1. **Every** template — bodies and output names alike — is parsed before
//!    the first file is written. One parse error fails the run with **no file
//!    written at all**, naming the template, the part and the line.
//! 2. The output name is a template too, rendered per table, and is resolved
//!    against [`Plan::output_dir`]. A name that reaches outside of it — an
//!    absolute path, a drive letter, any `..` component — is refused and
//!    recorded in [`Outcome::failed`]. A name with `/` in it is a
//!    subdirectory, which is created.
//! 3. Files are written atomically, through
//!    [`rudbgen_core::paths::write_atomic`]: the bytes land in a temporary
//!    sibling that is renamed over the destination, so a crash cannot leave a
//!    half-written source file behind.
//! 4. Line endings are whatever the engine produced. It already reuses the
//!    separator the template file is written with, so nothing here rewrites
//!    them.
//! 5. Under [`Overwrite::Ask`] the callback runs on **this** thread, which is
//!    the background thread the job is driven from: the application is
//!    expected to hand the question to the UI over a channel and block for the
//!    answer. [`Decision::OverwriteAll`] and [`Decision::SkipAll`] stop the
//!    asking, [`Decision::Cancel`] stops the run.
//! 6. Cancellation is checked at file boundaries. Files already written stay
//!    on disk and stay listed in [`Outcome::written`]; see below.
//! 7. Unknown-field warnings from the engine are collected per table ×
//!    template into [`Outcome::diagnostics`] rather than failing the pair —
//!    jdbgen renders an empty string for them and so does this.
//! 8. `author` is injected as the custom variable of that name, so that
//!    `${author}` and `${item:key=author}` are the same value. A non-empty
//!    [`Plan::author`] wins over a custom variable spelled the same; an empty
//!    one leaves the variable alone, because a form field nobody filled in
//!    must not erase a variable somebody typed.
//!
//! # What this deliberately does not do
//!
//! * **It does not choose an encoding.** Output is UTF-8, always, as jdbgen's
//!   is. A template file that is not UTF-8 fails to load rather than being
//!   transliterated. (The one place a byte encoding is honoured is padding
//!   width, and that lives in the engine — see `rudbgen_template`'s EUC-KR
//!   rule.)
//! * **It does not roll back.** A run that fails on the tenth file leaves the
//!   nine before it on disk. Deleting them would be worse: the files are the
//!   user's source tree, some of them are what the run was for, and nothing
//!   here knows which of them existed before. Instead every file the run
//!   touched is listed in the [`Outcome`], which is the summary D11 asks for.
//! * **It does not follow symlinks.** The escape check is on the rendered
//!   name, not on the path it resolves to, so a symlink already inside the
//!   output directory can still lead out of it. Guarding that would mean
//!   canonicalising a path that does not exist yet.
//! * **It does not cache, load or refresh metadata.** [`Plan::tables`] is
//!   whatever the caller handed over.

#![warn(missing_docs)]

mod cancel;
mod error;
mod outcome;
mod plan;
mod policy;
mod progress;
mod run;

pub use cancel::{CancelHandle, CancelToken};
pub use error::Error;
pub use outcome::{
    Diagnostic, DryRun, Failure, Outcome, Preview, RenderedFile, SkipReason, Skipped, TemplatePart,
};
pub use plan::{Plan, TemplateSpec, abbreviations_of};
pub use policy::{Decision, Overwrite};
pub use progress::{FileStatus, Progress};
pub use run::{dry_run, generate, preview};
