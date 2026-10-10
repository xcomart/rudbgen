use crate::{abbreviation_dialog, explorer};
use std::sync::Once;

use super::*;

/// Turns the shell's start-up release check off, once per test process.
///
/// Belt and braces beside the `cfg!(test)` guard in [`Workspace::new`]:
/// every test that builds a workspace calls this first, so that no path
/// into `rugpui_shell::update::check` — this one or a later one — can put an
/// HTTPS request to GitHub inside a test run.
fn silence_the_update_check() {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| update::set_startup_check_enabled(false));
}

/// A stand-in registry listing: the six built-in ids, each of which a
/// chrome theme shares a name with, plus one dark theme of the user's own
/// that none does — which is the case the rule below has to keep straight.
fn entries() -> Vec<EditorThemeEntry> {
    [
        ("one-dark", true, true),
        ("one-light", false, true),
        ("solarized-dark", true, true),
        ("solarized-light", false, true),
        ("gruvbox-dark", true, true),
        ("dracula", true, true),
        ("tokyo-night", true, false),
    ]
    .into_iter()
    .map(|(id, dark, builtin)| EditorThemeEntry {
        id: id.to_string(),
        name: id.to_string(),
        dark,
        builtin,
    })
    .collect()
}

/// What the palette is showing beside the table's `name` field.
fn palette_example(workspace: &Workspace, cx: &App) -> Option<SharedString> {
    workspace
        .palette
        .read(cx)
        .items()
        .iter()
        .find(|item| item.section == palette::Section::Table && item.name == "name")
        .expect("the palette lists the table's name")
        .example
        .clone()
}

/// Waits for the run to reach its summary, and hands back what it said.
///
/// The generation thread is a real thread, so the test alternates between
/// draining gpui's tasks and letting that thread run. Ten seconds is far
/// longer than two files take and short enough to fail rather than hang.
fn wait_for_outcome(
    cx: &mut gpui::TestAppContext,
    window: &gpui::WindowHandle<Workspace>,
) -> rudbgen_gen::Outcome {
    for _ in 0..1000 {
        cx.run_until_parked();
        let done = window
            .update(cx, |workspace, _window, cx| {
                workspace.job.read(cx).outcome().cloned()
            })
            .expect("the window is open");
        if let Some(outcome) = done {
            return outcome;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    window
        .update(cx, |workspace, _window, cx| {
            if let Some(refusal) = workspace.job.read(cx).refusal() {
                panic!("the run was refused: {refusal}");
            }
        })
        .expect("the window is open");
    panic!("the run never finished");
}

/// Waits for the overwrite question and answers it.
fn answer_conflict(
    cx: &mut gpui::TestAppContext,
    window: &gpui::WindowHandle<Workspace>,
    decision: rudbgen_gen::Decision,
) {
    for _ in 0..1000 {
        cx.run_until_parked();
        let answered = window
            .update(cx, |workspace, _window, cx| {
                workspace.job.update(cx, |job, cx| {
                    if job.is_asking() {
                        job.answer(decision, cx);
                        return true;
                    }
                    job.outcome().is_some()
                })
            })
            .expect("the window is open");
        if answered {
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    panic!("the run never asked about the file it was about to replace");
}

mod generation;
mod settings;
mod templates;
mod workspace;
