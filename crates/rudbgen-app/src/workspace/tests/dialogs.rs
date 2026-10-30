use super::*;

/// Presses and releases `Enter`, as a keyboard does: a focused button takes
/// its click from the release.
fn press_enter(cx: &mut gpui::VisualTestContext) {
    let keystroke = gpui::Keystroke::parse("enter").expect("a keystroke");
    cx.simulate_event(gpui::KeyDownEvent {
        keystroke: keystroke.clone(),
        is_held: false,
        prefer_character_input: false,
    });
    cx.simulate_event(gpui::KeyUpEvent { keystroke });
    cx.run_until_parked();
}

/// A window with the shell's own key bindings installed, as `run` installs
/// them — which a plain test window does not have.
fn bound_window(cx: &mut gpui::TestAppContext) -> gpui::WindowHandle<Workspace> {
    cx.update(|cx| {
        app_settings::init(cx);
        rugpui::init(cx);
        rugpui_editor::init(cx);
        bootstrap::bind_shortcuts(cx);
    });
    silence_the_update_check();
    cx.add_window(|window, cx| Workspace::new(TitlebarStyle::Custom, window, cx))
}

/// A chord meant for the window does nothing while a dialog is up: it would
/// otherwise reach past the dialog to what is behind its backdrop.
#[gpui::test]
fn shortcuts_stand_down_while_a_dialog_is_up(cx: &mut gpui::TestAppContext) {
    let window = bound_window(cx);
    window
        .update(cx, |workspace, window, cx| {
            workspace.open_about(window, cx);
            assert!(workspace.dialog_open(cx));

            let visible = workspace.explorer_visible;
            workspace.toggle_explorer_action(&ToggleExplorer, window, cx);
            assert_eq!(
                workspace.explorer_visible, visible,
                "Ctrl+B folded a panel behind the dialog"
            );
            workspace.open_settings_action(&OpenSettings, window, cx);
            assert!(
                !workspace.settings.read(cx).is_open(),
                "Ctrl+, opened a second dialog over the first"
            );
            assert!(workspace.about.read(cx).is_open());

            // With the dialog gone, the same chord is the window's again.
            workspace.dismiss_dialog_action(&DismissDialog, window, cx);
            assert!(!workspace.dialog_open(cx));
            workspace.toggle_explorer_action(&ToggleExplorer, window, cx);
            assert_ne!(workspace.explorer_visible, visible);
        })
        .expect("the window is open");
}

/// `Escape` on the overwrite question answers it with Cancel — the answer it
/// stands for everywhere else — and the run winds down from there.
#[gpui::test]
fn escape_on_the_overwrite_question_answers_cancel(cx: &mut gpui::TestAppContext) {
    let window = bound_window(cx);
    let (reply, answer) = std::sync::mpsc::channel();
    window
        .update(cx, |workspace, window, cx| {
            workspace.job.update(cx, |job, cx| {
                job.ask_for_test(PathBuf::from("/tmp/AlbumModel.java"), reply, cx);
            });
            assert!(workspace.job.read(cx).is_asking());
            workspace.dismiss_dialog_action(&DismissDialog, window, cx);
            assert!(
                !workspace.job.read(cx).is_asking(),
                "the question is still up"
            );
        })
        .expect("the window is open");
    assert_eq!(
        answer.try_recv().expect("the run was answered"),
        rudbgen_gen::Decision::Cancel
    );
}

/// The question takes the keyboard when it goes up, so `Enter` answers it —
/// with the highlighted Overwrite — rather than breaking a line in whatever
/// template was being typed into behind it.
#[gpui::test]
fn enter_answers_the_overwrite_question(cx: &mut gpui::TestAppContext) {
    let window = bound_window(cx);
    let (reply, answer) = std::sync::mpsc::channel();
    window
        .update(cx, |workspace, _window, cx| {
            workspace.job.update(cx, |job, cx| {
                job.ask_for_test(PathBuf::from("/tmp/AlbumModel.java"), reply, cx);
            });
        })
        .expect("the window is open");
    let mut cx = gpui::VisualTestContext::from_window(*window, cx);
    cx.run_until_parked();
    press_enter(&mut cx);
    assert_eq!(
        answer.try_recv().expect("the run was answered"),
        rudbgen_gen::Decision::Overwrite
    );
}

/// The Generate tab's two questions close on `Escape`, and are dialogs as far
/// as the shortcuts are concerned while they are up.
#[gpui::test]
fn the_set_prompts_close_on_escape(cx: &mut gpui::TestAppContext) {
    let window = bound_window(cx);
    window
        .update(cx, |workspace, window, cx| {
            workspace
                .generate
                .update(cx, |pane, cx| pane.open_set_prompt(cx));
            assert!(workspace.dialog_open(cx), "the prompt is not a dialog");
            workspace.dismiss_dialog_action(&DismissDialog, window, cx);
            assert!(!workspace.generate.read(cx).prompt_open());
            assert!(!workspace.dialog_open(cx));
        })
        .expect("the window is open");
}

