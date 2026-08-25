//! The generation run, as the window sees it: a progress dialog, the overwrite
//! question, and the result summary (architecture document, D11 and §4.6).
//!
//! `rudbgen-gen` knows nothing of gpui and blocks from beginning to end, so the
//! run happens on a thread of its own and reports back over a channel. Three
//! kinds of message come the other way:
//!
//! * [`Progress`], which moves the bar and adds a line to the log;
//! * the **overwrite question**, which arrives with a reply channel the run is
//!   already blocked on — the dialog answers it, and the run carries on;
//! * the end, which is a [`Outcome`] and turns the dialog into the summary.
//!
//! Cancelling sets the [`CancelToken`] the run checks at every file boundary,
//! so a cancelled run never leaves a half-written file behind. The dialog stays
//! up afterwards to show what it did manage to write, because a run that was
//! stopped half way is exactly the one whose list of files matters.

use std::path::{Path, PathBuf};
use std::sync::mpsc::{Sender, channel};

use futures::StreamExt;
use futures::channel::mpsc::{UnboundedSender, unbounded};
use gpui::{
    App, Context, EventEmitter, ScrollHandle, SharedString, Task, Window, div, prelude::*, px,
};
use rudbgen_core::OverwritePolicy;
use rudbgen_gen::{
    CancelHandle, CancelToken, Decision, FileStatus, Outcome, Overwrite, Plan, Progress, generate,
};
use rugpui::{Button, ButtonVariant, modal, theme};

use crate::i18n::ts;

/// How many lines of the run's log are kept.
///
/// A run of two hundred files writes two hundred lines and the dialog shows the
/// last screenful; keeping every one of a run over a whole schema would grow
/// without a bound nobody asked for.
const LOG_LIMIT: usize = 500;

/// Height of the log box inside the progress dialog.
const LOG_HEIGHT: f32 = 180.;

/// Width of both dialogs.
const DIALOG_WIDTH: f32 = 520.;

/// What one message from the run thread carries.
enum RunMessage {
    /// One step of the run.
    Step(Progress),
    /// The run is blocked on this file's fate.
    Ask {
        /// The file that is already there.
        path: PathBuf,
        /// Where the answer goes; the run thread is blocked on the other end.
        reply: Sender<Decision>,
    },
}

/// What the dialog tells the shell.
pub enum JobEvent {
    /// The dialog closed itself, and the keyboard should come back.
    Closed,
}

/// Where the run has got to.
enum Stage {
    /// Nothing is running and the dialog is out of the frame.
    Idle,
    /// The tables are being read; the run has not started yet.
    Loading {
        /// Tables described so far.
        done: usize,
        /// Tables to describe.
        total: usize,
    },
    /// The run is going.
    Running {
        /// Files handled so far.
        done: usize,
        /// Files the run would write.
        total: usize,
    },
    /// The run is blocked on an overwrite question.
    Asking {
        /// The file that is already there.
        path: PathBuf,
        /// Where the answer goes.
        reply: Sender<Decision>,
        /// Files handled so far.
        done: usize,
        /// Files the run would write.
        total: usize,
    },
    /// The run ended, and this is what it did.
    Summary(Box<Outcome>),
    /// The run never started, and this is why.
    Refused(SharedString),
}

/// The progress dialog, the overwrite question and the summary.
pub struct GenerationJob {
    /// Where the run has got to.
    stage: Stage,
    /// What the run has said, newest last.
    log: Vec<SharedString>,
    /// Stops the run at the next file boundary.
    cancel: Option<CancelHandle>,
    /// Where the files went, for the summary's one button.
    output_dir: Option<PathBuf>,
    /// Vertical scroll of the log, and of the summary's lists.
    scroll: ScrollHandle,
    /// Reads the run thread's channel; dropped when the dialog closes.
    _pump: Option<Task<()>>,
}

impl GenerationJob {
    /// A dialog that is not showing.
    pub fn new(_cx: &mut Context<Self>) -> Self {
        Self {
            stage: Stage::Idle,
            log: Vec::new(),
            cancel: None,
            output_dir: None,
            scroll: ScrollHandle::new(),
            _pump: None,
        }
    }

