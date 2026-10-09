use super::*;

/// M4's other half, against a real database: the live preview renders the
/// buffer — not the file — and an unknown field comes back as a warning on
/// the line that names it.
///
/// The typo is the case D12 exists for: `${nmae}` parses, renders to
/// nothing, and in jdbgen is found by reading the generated file.
#[gpui::test]
fn a_real_database_marks_an_unknown_field_in_the_live_preview(cx: &mut gpui::TestAppContext) {
    use rudbgen_jdbc::StatementSpec;

    let dir = tempfile::tempdir().expect("tempdir");
    let file = dir.path().join("model.java");
    std::fs::write(&file, "class ${name} {}\n").expect("the template is written");

    let profile = connection::h2::profile("preview");
    let driver = connection::h2::driver();
    let session = connection::connect(
        &profile,
        &driver,
        &Credentials::typed(Some(String::new()), None),
        &AppSettings::default(),
    )
    .expect("H2 opens an in-memory database without a server");
    session
            .session()
            .execute(&StatementSpec::new(
                "create table T_SAMPLE_ALBUM (ALBUM_ID integer not null, TITLE varchar(120), constraint PK_PREVIEW primary key (ALBUM_ID))",
            ))
            .expect("the table is created");

    cx.update(|cx| {
        app_settings::init(cx);
        rugpui::init(cx);
        rugpui_grid::init(cx);
        rugpui_editor::init(cx);
    });
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

    // Tick the schema, so the preview has a table to render against.
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
        })
        .expect("the window is open");
    cx.run_until_parked();

    window
        .update(cx, |workspace, _window, cx| {
            workspace.open_template(file.clone(), cx);
        })
        .expect("the window is open");
    cx.run_until_parked();

    // The template as written renders, and the preview is of the table.
    window
        .update(cx, |workspace, _window, cx| {
            let pane = workspace.templates[0].read(cx);
            assert!(pane.diagnostics().is_empty(), "{:?}", pane.diagnostics());
            assert!(
                pane.preview_text(cx).contains("T_SAMPLE_ALBUM"),
                "the preview is {:?}",
                pane.preview_text(cx)
            );
            // The palette knows what the table is called, which is what
            // makes an example an example.
            assert!(
                workspace.palette_table(cx).is_some(),
                "the palette has no model"
            );
        })
        .expect("the window is open");

    // Now the typo, in the buffer alone: the file on disk still says
    // `${name}`, and the preview follows the buffer.
    window
        .update(cx, |workspace, _window, cx| {
            let pane = workspace.templates[0].clone();
            pane.update(cx, |pane, cx| {
                pane.set_source("class ${nmae} {}\n", cx);
                pane.refresh(cx);
            });
        })
        .expect("the window is open");
    cx.run_until_parked();

    window
        .update(cx, |workspace, _window, cx| {
            let pane = workspace.templates[0].read(cx);
            let found = pane.diagnostics();
            assert_eq!(found.len(), 1, "{found:?}");
            assert!(
                !found[0].error,
                "an unknown field is a warning, not an error"
            );
            assert_eq!(found[0].line, 0);
            assert!(
                found[0].message.contains("nmae"),
                "the warning does not name the field: {:?}",
                found[0].message
            );
            assert!(
                !pane.preview_text(cx).contains("T_SAMPLE_ALBUM"),
                "the preview did not follow the buffer"
            );
            assert_eq!(
                std::fs::read_to_string(&file).expect("the file is still there"),
                "class ${name} {}\n",
                "the buffer was written to disk without a save"
            );
        })
        .expect("the window is open");

    window
        .update(cx, |workspace, _window, _cx| workspace.close_session())
        .expect("the window is open");
}

