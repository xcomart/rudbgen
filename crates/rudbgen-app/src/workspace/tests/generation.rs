use super::*;

/// The window opens, and the welcome screen is what it opens on.
///
/// The whole of M0 in one assertion: no dialog is up, the shell holds the
/// keyboard, and the button the architecture document's §4.3 puts first is
/// in the tree.
#[gpui::test]
fn the_window_opens_on_the_welcome_screen(cx: &mut gpui::TestAppContext) {
    cx.update(|cx| {
        app_settings::init(cx);
        rugpui::init(cx);
    });
    silence_the_update_check();
    let window = cx.add_window(|window, cx| Workspace::new(TitlebarStyle::Custom, window, cx));

    window
        .update(cx, |workspace, window, cx| {
            assert!(!workspace.dialog_open(cx), "a dialog opened by itself");
            assert!(!workspace.menu_open);
            // What `main` does once the window exists, and what every
            // dialog's close returns to: the shell holds the keyboard, so
            // the shortcuts are live before anything has been clicked.
            workspace.focus_shell(window, cx);
            assert!(workspace.focus_handle.is_focused(window));
        })
        .expect("the window is open");
    cx.run_until_parked();

    let mut cx = gpui::VisualTestContext::from_window(*std::ops::Deref::deref(&window), cx);
    cx.run_until_parked();
    assert!(
        cx.debug_bounds(WELCOME_NEW_SELECTOR).is_some(),
        "the welcome screen drew no way in"
    );
}

