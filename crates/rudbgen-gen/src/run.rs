//! The run itself: parse everything, then render and write pair by pair.

use std::fs;
use std::path::{Component, Path, PathBuf};

use rudbgen_core::paths::{strip_bom, write_atomic};
use rudbgen_meta::Table;
use rudbgen_template::{Diagnostics, RenderContext, Template};

use crate::cancel::CancelToken;
use crate::error::Error;
use crate::outcome::{
    Diagnostic, DryRun, Failure, Outcome, Preview, RenderedFile, SkipReason, Skipped, TemplatePart,
};
use crate::plan::{Plan, TemplateSpec};
use crate::policy::{Decision, Overwrite};
use crate::progress::{FileStatus, Progress};

/// One template of the plan, parsed.
struct Prepared {
    name: String,
    body: Template,
    out_name: Template,
}

/// One rendered pair, before anything is decided about the disk.
struct Rendered {
    path: PathBuf,
    content: String,
}

/// Render `plan` and write it (architecture document, §9).
///
/// Answers with the summary rather than with a `Result`: a run that failed on
/// one pair of two hundred is not a failed call, and the list of what was
/// written, skipped and failed is the point (D11). See the crate
/// documentation for the eight rules this follows.
///
/// `progress` is called on this thread, in the order documented on
/// [`Progress`], and so is the [`Overwrite::Ask`] callback.
pub fn generate(
    plan: &Plan,
    policy: Overwrite,
    cancel: &CancelToken,
    progress: &dyn Fn(Progress),
) -> Outcome {
    let total = plan.total();
    progress(Progress::Started { total });

    let mut outcome = Outcome::default();
    let prepared = match prepare(plan) {
        Ok(prepared) => prepared,
        Err(failures) => {
            // Rule 1: one broken template and the run writes nothing at all.
            outcome.failed = failures;
            progress(Progress::Finished(outcome.clone()));
            return outcome;
        }
    };
    progress(Progress::Parsed {
        templates: prepared.len(),
    });

    let ctx = plan.context();
    // The two `*All` answers turn the policy into a state machine; `settled`
    // is what they settle it to.
    let mut settled: Option<bool> = match policy {
        Overwrite::Overwrite => Some(true),
        Overwrite::Skip => Some(false),
        Overwrite::Ask(_) => None,
    };
    let mut index = 0;

    'run: for table in &plan.tables {
        for prep in &prepared {
            // Rule 6: cancellation is checked at file boundaries, so a file is
            // never half written and never written twice.
            if cancel.is_cancelled() {
                outcome.cancelled = true;
                break 'run;
            }
            index += 1;

            let rendered = match render_pair(plan, table, prep, &ctx, &mut outcome.diagnostics) {
                Ok(rendered) => rendered,
                Err(error) => {
                    report_failure(
                        &mut outcome.failed,
                        progress,
                        index,
                        total,
                        table,
                        prep,
                        error,
                    );
                    continue;
                }
            };

            // Rule 3 of §9's step list: the policy is applied to the resolved
            // path *before* the body is written, so a skipped file costs
            // nothing but the name.
            let write = if rendered.path.exists() {
                match settled {
                    Some(overwrite) => {
                        if !overwrite {
                            skipped(
                                &mut outcome.skipped,
                                progress,
                                index,
                                total,
                                table,
                                prep,
                                &rendered.path,
                                SkipReason::ExistingFile,
                            );
                            continue;
                        }
                        true
                    }
                    None => {
                        let Overwrite::Ask(ask) = &policy else {
                            unreachable!("only the ask policy leaves the decision open")
                        };
                        match ask(&rendered.path) {
                            Decision::Overwrite => true,
                            Decision::OverwriteAll => {
                                settled = Some(true);
                                true
                            }
                            Decision::Skip => false,
                            Decision::SkipAll => {
                                settled = Some(false);
                                false
                            }
                            Decision::Cancel => {
                                outcome.cancelled = true;
                                break 'run;
                            }
                        }
                    }
                }
            } else {
                true
            };

            if !write {
                skipped(
                    &mut outcome.skipped,
                    progress,
                    index,
                    total,
                    table,
                    prep,
                    &rendered.path,
                    SkipReason::UserSkipped,
                );
                continue;
            }

            // Rule 4: UTF-8, always. Rule 3: through the same atomic write the
            // configuration files use.
            match write_atomic(&rendered.path, rendered.content.as_bytes()) {
                Ok(()) => {
                    outcome.written.push(rendered.path.clone());
                    progress(Progress::File {
                        index,
                        total,
                        table: table.name.clone(),
                        template: prep.name.clone(),
                        path: Some(rendered.path),
                        status: FileStatus::Written,
                    });
                }
                Err(err) => {
                    let message = format!("{err:#}");
                    outcome.failed.push(Failure {
                        table: Some(table.name.clone()),
                        template: prep.name.clone(),
                        path: Some(rendered.path.clone()),
                        message: message.clone(),
                    });
                    progress(Progress::File {
                        index,
                        total,
                        table: table.name.clone(),
                        template: prep.name.clone(),
                        path: Some(rendered.path),
                        status: FileStatus::Failed(message),
                    });
                }
            }
        }
    }

    progress(Progress::Finished(outcome.clone()));
    outcome
}

