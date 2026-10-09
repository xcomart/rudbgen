use super::*;

/// The two panels of §4.2 are switches, and the switches are remembered.
///
/// Both of them are also out of the frame while nothing is connected, which
/// is the state every test here runs in: the assertion is about the switch
/// and the setting behind it, not about pixels.
#[gpui::test]
fn the_panels_toggle_and_the_layout_is_remembered(cx: &mut gpui::TestAppContext) {
    cx.update(|cx| {
        app_settings::init(cx);
        rugpui::init(cx);
    });
    silence_the_update_check();
    let window = cx.add_window(|window, cx| Workspace::new(TitlebarStyle::Custom, window, cx));

    window
        .update(cx, |workspace, window, cx| {
            // Both start where the settings left them, which on a first run
            // is on: a workbench whose tree is hidden until it is found in a
            // menu is a workbench that looks like it has none.
            assert!(workspace.explorer_visible);
            assert!(workspace.inspector_visible);
            // ...and out of the frame regardless, with nothing connected.
            assert!(!workspace.explorer_showing());
            assert!(!workspace.inspector_showing());

            workspace.toggle_explorer_action(&ToggleExplorer, window, cx);
            assert!(!workspace.explorer_visible);
            assert!(
                !app_settings::current(cx).explorer_visible,
                "the switch was flipped and not written down"
            );
            // The keyboard comes back in the same update the subtree went
            // in; see [`Workspace::hid_a_panel`].
            assert!(workspace.focus_handle.is_focused(window));

            workspace.toggle_inspector_action(&ToggleInspector, window, cx);
            assert!(!workspace.inspector_visible);
            assert!(!app_settings::current(cx).inspector_visible);

            workspace.toggle_explorer_action(&ToggleExplorer, window, cx);
            assert!(workspace.explorer_visible);
            assert!(app_settings::current(cx).explorer_visible);
        })
        .expect("the window is open");
}

/// The status bar counts the ticks, wherever they were made.
#[gpui::test]
fn the_status_bar_counts_the_ticked_tables(cx: &mut gpui::TestAppContext) {
    cx.update(|cx| {
        app_settings::init(cx);
        rugpui::init(cx);
    });
    silence_the_update_check();
    let window = cx.add_window(|window, cx| Workspace::new(TitlebarStyle::Custom, window, cx));

    window
        .update(cx, |workspace, _window, cx| {
            assert_eq!(workspace.explorer.read(cx).selected_count(cx), 0);
            workspace.explorer.update(cx, |explorer, cx| {
                explorer.deliver_schemas(
                    Ok(vec![rudbgen_meta::Schema {
                        catalog: String::new(),
                        schema: "PUBLIC".to_string(),
                        name: "PUBLIC".to_string(),
                    }]),
                    cx,
                );
            });
        })
        .expect("the window is open");
    cx.run_until_parked();

    window
        .update(cx, |workspace, _window, cx| {
            let key = SchemaKey {
                catalog: String::new(),
                schema: "PUBLIC".to_string(),
                name: "PUBLIC".to_string(),
            };
            workspace.explorer.update(cx, |explorer, cx| {
                explorer.deliver_tables(
                    key,
                    Ok(vec![rudbgen_meta::TableRef {
                        catalog: String::new(),
                        schema: "PUBLIC".to_string(),
                        name: "T_SAMPLE_ALBUM".to_string(),
                        kind: rudbgen_meta::KIND_TABLE.to_string(),
                        ..rudbgen_meta::TableRef::default()
                    }]),
                    cx,
                );
            });
            workspace.explorer.update(cx, |explorer, cx| {
                explorer.toggle_tick(
                    &explorer::NodeId::Table(TableKey {
                        catalog: String::new(),
                        schema: "PUBLIC".to_string(),
                        name: "T_SAMPLE_ALBUM".to_string(),
                    }),
                    cx,
                );
            });
            assert_eq!(workspace.explorer.read(cx).selected_count(cx), 1);
            assert_ne!(
                ts!("statusbar.tables_selected", count = 1),
                ts!("statusbar.no_selection"),
                "the bar would say the same thing either way"
            );
        })
        .expect("the window is open");
}

#[gpui::test]
fn the_dialogs_open_from_their_actions_and_close_on_escape(cx: &mut gpui::TestAppContext) {
    cx.update(|cx| {
        app_settings::init(cx);
        rugpui::init(cx);
    });
    silence_the_update_check();
    let window = cx.add_window(|window, cx| Workspace::new(TitlebarStyle::Custom, window, cx));

    window
        .update(cx, |workspace, window, cx| {
            workspace.open_about(window, cx);
            assert!(workspace.about.read(cx).is_open());
            // One at a time: opening the settings dialog puts the about box
            // away rather than stacking on top of it.
            workspace.open_settings(window, cx);
            assert!(!workspace.about.read(cx).is_open());
            assert!(workspace.settings.read(cx).is_open());

            workspace.dismiss_dialog_action(&DismissDialog, window, cx);
            assert!(
                !workspace.settings.read(cx).is_open(),
                "escape left the settings dialog up"
            );
            assert!(!workspace.dialog_open(cx));
        })
        .expect("the window is open");
    cx.run_until_parked();
}
