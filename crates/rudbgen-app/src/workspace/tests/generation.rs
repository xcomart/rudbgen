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
    assert!(!outcome.canceled, "the run canceled itself");
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

/// A window connected to a fresh H2 that `statements` were run against, with
/// the shipped Java model as the one template and `out` as the output directory, and every
/// table of PUBLIC ticked.
///
/// Nothing has been inspected, so the inspector's cache is empty and a run
/// starts by reading its tables — the phase the tests below are about.
fn ticked_h2_window(
    cx: &mut gpui::TestAppContext,
    name: &str,
    statements: &[&str],
    out: &std::path::Path,
) -> gpui::WindowHandle<Workspace> {
    use rudbgen_core::{GenerationProfile, TemplateRef};
    use rudbgen_jdbc::StatementSpec;

    let template = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../templates/java_model.java")
        .canonicalize()
        .expect("the shipped template is in the repository");
    let mut profile = connection::h2::profile(name);
    profile.generation = GenerationProfile {
        templates: vec![TemplateRef {
            name: "Java Model".to_string(),
            file: template,
            out_template: "${name.suffix.pascal}Model.java".to_string(),
            selected: true,
        }],
        output_dir: Some(out.to_path_buf()),
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
    for sql in statements {
        session
            .session()
            .execute(&StatementSpec::new(*sql))
            .unwrap_or_else(|error| panic!("{sql}: {error}"));
    }

    cx.update(|cx| {
        app_settings::init(cx);
        rugpui::init(cx);
        rugpui_grid::init(cx);
        rugpui_editor::init(cx);
    });
    // The generate runs on a thread of its own; see
    // `a_real_database_writes_the_files_the_status_bar_promised`.
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
            let tables = statements
                .iter()
                .filter(|sql| sql.starts_with("create table"))
                .count();
            assert_eq!(workspace.explorer.read(cx).selected_count(cx), tables);
        })
        .expect("the window is open");
    cx.run_until_parked();
    window
}

/// The two tables the tests below generate from: an album with a comment —
/// which the reader copies off the explorer's row rather than reading — and an
/// artist without one.
const ALBUM_AND_ARTIST: [&str; 3] = [
    "create table T_SAMPLE_ARTIST (ARTIST_ID integer not null, NAME varchar(80) not null, constraint PK_ARTIST primary key (ARTIST_ID))",
    "create table T_SAMPLE_ALBUM (ALBUM_ID integer not null, TITLE varchar(120), constraint PK_ALBUM primary key (ALBUM_ID))",
    "comment on table T_SAMPLE_ALBUM is 'an album'",
];

/// The explorer row for `name`, which has to be on screen.
fn table_key(
    cx: &mut gpui::TestAppContext,
    window: &gpui::WindowHandle<Workspace>,
    name: &str,
) -> TableKey {
    window
        .update(cx, |workspace, _window, cx| {
            workspace
                .explorer
                .read(cx)
                .visible_tables(cx)
                .into_iter()
                .find(|key| key.name == name)
                .unwrap_or_else(|| panic!("{name} is not a row"))
        })
        .expect("the window is open")
}

/// The names of `table`'s columns, in order.
fn column_names(table: &Table) -> Vec<String> {
    table
        .columns
        .iter()
        .map(|column| column.name.clone())
        .collect()
}

/// The table called `name` among the ones the last run was given.
fn run_table(workspace: &Workspace, name: &str) -> Table {
    workspace
        .run_tables
        .iter()
        .find(|table| table.name == name)
        .cloned()
        .unwrap_or_else(|| panic!("the run had no {name}"))
}