    /// Whether the dialog is on screen.
    pub fn is_open(&self) -> bool {
        !matches!(self.stage, Stage::Idle)
    }

    /// Whether the run is still going, and so must not be closed.
    pub fn is_busy(&self) -> bool {
        matches!(
            self.stage,
            Stage::Loading { .. } | Stage::Running { .. } | Stage::Asking { .. }
        )
    }

    /// What the run ended with, once it has.
    #[cfg(test)]
    pub fn outcome(&self) -> Option<&Outcome> {
        match &self.stage {
            Stage::Summary(outcome) => Some(outcome),
            _ => None,
        }
    }

    /// Whether the run is blocked on the overwrite question.
    #[cfg(test)]
    pub fn is_asking(&self) -> bool {
        matches!(self.stage, Stage::Asking { .. })
    }

    /// Why the run was refused, if it was.
    #[cfg(test)]
    pub fn refusal(&self) -> Option<&SharedString> {
        match &self.stage {
            Stage::Refused(message) => Some(message),
            _ => None,
        }
    }

    /// Puts the dialog up on the table-reading phase.
    ///
    /// The tables are read by the shell — it is what holds the session — so
    /// this is only the report of it. It is a phase of its own on the bar
    /// because reading twenty tables over a slow link takes longer than
    /// rendering them, and a dialog that says nothing for ten seconds looks
    /// stuck.
    pub fn begin_loading(&mut self, total: usize, cx: &mut Context<Self>) {
        self.stage = Stage::Loading { done: 0, total };
        self.log.clear();
        self.cancel = None;
        self.output_dir = None;
        self._pump = None;
        cx.notify();
    }

    /// One more table has been described.
    pub fn loaded(&mut self, name: &str, cx: &mut Context<Self>) {
        if let Stage::Loading { done, .. } = &mut self.stage {
            *done += 1;
        }
        self.note(ts!("progress.read_table", table = name.to_owned()), cx);
    }

    /// Takes the dialog down after a load that was not for a run of its own.
    ///
    /// A preview and a dry run put their answer in a tab rather than in a
    /// dialog, so the loading stage they share with a generate ends by simply
    /// going away. Refuses to touch anything but a load, so it can never pull a
    /// running job off the screen.
    pub fn finish_loading(&mut self, cx: &mut Context<Self>) {
        if matches!(self.stage, Stage::Loading { .. }) {
            self.stage = Stage::Idle;
            self.log.clear();
            cx.notify();
        }
    }

    /// The run cannot start, and this is why.
    pub fn refuse(&mut self, message: SharedString, cx: &mut Context<Self>) {
        self.stage = Stage::Refused(message);
        self.cancel = None;
        cx.notify();
    }

    /// Starts the run on a thread of its own.
    ///
    /// The plan is moved onto that thread, which is why it is taken by value:
    /// nothing on the UI side may hold a reference into a run that outlives the
    /// frame that started it.
    pub fn start(&mut self, plan: Plan, policy: OverwritePolicy, cx: &mut Context<Self>) {
        let total = plan.total();
        self.stage = Stage::Running { done: 0, total };
        self.output_dir = Some(plan.output_dir.clone());
        self.note(ts!("progress.started", count = total), cx);

        let token = CancelToken::new();
        self.cancel = Some(token.handle());

        let (tx, mut rx) = unbounded::<RunMessage>();
        // The one thread the run gets. `std::thread` rather than the background
        // executor on purpose: the *ask* policy blocks this thread until the
        // dialog answers, and a blocked pool thread is a pool one task short
        // for as long as the user is thinking about it.
        std::thread::Builder::new()
            .name("rudbgen-generate".to_string())
            .spawn(move || {
                let policy = overwrite_policy(policy, tx.clone());
                let sender = tx.clone();
                let report = move |progress: Progress| {
                    // A closed channel means the window went away; the run then
                    // has nobody to report to, and the cancel token is what
                    // stops it.
                    sender.unbounded_send(RunMessage::Step(progress)).ok();
                };
                generate(&plan, policy, &token, &report);
            })
            .expect("the operating system refused a thread for the generation run");

        self._pump = Some(cx.spawn(async move |job, cx| {
            while let Some(message) = rx.next().await {
                let carry_on = job
                    .update(cx, |job, cx| {
                        job.receive(message, cx);
                        job.is_busy()
                    })
                    .unwrap_or(false);
                if !carry_on {
                    break;
                }
            }
        }));
        cx.notify();
    }