/// The close question answers `Enter` with Save once it has the keyboard, and
/// a tab-strip close of a tab with nothing unsaved asks nothing at all.
#[gpui::test]
fn enter_saves_a_template_being_closed(cx: &mut gpui::TestAppContext) {
    let dir = tempfile::tempdir().expect("tempdir");
    let file = dir.path().join("model.java");
    std::fs::write(&file, "class A {}\n").expect("the template is written");
    let window = bound_window(cx);
    window
        .update(cx, |workspace, window, cx| {
            workspace.open_template(file.clone(), cx);
            let at = workspace.active_template(cx).expect("the tab is on top");
            workspace.templates[at].update(cx, |pane, cx| pane.insert("// edit\n", window, cx));
            workspace.retitle_templates(cx);
        })
        .expect("the window is open");
    let mut cx = gpui::VisualTestContext::from_window(*window, cx);
    // The tab is drawn, and its editor has the keyboard, before the close is
    // asked for — which is where a user presses the close button from.
    cx.run_until_parked();
    window
        .update(&mut cx, |workspace, window, cx| {
            let index = workspace.pane.active_index();
            workspace.request_close_tab(index, window, cx);
            assert!(workspace.closing.is_some(), "the edit was not asked about");
        })
        .expect("the window is open");
    cx.run_until_parked();
    press_enter(&mut cx);
    assert!(
        std::fs::read_to_string(&file)
            .expect("the template is there")
            .contains("// edit\n"),
        "Enter did not save"
    );
    window
        .update(&mut cx, |workspace, _window, _cx| {
            assert!(workspace.closing.is_none());
            assert!(!workspace.has_templates(), "the tab stayed open");
        })
        .expect("the window is open");
}

/// A dirty rules draft is not thrown away by `Escape`: the question comes up
/// instead, a second `Escape` keeps editing, and only *Discard* lets it go.
#[gpui::test]
fn escape_asks_before_a_changed_rules_draft_goes(cx: &mut gpui::TestAppContext) {
    let window = bound_window(cx);
    window
        .update(cx, |workspace, window, cx| {
            workspace.abbreviations.update(cx, |dialog, cx| {
                dialog.open(&rudbgen_core::AbbreviationStore::default(), Vec::new(), cx);
                assert!(!dialog.is_dirty(cx), "an untouched draft reads as edited");
                dialog.write_row(
                    0,
                    &rudbgen_core::AbbreviationRule {
                        enabled: true,
                        whole_name: false,
                        abbreviation: "EMP".to_string(),
                        replacement: "Employee".to_string(),
                    },
                    cx,
                );
                assert!(dialog.is_dirty(cx));
            });
            workspace.dismiss_dialog_action(&DismissDialog, window, cx);
            assert!(
                workspace.abbreviations.read(cx).is_open(),
                "Escape threw the draft away"
            );
            assert!(workspace.abbreviations.read(cx).is_discarding());
            // A second press keeps editing, with the draft as it was.
            workspace.dismiss_dialog_action(&DismissDialog, window, cx);
            assert!(workspace.abbreviations.read(cx).is_open());
            assert!(!workspace.abbreviations.read(cx).is_discarding());
            assert!(workspace.abbreviations.read(cx).is_dirty(cx));
            // A command that would open another dialog leaves this one be.
            workspace.open_settings(window, cx);
            assert!(workspace.abbreviations.read(cx).is_open());
            assert!(!workspace.settings.read(cx).is_open());
        })
        .expect("the window is open");
}

/// The shell's own right-click menus and the welcome row's delete question
/// are on `Escape`'s list too.
#[gpui::test]
fn escape_takes_the_tab_menu_down_first(cx: &mut gpui::TestAppContext) {
    let window = bound_window(cx);
    window
        .update(cx, |workspace, window, cx| {
            workspace.tab_menu = Some((0, gpui::point(px(10.), px(10.))));
            workspace.dismiss_dialog_action(&DismissDialog, window, cx);
            assert!(workspace.tab_menu.is_none());
            workspace.deleting_profile = Some(uuid::Uuid::new_v4());
            assert!(workspace.dialog_open(cx));
            workspace.dismiss_dialog_action(&DismissDialog, window, cx);
            assert!(workspace.deleting_profile.is_none());
        })
        .expect("the window is open");
}