/// Render `plan` without touching the disk.
///
/// The same passes [`generate`] makes, stopping short of the write: the
/// content of every file is kept in memory, and the only thing read from the
/// disk is whether a destination is already there
/// ([`RenderedFile::exists`]). There is no policy, because nothing is
/// overwritten.
///
/// The events are the same ones [`generate`] reports, so one progress dialog
/// serves both, which is why a rendered pair is reported as
/// [`FileStatus::Written`] here as well: it is what *would* be written.
pub fn dry_run(plan: &Plan, cancel: &CancelToken, progress: &dyn Fn(Progress)) -> DryRun {
    let total = plan.total();
    progress(Progress::Started { total });

    let mut result = DryRun::default();
    let prepared = match prepare(plan) {
        Ok(prepared) => prepared,
        Err(failures) => {
            result.failed = failures;
            progress(Progress::Finished(as_outcome(&result)));
            return result;
        }
    };
    progress(Progress::Parsed {
        templates: prepared.len(),
    });

    let ctx = plan.context();
    let mut index = 0;

    'run: for table in &plan.tables {
        for prep in &prepared {
            if cancel.is_cancelled() {
                result.cancelled = true;
                break 'run;
            }
            index += 1;

            match render_pair(plan, table, prep, &ctx, &mut result.diagnostics) {
                Ok(rendered) => {
                    progress(Progress::File {
                        index,
                        total,
                        table: table.name.clone(),
                        template: prep.name.clone(),
                        path: Some(rendered.path.clone()),
                        status: FileStatus::Written,
                    });
                    result.files.push(RenderedFile {
                        table: table.name.clone(),
                        template: prep.name.clone(),
                        exists: rendered.path.exists(),
                        path: rendered.path,
                        content: rendered.content,
                    });
                }
                Err(error) => report_failure(
                    &mut result.failed,
                    progress,
                    index,
                    total,
                    table,
                    prep,
                    error,
                ),
            }
        }
    }

    progress(Progress::Finished(as_outcome(&result)));
    result
}

/// Render one table × template pair, for the preview pane.
///
/// The indices are into [`Plan::tables`] and [`Plan::templates`]. Unlike the
/// two whole-run entry points this answers with a `Result`: one pair has one
/// answer, and a preview that cannot be rendered has nothing to summarise.
///
/// # Errors
///
/// [`Error::NoSuchTable`] or [`Error::NoSuchTemplate`] for an index the plan
/// does not have, [`Error::Read`] and [`Error::Parse`] for a template that
/// cannot be loaded, [`Error::Render`] for one that cannot be rendered against
/// this table, and [`Error::OutputPath`] for a name that leaves the output
/// directory.
pub fn preview(plan: &Plan, table_index: usize, template_index: usize) -> Result<Preview, Error> {
    let table = plan
        .tables
        .get(table_index)
        .ok_or(Error::NoSuchTable(table_index))?;
    let spec = plan
        .templates
        .get(template_index)
        .ok_or(Error::NoSuchTemplate(template_index))?;

    let prepared = prepare_one(spec)?;
    let mut diagnostics = Vec::new();
    let rendered = render_pair(plan, table, &prepared, &plan.context(), &mut diagnostics)?;

    Ok(Preview {
        path: rendered.path,
        content: rendered.content,
        diagnostics: diagnostics.into_iter().map(|d| d.warning).collect(),
    })
}

/// Rule 1: parse every body and every output name before anything is written.
///
/// All of them, not only up to the first error: a run stopped by a typo should
/// name every typo, so the user fixes them in one pass rather than in as many
/// runs as there are mistakes.
fn prepare(plan: &Plan) -> Result<Vec<Prepared>, Vec<Failure>> {
    let mut prepared = Vec::with_capacity(plan.templates.len());
    let mut failures = Vec::new();

    for spec in &plan.templates {
        match prepare_one(spec) {
            Ok(one) => prepared.push(one),
            Err(error) => failures.push(failure_of(&error, None)),
        }
    }

    if failures.is_empty() {
        Ok(prepared)
    } else {
        Err(failures)
    }
}