    /// Takes one message off the run thread's channel.
    fn receive(&mut self, message: RunMessage, cx: &mut Context<Self>) {
        match message {
            RunMessage::Step(progress) => self.step(progress, cx),
            RunMessage::Ask { path, reply } => {
                let (done, total) = self.counts();
                let name = SharedString::from(path.display().to_string());
                self.stage = Stage::Asking {
                    path,
                    reply,
                    done,
                    total,
                };
                self.note(ts!("progress.conflict", path = name), cx);
            }
        }
    }

    /// Moves the bar and adds the line one step of the run produced.
    fn step(&mut self, progress: Progress, cx: &mut Context<Self>) {
        match progress {
            Progress::Started { .. } => {}
            Progress::Parsed { templates } => {
                self.note(ts!("progress.parsed", count = templates), cx);
            }
            Progress::File {
                index,
                total,
                table,
                template,
                path,
                status,
            } => {
                self.stage = Stage::Running { done: index, total };
                let name = path
                    .map(|path| path.display().to_string())
                    .unwrap_or_else(|| format!("{table} × {template}"));
                let line = match status {
                    FileStatus::Written => ts!("progress.written", path = name),
                    FileStatus::Skipped(_) => ts!("progress.skipped", path = name),
                    FileStatus::Failed(message) => {
                        ts!("progress.failed", path = name, reason = message)
                    }
                };
                self.note(line, cx);
            }
            Progress::Finished(outcome) => {
                self.cancel = None;
                self.stage = Stage::Summary(Box::new(outcome));
                cx.notify();
            }
        }
    }

    /// Where the bar is, whatever stage the dialog is in.
    fn counts(&self) -> (usize, usize) {
        match &self.stage {
            Stage::Loading { done, total } | Stage::Running { done, total } => (*done, *total),
            Stage::Asking { done, total, .. } => (*done, *total),
            _ => (0, 0),
        }
    }

    /// Appends one line to the log, dropping the oldest once it is full.
    fn note(&mut self, line: SharedString, cx: &mut Context<Self>) {
        self.log.push(line);
        if self.log.len() > LOG_LIMIT {
            self.log.remove(0);
        }
        cx.notify();
    }

    /// Stops the run at the next file boundary.
    ///
    /// While the run is blocked on an overwrite question there is no next file
    /// boundary to reach, so the question is answered with
    /// [`Decision::Cancel`], which is what unblocks it.
    pub fn cancel(&mut self, cx: &mut Context<Self>) {
        if let Some(handle) = self.cancel.take() {
            handle.cancel();
        }
        if let Stage::Asking { reply, .. } =
            std::mem::replace(&mut self.stage, Stage::Running { done: 0, total: 0 })
        {
            reply.send(Decision::Cancel).ok();
        }
        self.note(ts!("progress.cancelling"), cx);
    }

    /// Answers the overwrite question and lets the run go on.
    pub fn answer(&mut self, decision: Decision, cx: &mut Context<Self>) {
        let Stage::Asking {
            reply, done, total, ..
        } = std::mem::replace(&mut self.stage, Stage::Idle)
        else {
            return;
        };
        self.stage = Stage::Running { done, total };
        if reply.send(decision).is_err() {
            log::warn!("the generation run was gone before its question was answered");
        }
        cx.notify();
    }

    /// Closes the dialog, unless the run is still going.
    pub fn close(&mut self, cx: &mut Context<Self>) {
        if self.is_busy() {
            return;
        }
        self.stage = Stage::Idle;
        self.log.clear();
        self._pump = None;
        cx.emit(JobEvent::Closed);
        cx.notify();
    }

    /// Opens the directory the run wrote into, in the platform's file manager.
    fn open_output(&self, cx: &mut App) {
        let Some(dir) = &self.output_dir else {
            return;
        };
        cx.open_with_system(dir);
    }