/// A close with a template unsaved asks first, naming it; with nothing to
/// lose the guard lets the close straight through.
#[gpui::test]
fn the_close_guard_lists_what_a_close_would_lose(cx: &mut gpui::TestAppContext) {
    let dir = tempfile::tempdir().expect("tempdir");
    let file = dir.path().join("model.java");
    std::fs::write(&file, "class A {}\n").expect("the template is written");
    let window = bound_window(cx);
    window
        .update(cx, |workspace, window, cx| {
            assert!(workspace.unsaved_lines(cx).is_empty());
            workspace.open_template(file.clone(), cx);
            let at = workspace.active_template(cx).expect("the tab is on top");
            workspace.templates[at].update(cx, |pane, cx| pane.insert("// edit\n", window, cx));
            let lines = workspace.unsaved_lines(cx);
            assert_eq!(lines.len(), 1);
            assert!(lines[0].contains("model.java"), "{lines:?}");
            rugpui_shell::request_close(window, cx);
        })
        .expect("the window is open");
    cx.run_until_parked();
    window
        .update(cx, |workspace, _window, cx| {
            assert!(
                workspace.close_guard.read(cx).is_open(),
                "the close did not ask"
            );
            assert!(workspace.dialog_open(cx), "the question is not a dialog");
        })
        .expect("the window closed over an unsaved template");

    // Save all writes the tab, and the close goes on.
    window
        .update(cx, |workspace, _window, cx| {
            assert!(workspace.save_all_templates(cx));
            assert!(workspace.unsaved_lines(cx).is_empty());
        })
        .expect("the window is open");
    assert!(
        std::fs::read_to_string(&file)
            .expect("the template is there")
            .contains("// edit")
    );
}

/// A press on the backdrop of a dialog with a changed draft does nothing — not
/// even the question `Escape` asks — while over an untouched one it closes it.
#[gpui::test]
fn the_backdrop_leaves_a_changed_draft_alone(cx: &mut gpui::TestAppContext) {
    let window = bound_window(cx);
    window
        .update(cx, |workspace, _window, cx| {
            workspace.abbreviations.update(cx, |dialog, cx| {
                dialog.open(&rudbgen_core::AbbreviationStore::default(), Vec::new(), cx);
                dialog.write_row(
                    0,
                    &rudbgen_core::AbbreviationRule {
                        enabled: true,
                        whole_name: false,
                        abbreviation: "EMP".to_string(),
                        replacement: "Employee".to_string(),
                    },
                    cx,
                );
            });
        })
        .expect("the window is open");
    let mut cx = gpui::VisualTestContext::from_window(*window, cx);
    cx.run_until_parked();
    // The window's top left corner is backdrop, well clear of the panel.
    cx.simulate_click(gpui::point(px(4.), px(4.)), gpui::Modifiers::none());
    cx.run_until_parked();
    window
        .update(&mut cx, |workspace, _window, cx| {
            assert!(
                workspace.abbreviations.read(cx).is_open(),
                "the click dropped the draft"
            );
            assert!(
                !workspace.abbreviations.read(cx).is_discarding(),
                "a stray click asked a question"
            );
            workspace.abbreviations.update(cx, |dialog, cx| {
                dialog.open(&rudbgen_core::AbbreviationStore::default(), Vec::new(), cx);
            });
        })
        .expect("the window is open");
    cx.run_until_parked();
    cx.simulate_click(gpui::point(px(4.), px(4.)), gpui::Modifiers::none());
    cx.run_until_parked();
    window
        .update(&mut cx, |workspace, _window, cx| {
            assert!(
                !workspace.abbreviations.read(cx).is_open(),
                "an untouched dialog stayed up"
            );
        })
        .expect("the window is open");
}

/// The interface size is the scale everything is drawn at: 14 pt is 1.0, and
/// a saved size moves it — the preview included.
#[gpui::test]
fn the_interface_size_sets_the_ui_scale(cx: &mut gpui::TestAppContext) {
    let _window = bound_window(cx);
    cx.update(|cx| {
        let mut settings = AppSettings::default();
        bootstrap::apply_themes(&settings, cx);
        assert_eq!(rugpui::ui_scale(cx), 1.0);
        settings.ui_font_size = 21.0;
        bootstrap::apply_themes(&settings, cx);
        assert_eq!(rugpui::ui_scale(cx), 1.5);
        assert_eq!(rugpui::TextSize::Xs.size(cx), px(16.5));
        bootstrap::apply_themes(&AppSettings::default(), cx);
    });
}

/// `F10` opens the application menu on its first row, and not over a dialog.
#[cfg(not(target_os = "macos"))]
#[gpui::test]
fn f10_opens_the_app_menu(cx: &mut gpui::TestAppContext) {
    // The menu names the application, which the shell is told at start-up.
    cx.update(|cx| rugpui_shell::init(IDENTITY, cx));
    let window = bound_window(cx);
    window
        .update(cx, |workspace, window, cx| {
            window.focus(&workspace.focus_handle, cx);
        })
        .expect("the window is open");
    let mut cx = gpui::VisualTestContext::from_window(*window, cx);
    cx.run_until_parked();
    cx.simulate_keystrokes("f10");
    window
        .update(&mut cx, |workspace, window, cx| {
            assert!(workspace.menu_open, "F10 did not open the menu");
            assert!(workspace.menu_by_keyboard, "the menu opened on no row");
            workspace.set_menu_open(false, cx);
            assert!(!workspace.menu_by_keyboard);
            workspace.open_about(window, cx);
        })
        .expect("the window is open");
    cx.simulate_keystrokes("f10");
    window
        .update(&mut cx, |workspace, _window, _cx| {
            assert!(!workspace.menu_open, "F10 opened the menu behind a dialog");
        })
        .expect("the window is open");
}