/// The palette's examples follow the preview's table even when that table
/// came out of the inspector's cache.
///
/// A table the user has already looked at is described once and kept, so a
/// preview of it is rendered from the cache and never reads anything. The
/// palette takes its examples from the very same table, and rebuilding it
/// only on the path that reads afresh left the examples on whichever table
/// happened to be in hand before — the rows would sit there with no `→`
/// beside them, or with the wrong one, until something else refreshed them.
#[gpui::test]
fn the_palette_follows_a_preview_table_taken_from_the_cache(cx: &mut gpui::TestAppContext) {
    use rudbgen_jdbc::StatementSpec;

    let dir = tempfile::tempdir().expect("tempdir");
    let file = dir.path().join("model.java");
    std::fs::write(&file, "class ${name} {}\n").expect("the template is written");

    let profile = connection::h2::profile("palette");
    let driver = connection::h2::driver();
    let session = connection::connect(
        &profile,
        &driver,
        &Credentials::typed(Some(String::new()), None),
        &AppSettings::default(),
    )
    .expect("H2 opens an in-memory database without a server");
    // Two tables, because the second half of this is the dropdown moving
    // from one to the other.
    for statement in [
        "create table T_SAMPLE_ALBUM (ALBUM_ID integer not null, TITLE varchar(120), constraint PK_ALBUM primary key (ALBUM_ID))",
        "create table T_SAMPLE_ARTIST (ARTIST_ID integer not null, ARTIST_NAME varchar(120), constraint PK_ARTIST primary key (ARTIST_ID))",
    ] {
        session
            .session()
            .execute(&StatementSpec::new(statement))
            .expect("the table is created");
    }

    cx.update(|cx| {
        app_settings::init(cx);
        rugpui::init(cx);
        rugpui_grid::init(cx);
        rugpui_editor::init(cx);
    });
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

    // Tick the schema, so both tables are on offer to the preview.
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
        })
        .expect("the window is open");
    cx.run_until_parked();

    // Both tables are described before the template is opened, which is
    // what a walk through the tree does: from here on nothing has to be
    // read, and every preview is a cache hit.
    let choices = window
        .update(cx, |workspace, _window, cx| {
            let (handle, driver, _) = workspace.meta_context().expect("the session is open");
            let keys = workspace.preview_tables(cx);
            for (key, _) in &keys {
                let reference = workspace
                    .explorer
                    .read(cx)
                    .table_ref(key, cx)
                    .expect("the ticked table is in the tree");
                let table = MetaReader::new(handle.session(), &driver)
                    .table(&reference)
                    .expect("H2 describes the table");
                workspace
                    .inspector
                    .update(cx, |panel, _cx| panel.remember(key.clone(), table));
            }
            keys
        })
        .expect("the window is open");
    assert_eq!(choices.len(), 2, "the two tables are what is on offer");

    window
        .update(cx, |workspace, _window, cx| {
            workspace.open_template(file.clone(), cx);
        })
        .expect("the window is open");
    cx.run_until_parked();

    // The tab renders against the first of the two, and so does the
    // palette: `name` is the table's own name, and the example says so.
    window
        .update(cx, |workspace, _window, cx| {
            let first = choices[0].0.name.clone();
            assert!(
                workspace.templates[0]
                    .read(cx)
                    .preview_text(cx)
                    .contains(&first),
                "the preview is of another table"
            );
            assert_eq!(
                palette_example(workspace, cx),
                Some(SharedString::from(first.clone())),
                "the palette's examples are not of {first}"
            );
        })
        .expect("the window is open");

    // The dropdown moves to the other table. It is cached too, so nothing
    // is read — and the palette still has to follow.
    window
        .update(cx, |workspace, _window, cx| {
            workspace.templates[0]
                .clone()
                .update(cx, |pane, cx| pane.choose(1, cx));
        })
        .expect("the window is open");
    cx.run_until_parked();

    window
        .update(cx, |workspace, _window, cx| {
            let second = choices[1].0.name.clone();
            assert!(
                workspace.templates[0]
                    .read(cx)
                    .preview_text(cx)
                    .contains(&second),
                "the preview did not follow the dropdown"
            );
            assert_eq!(
                palette_example(workspace, cx),
                Some(SharedString::from(second.clone())),
                "the palette did not follow the dropdown to {second}"
            );
        })
        .expect("the window is open");

    window
        .update(cx, |workspace, _window, _cx| workspace.close_session())
        .expect("the window is open");
}