    // --- rendering --------------------------------------------------------

    /// The bar, the log and the Cancel button.
    fn render_progress(&self, done: usize, total: usize, cx: &mut Context<Self>) -> gpui::AnyElement
    where
        Self: Sized,
    {
        let theme = theme(cx);
        let fraction = if total == 0 {
            0.
        } else {
            (done as f32 / total as f32).clamp(0., 1.)
        };
        let lines: Vec<SharedString> = self.log.clone();
        let this = cx.entity();

        modal(
            "generation-progress",
            ts!("progress.title"),
            px(DIALOG_WIDTH),
            div()
                .flex()
                .flex_col()
                .gap(px(10.))
                .child(
                    div()
                        .w_full()
                        .h(px(6.))
                        .rounded_full()
                        .bg(theme.surface)
                        .child(
                            div()
                                .h_full()
                                .w(gpui::relative(fraction))
                                .rounded_full()
                                .bg(theme.accent),
                        ),
                )
                .child(
                    div()
                        .text_size(px(12.))
                        .text_color(theme.text_muted)
                        .child(ts!("progress.count", done = done, total = total)),
                )
                .child(
                    div()
                        .id("generation-log")
                        .track_scroll(&self.scroll)
                        .flex()
                        .flex_col()
                        .gap(px(2.))
                        .h(px(LOG_HEIGHT))
                        .p(px(8.))
                        .overflow_y_scroll()
                        .rounded_md()
                        .bg(theme.surface)
                        .border_1()
                        .border_color(theme.border)
                        .text_size(px(11.))
                        .text_color(theme.text_muted)
                        .children(lines.into_iter().map(|line| div().child(line))),
                )
                .child(div().flex().flex_row().justify_end().child(
                    Button::new("generation-cancel", ts!("common.cancel")).on_click(
                        move |_, _window, cx| {
                            this.update(cx, |job, cx| job.cancel(cx));
                        },
                    ),
                )),
            // The backdrop does not dismiss a running job: there is nothing to
            // dismiss it to, and a click that quietly abandoned a run half way
            // through would be the worst answer of the three.
            |_window, _cx| {},
        )
        .into_any_element()
    }

    /// The overwrite question: one file, five answers.
    fn render_question(&self, path: &Path, cx: &mut Context<Self>) -> gpui::AnyElement {
        let theme = theme(cx);
        let shown = SharedString::from(path.display().to_string());
        let this = cx.entity();
        let answer = |label: SharedString, decision: Decision, id: &'static str, primary: bool| {
            let this = this.clone();
            Button::new(id, label)
                .variant(if primary {
                    ButtonVariant::Primary
                } else {
                    ButtonVariant::Secondary
                })
                .on_click(move |_, _window, cx| {
                    this.update(cx, |job, cx| job.answer(decision, cx));
                })
        };