/// Cancel during the table-reading phase stops the run before it starts.
///
/// The phase has no cancel token of its own — the reads are the shell's — so
/// this is the case that used to fall through: a canceled generate still
/// wrote its files once the reads came back, and a canceled dry run left a
/// progress dialog up that nothing could take down.
#[gpui::test]
fn cancel_while_the_tables_are_read_starts_nothing(cx: &mut gpui::TestAppContext) {
    let out = tempfile::tempdir().expect("tempdir");
    let window = ticked_h2_window(cx, "cancel-load", &ALBUM_AND_ARTIST, out.path());

    for kind in [RunKind::Generate, RunKind::DryRun, RunKind::Preview] {
        window
            .update(cx, |workspace, _window, cx| {
                workspace.start_run(kind, cx);
                assert!(
                    workspace.job.read(cx).is_busy(),
                    "{kind:?} did not start by reading its tables"
                );
                workspace.job.update(cx, |job, cx| job.cancel(cx));
            })
            .expect("the window is open");
        cx.run_until_parked();
        window
            .update(cx, |workspace, _window, cx| {
                assert!(
                    !workspace.job.read(cx).is_open(),
                    "canceling {kind:?} left the dialog up"
                );
                assert!(
                    workspace.run_tables.is_empty(),
                    "canceling {kind:?} still handed its tables to a run"
                );
                assert!(
                    workspace.preview_tab().is_none(),
                    "canceling {kind:?} still opened the preview"
                );
            })
            .expect("the window is open");
        assert_eq!(
            std::fs::read_dir(out.path())
                .expect("the output directory")
                .count(),
            0,
            "canceling {kind:?} wrote files"
        );
    }

    // Canceled and asked again before the first phase's reads have run: the
    // second phase is the only one that goes on, so exactly one run writes.
    window
        .update(cx, |workspace, _window, cx| {
            workspace.start_run(RunKind::Generate, cx);
            workspace.job.update(cx, |job, cx| job.cancel(cx));
            workspace.start_run(RunKind::Generate, cx);
            // A second press while the first is still reading is ignored
            // rather than started over it.
            workspace.start_run(RunKind::DryRun, cx);
        })
        .expect("the window is open");
    let outcome = wait_for_outcome(cx, &window);
    assert!(!outcome.canceled, "the second run canceled itself");
    assert_eq!(outcome.written.len(), 2, "written: {:?}", outcome.written);
    assert!(outcome.skipped.is_empty(), "a second run raced the first");
    window
        .update(cx, |workspace, _window, _cx| {
            assert!(
                workspace.preview_tab().is_none(),
                "the dry run pressed during the generate ran as well"
            );
        })
        .expect("the window is open");
}

/// What interrupts a reading phase once its first table is in.
#[derive(Clone, Copy, Debug)]
enum Interrupt {
    /// The dialog's Cancel.
    Cancel,
    /// The session going away.
    Disconnect,
}

/// Starts a generate that has to read `missing` tables and interrupts it as
/// soon as the first of them has been described — the moment a real Cancel
/// lands, since a read is what takes the time.
fn interrupt_after_the_first_read(
    cx: &mut gpui::TestAppContext,
    name: &str,
    cache_the_album: bool,
    interrupt: Interrupt,
) {
    let out = tempfile::tempdir().expect("tempdir");
    let window = ticked_h2_window(cx, name, &ALBUM_AND_ARTIST, out.path());
    if cache_the_album {
        let album = table_key(cx, &window, "T_SAMPLE_ALBUM");
        window
            .update(cx, |workspace, _window, cx| {
                workspace.explorer.update(cx, |explorer, cx| {
                    explorer.select(explorer::NodeId::Table(album.clone()), cx);
                });
            })
            .expect("the window is open");
        cx.run_until_parked();
    }

    let job = window
        .update(cx, |workspace, _window, _cx| workspace.job.clone())
        .expect("the window is open");
    let fired = std::rc::Rc::new(std::cell::Cell::new(false));
    cx.update(|cx| {
        let fired = fired.clone();
        cx.observe(&job, move |job, cx| {
            if fired.get() || job.read(cx).tables_read() != Some(1) {
                return;
            }
            fired.set(true);
            match interrupt {
                Interrupt::Cancel => job.update(cx, |job, cx| job.cancel(cx)),
                Interrupt::Disconnect => {
                    window
                        .update(cx, |workspace, _window, cx| workspace.disconnect(cx))
                        .expect("the window is open");
                }
            }
        })
        .detach();
    });

    window
        .update(cx, |workspace, _window, cx| {
            workspace.start_run(RunKind::Generate, cx);
        })
        .expect("the window is open");
    cx.run_until_parked();

    assert!(fired.get(), "{interrupt:?} never came: no table was read");
    window
        .update(cx, |workspace, _window, cx| {
            assert!(
                !workspace.job.read(cx).is_open(),
                "the dialog stayed up after {interrupt:?}"
            );
            assert!(
                workspace.run_tables.is_empty(),
                "the tables still went to a run after {interrupt:?}"
            );
        })
        .expect("the window is open");
    assert_eq!(
        std::fs::read_dir(out.path())
            .expect("the output directory")
            .count(),
        0,
        "files were written after {interrupt:?}"
    );
}