/// M5's promise, against a real database: a rule typed into the editor is
/// the rule the preview then applies.
///
/// The whole route the user takes — *Rules…* on the Generate tab opens the
/// dialog over the panel's own store, the trailing empty row takes the
/// rule, `Save` hands the store back, and `${name.abbr}` in a template tab
/// renders through it. The one step left out is the write to
/// `abbreviations.json`: the configuration directory is the user's own on
/// every platform, and a test that wrote into it would be editing the
/// machine it runs on. `AbbreviationStore::save_to` is proved in
/// `rudbgen-core` against a temporary directory instead.
#[gpui::test]
fn a_rule_typed_into_the_editor_is_the_rule_the_preview_applies(cx: &mut gpui::TestAppContext) {
    use rudbgen_core::AbbreviationRule;
    use rudbgen_jdbc::StatementSpec;

    let dir = tempfile::tempdir().expect("tempdir");
    let file = dir.path().join("model.java");
    std::fs::write(&file, "class ${name.abbr} {}\n").expect("the template is written");

    let profile = connection::h2::profile("abbr");
    let driver = connection::h2::driver();
    let session = connection::connect(
        &profile,
        &driver,
        &Credentials::typed(Some(String::new()), None),
        &AppSettings::default(),
    )
    .expect("H2 opens an in-memory database without a server");
    session
            .session()
            .execute(&StatementSpec::new(
                "create table T_SAMPLE_ALBUM (ALBUM_ID integer not null, TITLE varchar(120), constraint PK_ABBR primary key (ALBUM_ID))",
            ))
            .expect("the table is created");

    cx.update(|cx| {
        app_settings::init(cx);
        rugpui::init(cx);
        rugpui_grid::init(cx);
        rugpui_editor::init(cx);
    });
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
        })
        .expect("the window is open");
    cx.run_until_parked();

    // The template tab first, so that what the rule changes is visible as a
    // change: with no rule, `${name.abbr}` is the name itself.
    window
        .update(cx, |workspace, _window, cx| {
            workspace.open_template(file.clone(), cx);
        })
        .expect("the window is open");
    cx.run_until_parked();
    window
        .update(cx, |workspace, _window, cx| {
            assert!(
                workspace.templates[0]
                    .read(cx)
                    .preview_text(cx)
                    .contains("T_SAMPLE_ALBUM"),
                "an empty dictionary changed the name"
            );
        })
        .expect("the window is open");

    // *Rules…*, which is what the Generate tab's button dispatches.
    let store = window
        .update(cx, |workspace, window, cx| {
            workspace.open_abbreviations(window, cx);
            workspace.abbreviations.update(cx, |dialog, cx| {
                assert!(dialog.is_open());
                // The picker offers what the explorer has loaded, which is
                // the whole point of it being there.
                assert!(
                    dialog.tables().iter().any(|name| name == "T_SAMPLE_ALBUM"),
                    "the picker was not given the loaded tables: {:?}",
                    dialog.tables()
                );
                // One blank row to type into, and nothing else.
                assert_eq!(dialog.draft(cx).len(), 1);

                dialog.write_row(
                    0,
                    &AbbreviationRule {
                        enabled: true,
                        whole_name: false,
                        abbreviation: "album".to_string(),
                        replacement: "Disc".to_string(),
                    },
                    cx,
                );
                // The row was filled in, so a new blank one appeared under
                // it — and the blank one is not saved.
                assert_eq!(dialog.draft(cx).len(), 2);
                assert!(abbreviation_dialog::duplicates(&dialog.draft(cx)).is_empty());
                let store = dialog.drafted_store(cx);
                assert_eq!(store.rules.len(), 1);
                store
            })
        })
        .expect("the window is open");

    // What `AbbreviationDialogEvent::Saved` does with it.
    window
        .update(cx, |workspace, _window, cx| {
            workspace.generate.update(cx, |pane, cx| {
                pane.adopt_abbreviations(store, cx);
            });
            workspace.templates[0].update(cx, |pane, cx| pane.refresh(cx));
        })
        .expect("the window is open");
    cx.run_until_parked();

    window
        .update(cx, |workspace, _window, cx| {
            let text = workspace.templates[0].read(cx).preview_text(cx);
            // D10: the rule is spelled `album` and the segment is `ALBUM`.
            // jdbgen would have matched neither.
            assert!(
                text.contains("T_SAMPLE_Disc"),
                "the rule did not reach the preview: {text:?}"
            );
        })
        .expect("the window is open");

    window
        .update(cx, |workspace, _window, _cx| workspace.close_session())
        .expect("the window is open");
}

