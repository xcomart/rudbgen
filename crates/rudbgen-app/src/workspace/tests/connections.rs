use super::generation::{ALBUM_AND_ARTIST, h2_window};
use super::*;

/// The repository's Java template, which every window here generates from.
fn shipped_template() -> PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../templates/java_model.java")
        .canonicalize()
        .expect("the shipped template is in the repository")
}

/// The name a tick of the table `name` in H2's `PUBLIC` of `catalog` is
/// saved as.
fn public(catalog: &str, name: &str) -> rudbgen_core::TableName {
    rudbgen_core::TableName {
        catalog: catalog.to_string(),
        schema: "PUBLIC".to_string(),
        name: name.to_string(),
    }
}

/// The catalog H2 files the window's `PUBLIC` under — the database's own
/// name, which a saved tick carries.
fn catalog_of(cx: &mut gpui::TestAppContext, window: &gpui::WindowHandle<Workspace>) -> String {
    window
        .update(cx, |workspace, _window, cx| {
            workspace
                .explorer
                .read(cx)
                .visible_tables(cx)
                .first()
                .map(|key| key.catalog.clone())
                .expect("the schema is listed")
        })
        .expect("the window is open")
}

/// The ticks a profile was saved with come back on the next connection, and
/// one naming a table the database no longer has is taken off and reported
/// rather than handed to a run.
#[gpui::test]
fn the_ticks_a_connection_was_left_with_come_back(cx: &mut gpui::TestAppContext) {
    let out = tempfile::tempdir().expect("tempdir");
    let window = h2_window(
        cx,
        "ticks-back",
        &ALBUM_AND_ARTIST,
        out.path(),
        shipped_template(),
        Vec::new(),
        false,
    );
    let catalog = catalog_of(cx, &window);
    // The same connection opened again, its profile saved with two ticks —
    // one of them a table that has been dropped since.
    window
        .update(cx, |workspace, _window, cx| {
            if let ConnectionState::Open { profile, .. } = &mut workspace.connection {
                profile.generation.selected_tables = vec![
                    public(&catalog, "T_SAMPLE_ALBUM"),
                    public(&catalog, "T_SAMPLE_GONE"),
                ];
            }
            workspace.reset_panels(cx);
        })
        .expect("the window is open");
    cx.run_until_parked();
    window
        .update(cx, |workspace, _window, cx| {
            let selection: Vec<String> = workspace
                .explorer
                .read(cx)
                .selection(cx)
                .into_iter()
                .map(|key| key.name)
                .collect();
            assert_eq!(selection, vec!["T_SAMPLE_ALBUM".to_string()]);
            let missing: Vec<&str> = workspace
                .explorer
                .read(cx)
                .missing()
                .iter()
                .map(|key| key.name.as_str())
                .collect();
            assert_eq!(missing, vec!["T_SAMPLE_GONE"]);
            // The table that went is gone from what the profile keeps, too.
            assert_eq!(
                workspace.generate.read(cx).ticks(),
                &[public(&catalog, "T_SAMPLE_ALBUM")]
            );
        })
        .expect("the window is open");

    // A tick made now is what the profile will be saved with.
    window
        .update(cx, |workspace, _window, cx| {
            let artist = workspace
                .explorer
                .read(cx)
                .visible_tables(cx)
                .into_iter()
                .find(|key| key.name == "T_SAMPLE_ARTIST")
                .expect("the artist is a row");
            workspace.explorer.update(cx, |explorer, cx| {
                explorer.toggle_tick(&explorer::NodeId::Table(artist), cx);
            });
        })
        .expect("the window is open");
    cx.run_until_parked();
    window
        .update(cx, |workspace, _window, cx| {
            assert_eq!(
                workspace.generate.read(cx).ticks(),
                &[
                    public(&catalog, "T_SAMPLE_ALBUM"),
                    public(&catalog, "T_SAMPLE_ARTIST")
                ]
            );
        })
        .expect("the window is open");
}

/// Picking the connection that is already open — in the title bar, or on a
/// row that is still opening — starts nothing over.
#[gpui::test]
fn the_open_connection_picked_again_is_left_alone(cx: &mut gpui::TestAppContext) {
    let out = tempfile::tempdir().expect("tempdir");
    let window = h2_window(
        cx,
        "picked-again",
        &ALBUM_AND_ARTIST,
        out.path(),
        shipped_template(),
        Vec::new(),
        true,
    );
    window
        .update(cx, |workspace, window, cx| {
            let profile = workspace
                .connection
                .profile()
                .cloned()
                .expect("a connection is open");
            workspace.profiles.upsert(profile.clone());
            let epoch = workspace.connection_epoch;
            let ticked = workspace.explorer.read(cx).selected_count(cx);
            workspace.open_profile(profile.id, window, cx);
            assert!(matches!(workspace.connection, ConnectionState::Open { .. }));
            assert_eq!(workspace.connection_epoch, epoch, "it reconnected");
            assert_eq!(workspace.explorer.read(cx).selected_count(cx), ticked);
        })
        .expect("the window is open");
}