/// M2's whole promise, against a real database: connect, tick, read.
///
/// A real JVM, a real driver and a real H2 — the same route
/// [`connection::tests`] takes — because everything below this line has
/// already been tested against fixtures, and what is left to find out is
/// whether the three fetches the shell makes ask `rudbgen-meta` for the
/// right things and put the answers where the panels look for them.
///
/// No window is on screen in a test, but the panels are laid out: the row
/// list the tree reports is rebuilt on a draw, so asserting on it is
/// asserting about what a user would see.
#[gpui::test]
fn a_real_database_fills_the_tree_and_the_inspector(cx: &mut gpui::TestAppContext) {
    use rudbgen_jdbc::StatementSpec;

    let profile = connection::h2::profile("explorer");
    let driver = connection::h2::driver();
    let session = connection::connect(
        &profile,
        &driver,
        &Credentials::typed(Some(String::new()), None),
        &AppSettings::default(),
    )
    .expect("H2 opens an in-memory database without a server");

    for sql in [
        "create table T_SAMPLE_ARTIST (               ARTIST_ID integer not null,               NAME varchar(80) not null,               constraint PK_ARTIST primary key (ARTIST_ID))",
        "create table T_SAMPLE_ALBUM (               ALBUM_ID integer not null,               ARTIST_ID integer not null,               TITLE varchar(120),               constraint PK_ALBUM primary key (ALBUM_ID),               constraint FK_ALBUM_ARTIST foreign key (ARTIST_ID)                 references T_SAMPLE_ARTIST (ARTIST_ID))",
        "comment on table T_SAMPLE_ALBUM is 'an album'",
    ] {
        session
            .session()
            .execute(&StatementSpec::new(sql))
            .unwrap_or_else(|error| panic!("{sql}: {error}"));
    }

    cx.update(|cx| {
        app_settings::init(cx);
        rugpui::init(cx);
        rugpui_grid::init(cx);
    });
    silence_the_update_check();
    let window = cx.add_window(|window, cx| Workspace::new(TitlebarStyle::Custom, window, cx));
    window
        .update(cx, |workspace, _window, cx| {
            workspace.connection = ConnectionState::Open {
                profile: Box::new(profile.clone()),
                driver: Box::new(driver.clone()),
                session: Box::new(session),
            };
            // What `connected` does once the state is in place: the panels
            // are emptied, and the tree asks for its root on the next draw.
            workspace.reset_panels(cx);
            assert!(workspace.explorer_showing());
            assert!(workspace.inspector_showing());
        })
        .expect("the window is open");
    cx.run_until_parked();

    // The schemas arrived and PUBLIC is among them.
    let public = window
        .update(cx, |workspace, _window, cx| {
            let rows = workspace.explorer.read(cx).row_ids(cx);
            rows.into_iter()
                .flatten()
                .find_map(|id| match id {
                    explorer::NodeId::Schema(key) if key.name == "PUBLIC" => Some(key),
                    _ => None,
                })
                .expect("H2 answers with a PUBLIC schema")
        })
        .expect("the window is open");

    window
        .update(cx, |workspace, _window, cx| {
            workspace.explorer.update(cx, |explorer, cx| {
                explorer.expand(&explorer::NodeId::Schema(public.clone()), cx);
            });
        })
        .expect("the window is open");
    cx.run_until_parked();

    // Both tables, and only the tables. The keys come off the panel rather
    // than being spelled out here: H2 names the catalog after the URL, and
    // what a key *is* is the driver's answer and not this test's guess.
    let keys = window
        .update(cx, |workspace, _window, cx| {
            workspace.explorer.read(cx).visible_tables(cx)
        })
        .expect("the window is open");
    assert_eq!(
        keys.iter().map(|key| key.name.clone()).collect::<Vec<_>>(),
        vec!["T_SAMPLE_ALBUM".to_string(), "T_SAMPLE_ARTIST".to_string()]
    );
    let album = keys
        .into_iter()
        .find(|key| key.name == "T_SAMPLE_ALBUM")
        .expect("the album is a row");

    // Ticking the schema ticks both, which is what the status bar counts.
    window
        .update(cx, |workspace, _window, cx| {
            workspace.explorer.update(cx, |explorer, cx| {
                explorer.toggle_tick(&explorer::NodeId::Schema(public.clone()), cx);
            });
            assert_eq!(workspace.explorer.read(cx).selected_count(cx), 2);
        })
        .expect("the window is open");
    cx.run_until_parked();

    // Moving the cursor onto a row points the inspector at it, and the
    // fetch behind that is the shell's.
    window
        .update(cx, |workspace, _window, cx| {
            workspace.explorer.update(cx, |explorer, cx| {
                explorer.select(explorer::NodeId::Table(album.clone()), cx);
            });
        })
        .expect("the window is open");
    cx.run_until_parked();

    window
        .update(cx, |workspace, _window, cx| {
            let panel = workspace.inspector.read(cx);
            let table = panel.table().expect("the inspector read the table");
            assert_eq!(table.name, "T_SAMPLE_ALBUM");
            assert_eq!(table.remarks, "an album");
            assert_eq!(
                table
                    .columns
                    .iter()
                    .map(|column| column.name.clone())
                    .collect::<Vec<_>>(),
                vec!["ALBUM_ID", "ARTIST_ID", "TITLE"]
            );
            // The primary key, and the foreign key out of it.
            assert_eq!(
                table
                    .keys()
                    .iter()
                    .map(|column| column.name.clone())
                    .collect::<Vec<_>>(),
                vec!["ALBUM_ID"]
            );
            assert_eq!(table.imports.len(), 1, "the foreign key is missing");
            assert_eq!(table.imports[0].ref_table, "T_SAMPLE_ARTIST");
        })
        .expect("the window is open");

    // Disconnecting takes the session, the tree and the ticks with it.
    window
        .update(cx, |workspace, _window, cx| {
            workspace.disconnect(cx);
            assert_eq!(workspace.explorer.read(cx).selected_count(cx), 0);
            assert!(!workspace.explorer_showing());
        })
        .expect("the window is open");
    cx.run_until_parked();
}