        modal(
            "generation-conflict",
            ts!("summary.conflict_title"),
            px(DIALOG_WIDTH),
            div()
                .flex()
                .flex_col()
                .gap(px(12.))
                .child(
                    div()
                        .text_size(px(12.))
                        .text_color(theme.text)
                        .child(ts!("summary.conflict_body")),
                )
                .child(
                    div()
                        .p(px(8.))
                        .rounded_md()
                        .bg(theme.surface)
                        .text_size(px(11.))
                        .text_color(theme.text_muted)
                        .child(shown),
                )
                .child(
                    div()
                        .flex()
                        .flex_row()
                        .flex_wrap()
                        .justify_end()
                        .gap(px(6.))
                        .child(answer(
                            ts!("summary.overwrite"),
                            Decision::Overwrite,
                            "conflict-overwrite",
                            true,
                        ))
                        .child(answer(
                            ts!("summary.skip"),
                            Decision::Skip,
                            "conflict-skip",
                            false,
                        ))
                        .child(answer(
                            ts!("summary.overwrite_all"),
                            Decision::OverwriteAll,
                            "conflict-overwrite-all",
                            false,
                        ))
                        .child(answer(
                            ts!("summary.skip_all"),
                            Decision::SkipAll,
                            "conflict-skip-all",
                            false,
                        ))
                        .child(answer(
                            ts!("common.cancel"),
                            Decision::Cancel,
                            "conflict-cancel",
                            false,
                        )),
                ),
            |_window, _cx| {},
        )
        .into_any_element()
    }

    /// The result summary: what was written, what was skipped, what failed.
    fn render_summary(&self, outcome: &Outcome, cx: &mut Context<Self>) -> gpui::AnyElement {
        let theme = theme(cx);
        let this = cx.entity();
        let closing = cx.entity();
        let opening = cx.entity();

        let section = |title: SharedString, lines: Vec<SharedString>, color: gpui::Hsla| {
            (!lines.is_empty()).then(|| {
                div()
                    .flex()
                    .flex_col()
                    .gap(px(2.))
                    .child(div().text_size(px(12.)).text_color(color).child(title))
                    .children(lines.into_iter().map(|line| {
                        div()
                            .text_size(px(11.))
                            .text_color(theme.text_muted)
                            .child(line)
                    }))
            })
        };

        let written: Vec<SharedString> = outcome
            .written
            .iter()
            .map(|path| SharedString::from(path.display().to_string()))
            .collect();
        let skipped: Vec<SharedString> = outcome
            .skipped
            .iter()
            .map(|skip| SharedString::from(skip.path.display().to_string()))
            .collect();
        // The template and, where the engine gave one, the line: a template that
        // failed to parse is the one failure the user has to be able to find in
        // an editor, and "it failed" without a line is not a way to find it.
        let failed: Vec<SharedString> = outcome
            .failed
            .iter()
            .map(|failure| {
                SharedString::from(match &failure.table {
                    Some(table) => format!("{} × {}: {}", table, failure.template, failure.message),
                    None => format!("{}: {}", failure.template, failure.message),
                })
            })
            .collect();

        let headline = if outcome.cancelled {
            ts!("summary.cancelled")
        } else if outcome.failed.is_empty() {
            ts!("summary.done")
        } else {
            ts!("summary.with_failures")
        };

        modal(
            "generation-summary",
            ts!("summary.title"),
            px(DIALOG_WIDTH),
            div()
                .flex()
                .flex_col()
                .gap(px(10.))
                .min_h_0()
                .child(
                    div()
                        .text_size(px(13.))
                        .text_color(if outcome.failed.is_empty() {
                            theme.text
                        } else {
                            theme.danger
                        })
                        .child(headline),
                )
                .child(
                    div()
                        .text_size(px(12.))
                        .text_color(theme.text_muted)
                        .child(ts!(
                            "summary.counts",
                            written = outcome.written.len(),
                            skipped = outcome.skipped.len(),
                            failed = outcome.failed.len(),
                            warnings = outcome.diagnostics.len()
                        )),
                )
                .child(
                    div()
                        .id("generation-summary-lists")
                        .track_scroll(&self.scroll)
                        .flex()
                        .flex_col()
                        .gap(px(8.))
                        .min_h_0()
                        .max_h(px(240.))
                        .overflow_y_scroll()
                        .children(section(ts!("summary.written"), written, theme.success))
                        .children(section(ts!("summary.skipped"), skipped, theme.text_muted))
                        .children(section(ts!("summary.failed"), failed, theme.danger)),
                )
                .child(
                    div()
                        .flex()
                        .flex_row()
                        .justify_end()
                        .gap(px(8.))
                        .child(
                            Button::new("summary-open", ts!("summary.open_output"))
                                .variant(ButtonVariant::Secondary)
                                .disabled(self.output_dir.is_none())
                                .on_click(move |_, _window, cx| {
                                    opening.update(cx, |job, cx| job.open_output(cx));
                                }),
                        )
                        .child(
                            Button::new("summary-close", ts!("common.close"))
                                .variant(ButtonVariant::Primary)
                                .on_click(move |_, _window, cx| {
                                    closing.update(cx, |job, cx| job.close(cx));
                                }),
                        ),
                ),
            move |_window, cx| {
                this.update(cx, |job, cx| job.close(cx));
            },
        )
        .into_any_element()
    }

    /// The run was refused before it started: one message and a way out.
    fn render_refusal(&self, message: &SharedString, cx: &mut Context<Self>) -> gpui::AnyElement {
        let theme = theme(cx);
        let this = cx.entity();
        let dismiss = cx.entity();
        modal(
            "generation-refused",
            ts!("summary.title"),
            px(DIALOG_WIDTH),
            div()
                .flex()
                .flex_col()
                .gap(px(12.))
                .child(
                    div()
                        .text_size(px(12.))
                        .text_color(theme.danger)
                        .child(message.clone()),
                )
                .child(
                    div().flex().flex_row().justify_end().child(
                        Button::new("refused-close", ts!("common.close"))
                            .variant(ButtonVariant::Primary)
                            .on_click(move |_, _window, cx| {
                                this.update(cx, |job, cx| job.close(cx));
                            }),
                    ),
                ),
            move |_window, cx| {
                dismiss.update(cx, |job, cx| job.close(cx));
            },
        )
        .into_any_element()
    }
}