/// A database that refuses the password puts the window's own question up,
/// with the reason; the right password then opens the session.
#[gpui::test]
fn a_refused_password_is_asked_for_again(cx: &mut gpui::TestAppContext) {
    cx.update(|cx| {
        app_settings::init(cx);
        rugpui::init(cx);
        rugpui_grid::init(cx);
        rugpui_editor::init(cx);
    });
    cx.executor().allow_parking();
    silence_the_update_check();

    // The database has to exist, with a password, before a wrong one can be
    // refused by it — and `DB_CLOSE_DELAY=-1` keeps it between the two.
    let mut profile = connection::h2::profile("refused");
    profile.url = format!("{};DB_CLOSE_DELAY=-1", profile.url);
    let driver = connection::h2::driver();
    let created = connection::connect(
        &profile,
        &driver,
        &Credentials::typed(Some("hunter2".into()), None),
        &AppSettings::default(),
    )
    .expect("the first connection creates the database");

    let window = cx.add_window(|window, cx| Workspace::new(TitlebarStyle::Custom, window, cx));
    window
        .update(cx, |workspace, _window, cx| {
            workspace.start_connect(
                profile.clone(),
                driver.clone(),
                Some(Credentials::typed(Some("not-hunter2".into()), None)),
                None,
                cx,
            );
        })
        .expect("the window is open");
    wait_until(cx, &window, |workspace| {
        !matches!(workspace.connection, ConnectionState::Connecting { .. })
    });
    window
        .update(cx, |workspace, window, cx| {
            assert!(matches!(
                workspace.connection,
                ConnectionState::Failed { hint: Some(_), .. }
            ));
            let prompt = workspace
                .password_prompt
                .as_mut()
                .expect("the password was not asked for again");
            assert!(prompt.reason.is_some(), "the question does not say why");
            // Nothing is to go into this machine's keychain from a test.
            prompt.remember = false;
            prompt
                .input
                .update(cx, |input, cx| input.set_content("hunter2", cx));
            workspace.submit_password(window, cx);
            assert!(workspace.password_prompt.is_none());
        })
        .expect("the window is open");
    wait_until(cx, &window, |workspace| {
        !matches!(workspace.connection, ConnectionState::Connecting { .. })
    });
    window
        .update(cx, |workspace, _window, _cx| {
            assert!(
                matches!(workspace.connection, ConnectionState::Open { .. }),
                "the right password did not open the session"
            );
        })
        .expect("the window is open");
    window
        .update(cx, |workspace, _window, cx| workspace.disconnect(cx))
        .expect("the window is open");
    created.close().expect("close");
}

/// A run over a ticked template whose tab has unsaved edits asks first: Cancel
/// runs nothing, and *Save and run* writes the tab before the run reads it.
#[gpui::test]
fn a_run_over_an_unsaved_template_asks_first(cx: &mut gpui::TestAppContext) {
    let out = tempfile::tempdir().expect("tempdir");
    let dir = tempfile::tempdir().expect("tempdir");
    let template = dir.path().join("model.java");
    std::fs::copy(shipped_template(), &template).expect("the template is copied");
    let window = h2_window(
        cx,
        "unsaved-run",
        &ALBUM_AND_ARTIST,
        out.path(),
        template.clone(),
        Vec::new(),
        true,
    );
    window
        .update(cx, |workspace, window, cx| {
            workspace.open_template(template.clone(), cx);
            let at = workspace.active_template(cx).expect("the tab is on top");
            workspace.templates[at].update(cx, |pane, cx| {
                pane.insert("// edited\n", window, cx);
            });
            assert_eq!(workspace.unsaved_run_templates(cx), vec![at]);

            workspace.start_run(RunKind::DryRun, cx);
            assert_eq!(workspace.unsaved_run, Some(RunKind::DryRun));
            assert!(!workspace.job.read(cx).is_busy(), "the run did not wait");
            workspace.answer_unsaved_run(None, window, cx);
            assert!(workspace.unsaved_run.is_none());
            assert!(!workspace.job.read(cx).is_busy(), "Cancel still ran");

            workspace.start_run(RunKind::DryRun, cx);
            workspace.answer_unsaved_run(Some(true), window, cx);
            assert!(
                workspace.unsaved_run_templates(cx).is_empty(),
                "the tab was not saved"
            );
        })
        .expect("the window is open");
    assert!(
        std::fs::read_to_string(&template)
            .expect("the template is there")
            .contains("// edited"),
        "Save and run did not save"
    );
    cx.run_until_parked();
}

/// Steps gpui and the threads behind it until `done` holds of the window.
fn wait_until(
    cx: &mut gpui::TestAppContext,
    window: &gpui::WindowHandle<Workspace>,
    done: impl Fn(&Workspace) -> bool,
) {
    for _ in 0..1000 {
        cx.run_until_parked();
        let finished = window
            .update(cx, |workspace, _window, _cx| done(workspace))
            .expect("the window is open");
        if finished {
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    panic!("the window never got there");
}