/// Load and parse one template, body and output name.
fn prepare_one(spec: &TemplateSpec) -> Result<Prepared, Error> {
    let source = match &spec.source {
        Some(source) => source.clone(),
        None => read_template(spec)?,
    };

    let body = Template::parse(&source).map_err(|error| Error::Parse {
        template: spec.name.clone(),
        part: TemplatePart::Body,
        error,
    })?;
    let out_name = Template::parse(&spec.out_template).map_err(|error| Error::Parse {
        template: spec.name.clone(),
        part: TemplatePart::OutputName,
        error,
    })?;

    Ok(Prepared {
        name: spec.name.clone(),
        body,
        out_name,
    })
}

/// Read a template file as UTF-8.
///
/// Rule 4 cuts both ways: what is written is UTF-8, and so is what is read. A
/// byte order mark is tolerated because these files are edited by hand and
/// several Windows editors add one — the same forgiveness `rudbgen-core`
/// applies to its configuration files, through the same helper — but anything
/// else that is not UTF-8 is an error rather than a transliteration, because a
/// silently mangled template renders silently mangled source.
fn read_template(spec: &TemplateSpec) -> Result<String, Error> {
    let bytes = fs::read(&spec.file).map_err(|err| Error::Read {
        template: spec.name.clone(),
        path: spec.file.clone(),
        message: err.to_string(),
    })?;
    String::from_utf8(strip_bom(&bytes).to_vec()).map_err(|_| Error::Read {
        template: spec.name.clone(),
        path: spec.file.clone(),
        message: "the file is not valid UTF-8".to_string(),
    })
}

/// Render the output name, resolve it, then render the body.
///
/// That order is §9's, and it is what makes a skipped file cheap: the body of
/// a template is never rendered for a file the policy is going to leave alone.
fn render_pair(
    plan: &Plan,
    table: &Table,
    prep: &Prepared,
    ctx: &RenderContext,
    diagnostics: &mut Vec<Diagnostic>,
) -> Result<Rendered, Error> {
    let mut diags = Diagnostics::new();

    let name = prep
        .out_name
        .render_diagnosed(table, ctx, &mut diags)
        .map_err(|error| Error::Render {
            template: prep.name.clone(),
            part: TemplatePart::OutputName,
            table: table.name.clone(),
            error,
        })?;
    collect(
        diagnostics,
        &mut diags,
        table,
        prep,
        TemplatePart::OutputName,
    );

    let path = resolve_output(plan.output_dir(), &name).map_err(|message| Error::OutputPath {
        template: prep.name.clone(),
        table: table.name.clone(),
        message,
    })?;

    let content = prep
        .body
        .render_diagnosed(table, ctx, &mut diags)
        .map_err(|error| Error::Render {
            template: prep.name.clone(),
            part: TemplatePart::Body,
            table: table.name.clone(),
            error,
        })?;
    collect(diagnostics, &mut diags, table, prep, TemplatePart::Body);

    Ok(Rendered { path, content })
}

/// Rule 7: move the warnings of one pass into the summary and empty the pass.
fn collect(
    into: &mut Vec<Diagnostic>,
    diags: &mut Diagnostics,
    table: &Table,
    prep: &Prepared,
    part: TemplatePart,
) {
    for warning in diags.warnings() {
        into.push(Diagnostic {
            table: table.name.clone(),
            template: prep.name.clone(),
            part,
            warning: warning.clone(),
        });
    }
    diags.clear();
}

/// Rule 2: resolve a rendered name against the output directory, or refuse it.
///
/// A name is a *relative* path below `output_dir`, which is what lets
/// `${package.replace('.','/')}/${name.suffix.pascal}.java` write a package
/// tree. Everything that could reach outside is refused rather than clamped:
/// an absolute path, a Windows drive letter or UNC prefix, and any `..` at
/// all — even one that would land back inside, because a name that has to be
/// simplified to be judged is a name nobody meant to write.
///
/// The check is on the name, not on what the filesystem makes of it: a symlink
/// already inside the output directory can still lead out of it, and catching
/// that would mean canonicalising a path that does not exist yet.
fn resolve_output(output_dir: &Path, rendered: &str) -> Result<PathBuf, String> {
    let trimmed = rendered.trim();
    if trimmed.is_empty() {
        return Err("the output name rendered empty".to_string());
    }

    let mut path = output_dir.to_path_buf();
    for component in Path::new(trimmed).components() {
        match component {
            Component::Normal(part) => path.push(part),
            Component::CurDir => {}
            Component::ParentDir => {
                return Err(format!(
                    "output name '{trimmed}' leaves the output directory"
                ));
            }
            Component::RootDir | Component::Prefix(_) => {
                return Err(format!("output name '{trimmed}' is an absolute path"));
            }
        }
    }

    if path == output_dir {
        return Err(format!("output name '{trimmed}' names no file"));
    }
    Ok(path)
}