impl EventEmitter<JobEvent> for GenerationJob {}

impl Render for GenerationJob {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        match &self.stage {
            Stage::Idle => div().into_any_element(),
            Stage::Loading { done, total } | Stage::Running { done, total } => {
                let (done, total) = (*done, *total);
                self.render_progress(done, total, cx)
            }
            Stage::Asking { path, .. } => {
                let path = path.clone();
                self.render_question(&path, cx)
            }
            Stage::Summary(outcome) => {
                let outcome = outcome.clone();
                self.render_summary(&outcome, cx)
            }
            Stage::Refused(message) => {
                let message = message.clone();
                self.render_refusal(&message, cx)
            }
        }
    }
}

/// Turns the saved policy into the run's, with the *ask* case wired to the
/// dialog.
///
/// The callback runs on the generation thread and blocks it: it puts the
/// question on the channel and waits for the answer. A channel that has gone —
/// the window closed under the run — answers [`Decision::Cancel`], which is the
/// only safe reading of "nobody is there to say".
fn overwrite_policy(policy: OverwritePolicy, tx: UnboundedSender<RunMessage>) -> Overwrite {
    Overwrite::from_policy(policy, move |path| {
        let (reply, answer) = channel();
        if tx
            .unbounded_send(RunMessage::Ask {
                path: path.to_path_buf(),
                reply,
            })
            .is_err()
        {
            return Decision::Cancel;
        }
        answer.recv().unwrap_or(Decision::Cancel)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_two_settled_policies_never_ask() {
        let (tx, _rx) = unbounded();
        assert!(matches!(
            overwrite_policy(OverwritePolicy::Overwrite, tx.clone()),
            Overwrite::Overwrite
        ));
        assert!(matches!(
            overwrite_policy(OverwritePolicy::Skip, tx),
            Overwrite::Skip
        ));
    }

    #[test]
    fn a_question_nobody_is_left_to_answer_cancels_the_run() {
        // The window went away while the run was going: the receiving end of
        // the channel is gone, and a run that cannot ask must not guess.
        let (tx, rx) = unbounded();
        drop(rx);
        let Overwrite::Ask(ask) = overwrite_policy(OverwritePolicy::Ask, tx) else {
            panic!("the ask policy did not survive the conversion");
        };
        assert_eq!(ask(Path::new("/tmp/whatever.java")), Decision::Cancel);
    }

    #[test]
    fn the_answer_the_dialog_sends_is_the_one_the_run_gets() {
        let (tx, mut rx) = unbounded();
        let Overwrite::Ask(ask) = overwrite_policy(OverwritePolicy::Ask, tx) else {
            panic!("the ask policy did not survive the conversion");
        };
        // The run blocks on the answer, so the question has to be answered from
        // another thread — which is exactly what the dialog is.
        let asking = std::thread::spawn(move || ask(Path::new("/tmp/model.java")));
        let message = futures::executor::block_on(rx.next()).expect("the question was asked");
        let RunMessage::Ask { path, reply } = message else {
            panic!("the run reported something other than a question");
        };
        assert_eq!(path, Path::new("/tmp/model.java"));
        reply.send(Decision::SkipAll).expect("the run is waiting");
        assert_eq!(asking.join().expect("the run thread"), Decision::SkipAll);
    }
}
