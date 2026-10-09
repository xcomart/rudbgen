use super::*;

#[test]
fn a_pinned_editor_theme_is_left_alone() {
    // The switch is off, so nothing about the chrome may reach the editor —
    // not even a cast that clashes with it.
    assert_eq!(
        editor_theme_for("tokyo-night", false, "one-light", false, &entries()),
        "tokyo-night"
    );
}

#[test]
fn following_the_ui_prefers_the_chrome_themes_namesake() {
    // The rule the switch is named for: the editor theme sharing the chrome
    // theme's id wins, whatever the settings file still carries. A dark
    // editor under a light window is dropped for the light namesake…
    assert_eq!(
        editor_theme_for("tokyo-night", true, "one-light", false, &entries()),
        "one-light"
    );
    // …and so is a dark editor under a *different* dark window: the cast
    // already matched, so the configured id would otherwise win and the
    // editor would never move off One Dark however far the chrome went.
    assert_eq!(
        editor_theme_for("one-dark", true, "dracula", true, &entries()),
        "dracula"
    );
    assert_eq!(
        editor_theme_for("dracula", true, "gruvbox-dark", true, &entries()),
        "gruvbox-dark"
    );
}

#[test]
fn following_the_ui_keeps_the_configured_pick_when_the_chrome_has_no_namesake() {
    // A chrome theme of the user's own, which no editor theme is named
    // after: there is no pair to honour, so the configured editor theme
    // stands as long as its cast fits the window.
    assert_eq!(
        editor_theme_for("tokyo-night", true, "my-chrome", true, &entries()),
        "tokyo-night"
    );
}

#[test]
fn following_the_ui_refuses_a_namesake_of_the_wrong_cast() {
    // The one thing that outranks the name match. A user has written a dark
    // editor theme under the id of a light chrome theme; pairing them would
    // put a dark editor in a light window, which is the accident this switch
    // exists to prevent, so the namesake is passed over.
    let mut entries = entries();
    entries.push(EditorThemeEntry {
        id: "my-chrome".to_string(),
        name: "Mine".to_string(),
        dark: true,
        builtin: false,
    });
    assert_eq!(
        editor_theme_for("solarized-light", true, "my-chrome", false, &entries),
        "solarized-light"
    );
}

#[test]
fn following_the_ui_falls_back_to_any_theme_of_the_right_cast() {
    // A chrome theme with no editor theme of its name — a palette the user
    // wrote themselves, say — still has to produce a light editor.
    assert_eq!(
        editor_theme_for("one-dark", true, "my-light-theme", false, &entries()),
        "one-light"
    );
}

#[test]
fn following_the_ui_keeps_the_configured_id_when_nothing_matches() {
    // Nothing of the right cast exists, so there is no better answer than
    // the id the settings already carry; resolving it falls back on its own.
    let only_dark = vec![EditorThemeEntry {
        id: "one-dark".to_string(),
        name: "One Dark".to_string(),
        dark: true,
        builtin: true,
    }];
    assert_eq!(
        editor_theme_for("one-dark", true, "one-light", false, &only_dark),
        "one-dark"
    );
    // And an empty registry cannot make one up either.
    assert_eq!(
        editor_theme_for("whatever", true, "one-light", false, &[]),
        "whatever"
    );
}

#[test]
fn ids_are_matched_case_insensitively() {
    // `settings.json` is hand-editable and the registries resolve ids
    // case-insensitively, so this rule has to as well.
    assert_eq!(
        editor_theme_for("One-Dark", true, "irrelevant", true, &entries()),
        "one-dark"
    );
}