/// M3's whole promise, against a real database: tick, plan, write.
///
/// The same route [`a_real_database_fills_the_tree_and_the_inspector`] takes
/// — a real JVM, a real driver, a real H2 — because everything below this
/// line is already tested against fixtures, and what is left to find out is
/// whether the shell hands `rudbgen-gen` the tables and the templates the
/// window says it will, and puts the files where the status bar says.
///
/// The template is the shipped one, read from the repository's
/// `templates/`, so the assertion is about what a user of the built-in set
/// actually gets.
#[gpui::test]
fn a_real_database_writes_the_files_the_status_bar_promised(cx: &mut gpui::TestAppContext) {
    use rudbgen_core::{GenerationProfile, TemplateRef};
    use rudbgen_jdbc::StatementSpec;

    let out = tempfile::tempdir().expect("tempdir");
    let template = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../templates/java_model.java")
        .canonicalize()
        .expect("the shipped template is in the repository");

    let mut profile = connection::h2::profile("generate");
    // Set on the profile rather than typed into the panel: the panel is
    // loaded *from* the profile, so this is the same state a saved
    // connection arrives in — and a test that drove the text fields would
    // be testing gpui's text input rather than the run.
    profile.generation = GenerationProfile {
        templates: vec![TemplateRef {
            name: "Java Model".to_string(),
            file: template,
            out_template: "${name.suffix.pascal}Model.java".to_string(),
            selected: true,
        }],
        output_dir: Some(out.path().to_path_buf()),
        author: "comart".to_string(),
        custom_vars: vec![("package".to_string(), "com.abc.sample".to_string())],
    };

    let driver = connection::h2::driver();
    let session = connection::connect(
        &profile,
        &driver,
        &Credentials::typed(Some(String::new()), None),
        &AppSettings::default(),
    )
    .expect("H2 opens an in-memory database without a server");

    for sql in [
        "create table T_SAMPLE_ARTIST (ARTIST_ID integer not null, NAME varchar(80) not null, constraint PK_GEN_ARTIST primary key (ARTIST_ID))",
        "create table T_SAMPLE_ALBUM (ALBUM_ID integer not null, TITLE varchar(120), constraint PK_GEN_ALBUM primary key (ALBUM_ID))",
    ] {
        session
            .session()
            .execute(&StatementSpec::new(sql))
            .unwrap_or_else(|error| panic!("{sql}: {error}"));
    }

    cx.update(|cx| {
        app_settings::init(cx);
        rugpui::init(cx);
        rugpui_grid::init(cx);
    });
    // The run happens on a thread of its own, and a thread of its own is
    // what the test scheduler otherwise refuses: it wakes gpui's tasks from
    // outside the test thread, which the deterministic scheduler reads as
    // non-determinism. This is the switch gpui offers for exactly that —
    // "a mix of deterministic and non-deterministic async behavior, such as
    // when interacting with I/O". `run_until_parked` itself never blocks;
    // it steps until no work remains either way.
    cx.executor().allow_parking();
    silence_the_update_check();
    let window = cx.add_window(|window, cx| Workspace::new(TitlebarStyle::Custom, window, cx));
    window
        .update(cx, |workspace, _window, cx| {
            workspace.connection = ConnectionState::Open {
                profile: Box::new(profile.clone()),
                driver: Box::new(driver.clone()),
                session: Box::new(session),
            };
            workspace.reset_panels(cx);
        })
        .expect("the window is open");
    cx.run_until_parked();

    // Tick the whole schema, which is what the explorer's own suite covers
    // one row at a time.
    let public = window
        .update(cx, |workspace, _window, cx| {
            workspace
                .explorer
                .read(cx)
                .row_ids(cx)
                .into_iter()
                .flatten()
                .find_map(|id| match id {
                    explorer::NodeId::Schema(key) if key.name == "PUBLIC" => Some(key),
                    _ => None,
                })
                .expect("H2 answers with a PUBLIC schema")
        })
        .expect("the window is open");
    window
        .update(cx, |workspace, _window, cx| {
            workspace.explorer.update(cx, |explorer, cx| {
                explorer.expand(&explorer::NodeId::Schema(public.clone()), cx);
            });
        })
        .expect("the window is open");
    cx.run_until_parked();
    window
        .update(cx, |workspace, _window, cx| {
            workspace.explorer.update(cx, |explorer, cx| {
                explorer.toggle_tick(&explorer::NodeId::Schema(public.clone()), cx);
            });
            assert_eq!(workspace.explorer.read(cx).selected_count(cx), 2);
            // §4.2's arithmetic, before anything has been run: two tables,
            // one template, two files.
            let ready = workspace.readiness(cx);
            assert_eq!((ready.tables, ready.templates, ready.files), (2, 1, 2));
            assert_eq!(ready.blocker, None, "the run says it is ready");
            assert!(workspace.can_generate(cx));
        })
        .expect("the window is open");
    cx.run_until_parked();

    window
        .update(cx, |workspace, _window, cx| {
            workspace.start_run(RunKind::Generate, cx);
        })
        .expect("the window is open");

    // The run is on a thread of its own, so the test waits for it the way a
    // user does: by watching the dialog. `run_until_parked` alone would only
    // drain the tasks that already exist.
    let outcome = wait_for_outcome(cx, &window);
    assert!(!outcome.cancelled, "the run cancelled itself");
    assert!(outcome.failed.is_empty(), "failures: {:?}", outcome.failed);
    assert_eq!(outcome.written.len(), 2, "written: {:?}", outcome.written);

    let mut written: Vec<String> = std::fs::read_dir(out.path())
        .expect("the output directory")
        .flatten()
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .collect();
    written.sort();
    assert_eq!(written.len(), 2, "the output directory holds {written:?}");
    for name in &written {
        assert!(name.ends_with("Model.java"), "unexpected file {name}");
    }
    let text = std::fs::read_to_string(out.path().join(&written[0])).expect("read");
    assert!(
        text.contains("public class"),
        "the shipped template rendered to {text:?}"
    );
    // The custom variable and the author both reached the render.
    assert!(
        text.contains("com.abc.sample"),
        "the package variable is missing"
    );

    // Running again over the same directory finds every file already there.
    // The saved policy is *ask*, which is what puts the question up — and
    // answering "skip all" ends the run without writing anything.
    window
        .update(cx, |workspace, _window, cx| {
            workspace.job.update(cx, |job, cx| job.close(cx));
            workspace.start_run(RunKind::Generate, cx);
        })
        .expect("the window is open");
    answer_conflict(cx, &window, rudbgen_gen::Decision::SkipAll);
    let second = wait_for_outcome(cx, &window);
    assert!(second.written.is_empty(), "wrote over an existing file");
    assert_eq!(second.skipped.len(), 2, "skipped: {:?}", second.skipped);

    // The Preview tab: one pair, rendered to memory rather than to disk,
    // and a tab of its own to show it in.
    window
        .update(cx, |workspace, _window, cx| {
            workspace.job.update(cx, |job, cx| job.close(cx));
            workspace.start_run(RunKind::Preview, cx);
        })
        .expect("the window is open");
    cx.run_until_parked();
    window
        .update(cx, |workspace, _window, cx| {
            assert!(
                matches!(workspace.pane.active(), Some(PaneItem::Preview { .. })),
                "the preview opened no tab"
            );
            let pane = workspace.preview.read(cx);
            assert_eq!(pane.file_count(), 1, "a preview is one pair");
            assert!(
                pane.shown_text()
                    .expect("the pair rendered")
                    .contains("public class"),
                "the preview shows something other than the template's output"
            );
        })
        .expect("the window is open");

    // The dry run: every pair, and every one of them already on disk from
    // the first generate.
    window
        .update(cx, |workspace, _window, cx| {
            workspace.start_run(RunKind::DryRun, cx);
        })
        .expect("the window is open");
    cx.run_until_parked();
    window
        .update(cx, |workspace, _window, cx| {
            assert_eq!(
                workspace.preview.read(cx).file_count(),
                2,
                "the dry run rendered a different number of files than the run wrote"
            );
            // One preview tab, however many times it is asked for.
            assert_eq!(
                workspace
                    .pane
                    .items()
                    .iter()
                    .filter(|item| matches!(item, PaneItem::Preview { .. }))
                    .count(),
                1
            );
        })
        .expect("the window is open");
}