/// Every command the shell offers reaches the workspace, and `Escape`
/// closes what the command opened.
/// M4's whole promise, without a database: open a template, edit it, see
/// what is wrong with it, and write it back.
///
/// No connection at all, deliberately — §4.3 says a template may be edited
/// without one, and everything here but the live preview works that way.
/// The CRLF the shipped templates are written with is what the file is
/// made of, so the round trip through the buffer is under test too.
#[gpui::test]
fn a_template_opens_edits_diagnoses_and_saves(cx: &mut gpui::TestAppContext) {
    cx.update(|cx| {
        app_settings::init(cx);
        rugpui::init(cx);
        rugpui_editor::init(cx);
    });

    let dir = tempfile::tempdir().expect("a temporary directory");
    let file = dir.path().join("java_model.java");
    std::fs::write(&file, "public class ${name.pascal} {\r\n}\r\n").expect("the file");

    silence_the_update_check();
    let window = cx.add_window(|window, cx| Workspace::new(TitlebarStyle::Custom, window, cx));
    window
        .update(cx, |workspace, _window, cx| {
            workspace.open_template(file.clone(), cx);
        })
        .expect("the window is open");
    cx.run_until_parked();

    // One tab, one buffer, and the buffer is `\n` only whatever the file is.
    window
        .update(cx, |workspace, _window, cx| {
            assert_eq!(workspace.templates.len(), 1);
            assert!(matches!(
                workspace.pane.active(),
                Some(PaneItem::Template { .. })
            ));
            let pane = workspace.templates[0].read(cx);
            assert!(!pane.source(cx).is_empty());
            assert!(pane.source(cx).contains("\r\n"), "the ending was forgotten");
            assert!(!pane.is_dirty(cx), "a freshly opened file is not dirty");
            assert!(pane.preview_open(), "the preview half starts open");
            // The palette took the place of the inspector, and it is full
            // even with no connection: only the examples need a table.
            assert!(
                workspace.palette.read(cx).len() > 90,
                "the palette is empty"
            );
            // Opening the same file again is a navigation, not a second
            // buffer over one path.
            workspace.open_template(file.clone(), cx);
            assert_eq!(workspace.templates.len(), 1);
            // And the preview half puts itself away when asked.
            let pane = workspace.templates[0].clone();
            pane.update(cx, |pane, cx| {
                pane.toggle_preview(cx);
                assert!(!pane.preview_open());
                pane.toggle_preview(cx);
            });
        })
        .expect("the window is open");

    // A typo in a field name: the template still parses, so the verdict has
    // to come from a render — which needs no connection to *fail*, and the
    // parse below is what the tab computes locally.
    window
        .update(cx, |workspace, window, cx| {
            let pane = workspace.templates[0].clone();
            pane.update(cx, |pane, cx| {
                pane.insert("${for:item=columns}", window, cx);
            });
        })
        .expect("the window is open");
    cx.executor()
        .advance_clock(std::time::Duration::from_millis(500));
    cx.run_until_parked();

    // A `for` with no `endfor` does not parse, and the tab says where.
    window
        .update(cx, |workspace, _window, cx| {
            let pane = workspace.templates[0].read(cx);
            assert!(pane.is_dirty(cx), "the edit did not mark the tab");
            let found = pane.diagnostics();
            assert_eq!(found.len(), 1, "{found:?}");
            assert!(found[0].error, "{found:?}");
            // And the tab strip is wearing the marker.
            assert!(matches!(
                workspace.pane.active(),
                Some(PaneItem::Template { dirty: true, .. })
            ));
        })
        .expect("the window is open");

    // Closing it now asks rather than throwing the edit away.
    window
        .update(cx, |workspace, window, cx| {
            let index = workspace.pane.active_index();
            workspace.request_close_tab(index, window, cx);
            assert_eq!(workspace.closing, Some(index));
            assert_eq!(workspace.templates.len(), 1, "it closed without asking");
        })
        .expect("the window is open");

    // Cancel leaves everything where it was; Save writes and closes.
    window
        .update(cx, |workspace, window, cx| {
            workspace.closing = None;
            let pane = workspace.templates[0].clone();
            pane.update(cx, |pane, cx| {
                // Undo the unterminated loop, so the file that is written
                // is one that parses.
                let source = pane.source(cx);
                let fixed = source.replace("${for:item=columns}", "");
                pane.set_source(&fixed, cx);
                assert!(pane.save(cx), "the file did not write");
            });
            let index = workspace.pane.active_index();
            workspace.request_close_tab(index, window, cx);
            assert_eq!(workspace.closing, None, "a saved tab still asked");
            assert!(workspace.templates.is_empty(), "the tab stayed open");
        })
        .expect("the window is open");

    let written = std::fs::read_to_string(&file).expect("the file is still there");
    assert!(
        written.contains("\r\n"),
        "the line ending was not restored: {written:?}"
    );
    assert!(!written.contains("\n\n"), "a bare newline was written");
}