/// Cancel pressed while the last missing table is being read: no read is
/// left to skip, and what stops the run is the check before it is started.
#[gpui::test]
fn cancel_during_the_last_read_starts_nothing(cx: &mut gpui::TestAppContext) {
    interrupt_after_the_first_read(cx, "cancel-last", true, Interrupt::Cancel);
}

/// The session going away half way through the reads takes the dialog down
/// and starts nothing: the answers describe a database the window has left.
#[gpui::test]
fn a_disconnect_during_the_reads_starts_nothing(cx: &mut gpui::TestAppContext) {
    interrupt_after_the_first_read(cx, "disconnect-load", false, Interrupt::Disconnect);
}

/// The explorer's Refresh makes the next run read its tables again.
///
/// The inspector caches every table it has described, and a run takes its
/// tables from that cache. Before the explorer's Refresh reached it, a column
/// added after a first read was missing from every later run of the session.
/// The second half is what a refresh must keep: the comment, which the reader
/// copies off the explorer's row, and which an emptied row index turned into
/// a blank on every table read straight after the refresh.
#[gpui::test]
fn refresh_reads_the_tables_again(cx: &mut gpui::TestAppContext) {
    use rudbgen_jdbc::StatementSpec;

    let out = tempfile::tempdir().expect("tempdir");
    let window = ticked_h2_window(cx, "refresh", &ALBUM_AND_ARTIST, out.path());
    let album = table_key(cx, &window, "T_SAMPLE_ALBUM");

    // A dry run reads both tables and leaves them in the inspector's cache;
    // the cursor on the album puts it on screen as well.
    window
        .update(cx, |workspace, _window, cx| {
            workspace.start_run(RunKind::DryRun, cx)
        })
        .expect("the window is open");
    cx.run_until_parked();
    window
        .update(cx, |workspace, _window, cx| {
            assert_eq!(
                column_names(&run_table(workspace, "T_SAMPLE_ALBUM")),
                vec!["ALBUM_ID", "TITLE"]
            );
            workspace.explorer.update(cx, |explorer, cx| {
                explorer.select(explorer::NodeId::Table(album.clone()), cx);
            });
        })
        .expect("the window is open");
    cx.run_until_parked();

    // A column on each: the album is on screen, the artist only cached.
    window
        .update(cx, |workspace, _window, cx| {
            let session = workspace
                .connection
                .session()
                .expect("the session is open")
                .session();
            for sql in [
                "alter table T_SAMPLE_ALBUM add column RELEASED integer",
                "alter table T_SAMPLE_ARTIST add column BORN integer",
            ] {
                session
                    .execute(&StatementSpec::new(sql))
                    .unwrap_or_else(|error| panic!("{sql}: {error}"));
            }
            workspace
                .explorer
                .update(cx, |explorer, cx| explorer.refresh(cx));
        })
        .expect("the window is open");
    cx.run_until_parked();

    window
        .update(cx, |workspace, _window, cx| {
            let panel = workspace.inspector.read(cx);
            let shown = panel.table().expect("the inspector read the album again");
            assert_eq!(
                column_names(shown),
                vec!["ALBUM_ID", "TITLE", "RELEASED"],
                "the table on screen still shows the old columns"
            );
            assert_eq!(shown.remarks, "an album", "the refresh lost the comment");
            workspace.start_run(RunKind::DryRun, cx);
        })
        .expect("the window is open");
    cx.run_until_parked();
    window
        .update(cx, |workspace, _window, _cx| {
            let album = run_table(workspace, "T_SAMPLE_ALBUM");
            assert_eq!(
                column_names(&album),
                vec!["ALBUM_ID", "TITLE", "RELEASED"],
                "the run after a refresh used the cached columns"
            );
            assert_eq!(album.remarks, "an album", "the run lost the comment");
            assert_eq!(
                column_names(&run_table(workspace, "T_SAMPLE_ARTIST")),
                vec!["ARTIST_ID", "NAME", "BORN"],
                "a table that was only cached kept its old columns"
            );
        })
        .expect("the window is open");
}