/// The summary line one [`Error`] makes.
///
/// The error already knows the table and the template, so nothing here has to
/// be told them twice.
fn failure_of(error: &Error, path: Option<PathBuf>) -> Failure {
    let (table, template) = match error {
        Error::Read { template, .. } | Error::Parse { template, .. } => (None, template.clone()),
        Error::Render {
            template, table, ..
        }
        | Error::OutputPath {
            template, table, ..
        } => (Some(table.clone()), template.clone()),
        // Neither index error can reach a summary: they are the preview's.
        Error::NoSuchTable(_) | Error::NoSuchTemplate(_) | Error::NoOutputDir => {
            (None, String::new())
        }
    };
    Failure {
        table,
        template,
        path,
        message: error.to_string(),
    }
}

/// Record a failed pair and report it in the same breath.
#[allow(clippy::too_many_arguments)]
fn report_failure(
    failed: &mut Vec<Failure>,
    progress: &dyn Fn(Progress),
    index: usize,
    total: usize,
    table: &Table,
    prep: &Prepared,
    error: Error,
) {
    let failure = failure_of(&error, None);
    let message = failure.message.clone();
    failed.push(failure);
    progress(Progress::File {
        index,
        total,
        table: table.name.clone(),
        template: prep.name.clone(),
        path: None,
        status: FileStatus::Failed(message),
    });
}

/// Record a skipped pair and report it in the same breath.
#[allow(clippy::too_many_arguments)]
fn skipped(
    skipped_files: &mut Vec<Skipped>,
    progress: &dyn Fn(Progress),
    index: usize,
    total: usize,
    table: &Table,
    prep: &Prepared,
    path: &Path,
    reason: SkipReason,
) {
    skipped_files.push(Skipped {
        path: path.to_path_buf(),
        table: table.name.clone(),
        template: prep.name.clone(),
        reason,
    });
    progress(Progress::File {
        index,
        total,
        table: table.name.clone(),
        template: prep.name.clone(),
        path: Some(path.to_path_buf()),
        status: FileStatus::Skipped(reason),
    });
}

/// A dry run's summary, in the shape [`Progress::Finished`] carries.
///
/// The event is the same for both entry points so that one progress dialog
/// serves both; the files a dry run "wrote" are the ones it rendered.
fn as_outcome(result: &DryRun) -> Outcome {
    Outcome {
        written: result.files.iter().map(|file| file.path.clone()).collect(),
        skipped: Vec::new(),
        failed: result.failed.clone(),
        cancelled: result.cancelled,
        diagnostics: result.diagnostics.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A run is driven from a background thread and reports to the UI over a
    /// channel, so the plan has to travel to that thread and every event has
    /// to travel back. Neither is expressed in a signature anywhere, which is
    /// why it is asserted here.
    const fn _the_job_can_be_moved_to_a_thread_and_report_back() {
        const fn assert_send<T: Send>() {}
        assert_send::<Plan>();
        assert_send::<Outcome>();
        assert_send::<Progress>();
        assert_send::<CancelToken>();
        assert_send::<crate::cancel::CancelHandle>();
        assert_send::<DryRun>();
        assert_send::<Preview>();
    }

    #[test]
    fn a_name_with_directories_in_it_lands_below_the_output_directory() {
        let out = Path::new("/out");
        assert_eq!(
            resolve_output(out, "com/abc/x/UserModel.java").unwrap(),
            Path::new("/out/com/abc/x/UserModel.java")
        );
        assert_eq!(
            resolve_output(out, "  Model.java \n").unwrap(),
            Path::new("/out/Model.java"),
            "a template file's trailing newline is not part of the name"
        );
        assert_eq!(
            resolve_output(out, "./a/./b.java").unwrap(),
            Path::new("/out/a/b.java")
        );
    }

    #[test]
    fn a_name_that_could_reach_outside_is_refused() {
        let out = Path::new("/out");
        for name in ["../escaped.java", "a/../../escaped.java", "/etc/passwd"] {
            assert!(resolve_output(out, name).is_err(), "{name} was not refused");
        }
        assert!(
            resolve_output(out, "a/../b.java").is_err(),
            "a '..' that lands back inside is still refused"
        );
    }

    #[test]
    fn a_name_that_is_no_name_is_refused() {
        let out = Path::new("/out");
        assert!(resolve_output(out, "").is_err());
        assert!(resolve_output(out, "   ").is_err());
        assert!(resolve_output(out, ".").is_err());
    }
}