/// The completion popup offers what the caret's context allows, and
/// accepting one writes it over what was typed.
#[gpui::test]
fn the_completion_popup_offers_and_accepts(cx: &mut gpui::TestAppContext) {
    cx.update(|cx| {
        app_settings::init(cx);
        rugpui::init(cx);
        rugpui_editor::init(cx);
    });

    let dir = tempfile::tempdir().expect("a temporary directory");
    let file = dir.path().join("model.java");
    std::fs::write(&file, "class X {\n}\n").expect("the file");

    silence_the_update_check();
    let window = cx.add_window(|window, cx| Workspace::new(TitlebarStyle::Custom, window, cx));
    window
        .update(cx, |workspace, _window, cx| {
            workspace.open_template(file.clone(), cx);
        })
        .expect("the window is open");
    cx.run_until_parked();

    window
        .update(cx, |workspace, window, cx| {
            let pane = workspace.templates[0].clone();
            pane.update(cx, |pane, cx| pane.insert("${na", window, cx));
        })
        .expect("the window is open");
    // The buffer's `Changed` reaches the tab on the effect cycle, and the
    // popup follows it.
    cx.run_until_parked();

    window
        .update(cx, |workspace, _window, cx| {
            let pane = workspace.templates[0].clone();
            pane.update(cx, |pane, cx| {
                assert!(pane.completion_open(), "typing a brace offered nothing");
                let names = pane.completion_names();
                assert_eq!(names.first().map(|name| name.as_ref()), Some("name"));
                // Accepting writes the rest of the word over the prefix,
                // and takes the popup down.
                pane.accept_completion(0, cx);
                assert!(!pane.completion_open());
                assert!(pane.source(cx).contains("${name"), "{}", pane.source(cx));
            });
        })
        .expect("the window is open");
}