/// With nothing ticked, a template's live preview is of the table on screen,
/// and a refresh must neither leave it without one nor render it from the
/// stale description.
#[gpui::test]
fn refresh_keeps_the_live_preview_on_the_table_on_screen(cx: &mut gpui::TestAppContext) {
    use rudbgen_jdbc::StatementSpec;

    let out = tempfile::tempdir().expect("tempdir");
    let template = out.path().join("columns.txt");
    std::fs::write(
        &template,
        "${name} ${remarks}: ${for:item=columns, inStr=\",\"}${name}${endfor}\n",
    )
    .expect("the template is written");
    let window = ticked_h2_window(cx, "refresh-preview", &ALBUM_AND_ARTIST, out.path());
    let album = table_key(cx, &window, "T_SAMPLE_ALBUM");
    let artist = table_key(cx, &window, "T_SAMPLE_ARTIST");
    window
        .update(cx, |workspace, _window, cx| {
            workspace.explorer.update(cx, |explorer, cx| {
                explorer.toggle_tick(&explorer::NodeId::Table(album.clone()), cx);
                explorer.toggle_tick(&explorer::NodeId::Table(artist.clone()), cx);
                explorer.select(explorer::NodeId::Table(album.clone()), cx);
            });
            assert_eq!(workspace.explorer.read(cx).selected_count(cx), 0);
        })
        .expect("the window is open");
    cx.run_until_parked();
    // Opened once the inspector has the album: the tab takes the table on
    // screen as its choice when it opens.
    window
        .update(cx, |workspace, _window, cx| {
            workspace.open_template(template.clone(), cx);
        })
        .expect("the window is open");
    cx.run_until_parked();
    let preview = |cx: &mut gpui::TestAppContext| -> String {
        window
            .update(cx, |workspace, _window, cx| {
                workspace.templates[0].read(cx).preview_text(cx).to_string()
            })
            .expect("the window is open")
    };
    let before = preview(cx);
    assert!(
        before.contains("T_SAMPLE_ALBUM an album: ALBUM_ID,TITLE"),
        "the preview is {before:?}"
    );

    window
        .update(cx, |workspace, _window, cx| {
            workspace
                .connection
                .session()
                .expect("the session is open")
                .session()
                .execute(&StatementSpec::new(
                    "alter table T_SAMPLE_ALBUM add column RELEASED integer",
                ))
                .expect("H2 adds the column");
            workspace
                .explorer
                .update(cx, |explorer, cx| explorer.refresh(cx));
        })
        .expect("the window is open");
    cx.run_until_parked();

    let after = preview(cx);
    assert!(
        after.contains("T_SAMPLE_ALBUM an album: ALBUM_ID,TITLE,RELEASED"),
        "the refresh left the preview without its table, or on the old columns: {after:?}"
    );
}