#[test]
fn every_word_the_shell_draws_is_translated() {
    // `t!` answers with the key path when a key is missing, so a typo
    // reaches the screen as "welcome.taglne".
    for label in [
        ts!("welcome.tagline"),
        ts!("welcome.hint", shortcut = "Ctrl+N"),
        ts!("welcome.new_connection"),
        ts!("welcome.import_jdbgen"),
        ts!("welcome.open_template"),
        ts!("welcome.saved"),
        ts!("welcome.empty"),
        ts!("titlebar.no_connection"),
        ts!("titlebar.tip_menu"),
        ts!("statusbar.no_connection"),
        ts!("statusbar.no_selection"),
        ts!("statusbar.tables_selected", count = 2),
        ts!(
            "statusbar.run",
            tables = 2,
            templates = 2,
            files = 4,
            dir = "/out"
        ),
        ts!(
            "statusbar.run_nowhere",
            tables = 2,
            templates = 2,
            files = 4
        ),
        ts!("generate.tab"),
        ts!("generate.template_set"),
        ts!("generate.set_custom"),
        ts!("generate.save_as_set"),
        ts!("generate.templates"),
        ts!("generate.no_templates"),
        ts!("generate.add_template"),
        ts!("generate.options"),
        ts!("generate.output_dir"),
        ts!("generate.author"),
        ts!("generate.variables"),
        ts!("generate.apply_abbreviations"),
        ts!("generate.rules"),
        ts!("generate.preview"),
        ts!("generate.dry_run"),
        ts!("generate.run"),
        ts!("generate.blocked_connection"),
        ts!("generate.blocked_tables"),
        ts!("generate.blocked_templates"),
        ts!("generate.blocked_output"),
        ts!("generate.blocked_running"),
        ts!("generate.at_line", line = 3),
        ts!("generate.read_failed", table = "T", reason = "why"),
        ts!("progress.title"),
        ts!("progress.count", done = 1, total = 4),
        ts!("progress.started", count = 4),
        ts!("progress.parsed", count = 2),
        ts!("progress.read_table", table = "T"),
        ts!("progress.written", path = "/out/a"),
        ts!("progress.skipped", path = "/out/a"),
        ts!("progress.failed", path = "/out/a", reason = "why"),
        ts!("progress.conflict", path = "/out/a"),
        ts!("progress.cancelling"),
        ts!("summary.title"),
        ts!("summary.done"),
        ts!("summary.with_failures"),
        ts!("summary.cancelled"),
        ts!(
            "summary.counts",
            written = 4,
            skipped = 0,
            failed = 0,
            warnings = 0
        ),
        ts!("summary.written"),
        ts!("summary.skipped"),
        ts!("summary.failed"),
        ts!("summary.open_output"),
        ts!("summary.conflict_title"),
        ts!("summary.conflict_body"),
        ts!("summary.overwrite"),
        ts!("summary.skip"),
        ts!("summary.overwrite_all"),
        ts!("summary.skip_all"),
        ts!("menu.preview"),
        ts!("menu.dry_run"),
        ts!("menu.generate"),
        ts!("menu.generate_menu"),
        ts!("menu.new_connection"),
        ts!("menu.settings"),
        ts!("menu.toggle_explorer"),
        ts!("menu.toggle_inspector"),
        ts!("menu.check_updates"),
        ts!("menu.about"),
        ts!("menu.quit"),
        ts!("menu.connection"),
        ts!("menu.view"),
        ts!("menu.mac.new_connection"),
        ts!("menu.mac.quit"),
        ts!("menu.abbreviations"),
        ts!("menu.import_jdbgen"),
    ] {
        assert!(!label.is_empty(), "empty label");
        for namespace in [
            "welcome.",
            "titlebar.",
            "statusbar.",
            "menu.",
            "generate.",
            "progress.",
            "summary.",
        ] {
            assert!(
                !label.starts_with(namespace),
                "untranslated label {label:?}"
            );
        }
    }
    // The welcome screen shows one heading or the other, never both, so a
    // shared wording would make the two states indistinguishable.
    assert_ne!(ts!("welcome.saved"), ts!("welcome.empty"));
}

#[test]
fn a_maximized_window_is_restored_maximized() {
    // The bounds a maximized window carries are its *restore* size, so both
    // halves have to survive: the state, and the size to un-maximize to.
    let state = WindowState {
        x: Some(10),
        y: Some(20),
        width: 1280,
        height: 720,
        maximized: true,
        ..WindowState::default()
    };
    let geometry = app_settings::saved_geometry(&state).expect("the position is set");
    assert_eq!(geometry.bounds().size.width, px(1280.));
    assert_eq!(geometry.bounds().origin.x, px(10.));
    assert!(state.maximized);
}

#[test]
fn the_window_appearance_follows_the_settings() {
    let opaque = WindowState::default();
    assert_eq!(
        window_appearance(&opaque),
        WindowBackgroundAppearance::Opaque
    );

    let translucent = WindowState {
        background_opacity: 0.8,
        ..WindowState::default()
    };
    assert_eq!(
        window_appearance(&translucent),
        WindowBackgroundAppearance::Transparent
    );

    // Blur wins even at full opacity: it is the stronger request, and a
    // blurred surface has to permit alpha whatever the fill does.
    let blurred = WindowState {
        background_blur: true,
        ..WindowState::default()
    };
    assert_eq!(
        window_appearance(&blurred),
        WindowBackgroundAppearance::Blurred
    );
}
