//! Bootstrap.

use super::*;

/// Installs both palettes the settings name.
///
/// The chrome theme comes straight from the configured id; the editor theme
/// goes through [`editor_theme_for`], which is where "follow the UI theme" is
/// decided. That decision lives here rather than in the settings dialog because
/// it has to hold whatever changed the inputs — a theme file appearing in the
/// user's directory moves the answer without anybody having opened a dialog.
///
/// An id nothing answers to — a theme file the user has since deleted — falls
/// back to the default rather than failing; see [`ThemeRegistry::resolve`].
pub(super) fn apply_themes(settings: &AppSettings, cx: &mut App) {
    let ui = ThemeRegistry::resolve(&settings.theme, cx);
    let editor_id = editor_theme_for(
        &settings.editor_theme,
        settings.editor_theme_follows_ui,
        &settings.theme,
        ui.dark,
        &EditorThemeRegistry::all(cx),
    );
    let editor = EditorThemeRegistry::resolve(&editor_id, cx);
    set_theme(ui, cx);
    set_editor_theme(editor, cx);
}

/// The editor theme to install, given the configured one and the chrome theme.
///
/// With the "follows the UI" switch off the configured id is used as it stands.
/// With it on the answer is the first of these that exists:
///
/// 1. the editor theme sharing the chrome theme's id, when its cast matches the
///    chrome — which is how the pairs that ship under one name stay together,
///    and, since every built-in chrome theme has an editor theme of the same id,
///    is the answer for every built-in;
/// 2. the configured theme, when its cast matches the chrome — the fallback for
///    a chrome theme of the user's own, which no editor theme is named after;
/// 3. any editor theme of the right cast;
/// 4. the configured id after all, when nothing of the right cast exists.
///
/// The namesake comes first because that is what the switch promises: its label
/// says the editor theme is *matched to* the interface theme, not merely kept on
/// the same side of light and dark, and while it is on the settings dialog
/// disables the editor theme dropdown outright — so there is no pick of the
/// user's here to preserve, and letting the configured id win would freeze the
/// editor on one palette however far the chrome moved.
///
/// The cast is still checked in rule 1, though, and deliberately: a user who has
/// written a *dark* editor theme under the id of a *light* chrome theme must not
/// have it dragged into a light window. Preventing that pairing is the whole
/// reason the switch exists, so it outranks the name match.
///
/// Pure and taking the theme list as an argument so that the rule can be tested
/// without an [`App`]; the caller supplies [`EditorThemeRegistry::all`].
pub(crate) fn editor_theme_for(
    configured: &str,
    follows_ui: bool,
    ui_theme_id: &str,
    ui_dark: bool,
    entries: &[EditorThemeEntry],
) -> String {
    if !follows_ui {
        return configured.to_string();
    }

    let matching = |id: &str| {
        entries
            .iter()
            .find(|entry| entry.id.eq_ignore_ascii_case(id) && entry.dark == ui_dark)
    };
    if let Some(entry) = matching(ui_theme_id).or_else(|| matching(configured)) {
        return entry.id.clone();
    }
    entries
        .iter()
        .find(|entry| entry.dark == ui_dark)
        .map(|entry| entry.id.clone())
        .unwrap_or_else(|| configured.to_string())
}

/// Records the window's placement in the settings global.
pub(super) fn record_window_geometry(window: &Window, cx: &mut App) {
    app_settings::record_window_geometry(rugpui_shell::window_geometry(window), cx);
}

/// The placement to open the window at.
pub(super) fn window_bounds(state: &WindowState, cx: &mut App) -> WindowBounds {
    rugpui_shell::window_bounds(
        app_settings::saved_geometry(state),
        state.width,
        state.height,
        state.maximized,
        cx,
    )
}

/// rudbgen's title bar setting, as the shell spells it.
///
/// The two enums are the same two variants with the same `snake_case`
/// serialisation, and they are two enums because [`rudbgen_core`] is
/// deliberately free of gpui: the shell's copy is what
/// [`chrome::draws_own_titlebar`] and the window options branch on, and the
/// settings file is `rudbgen-core`'s. This is the one line between them.
pub(super) fn chrome_style(style: TitlebarStyle) -> chrome::TitlebarStyle {
    match style {
        TitlebarStyle::Custom => chrome::TitlebarStyle::Custom,
        TitlebarStyle::System => chrome::TitlebarStyle::System,
    }
}

/// Maps the window settings onto a gpui background appearance.
pub(super) fn window_appearance(window: &WindowState) -> WindowBackgroundAppearance {
    chrome::window_appearance(window.background_blur, window.background_opacity)
}

/// The application menu bar, in macOS layout.
///
/// gpui only turns this into a real menu bar on macOS — the Windows and Linux
/// backends store it and draw nothing — so the other platforms get the same
/// commands from the application menu built by [`Workspace::render_app_menu`].
/// Every item dispatches an action that is also bound to a shortcut in
/// [`bind_shortcuts`], which is what lets the macOS backend label the items with
/// their key equivalents; register the bindings first so the keymap it reads is
/// already populated.
///
/// About, Check for updates, Settings and Quit live in the application menu
/// because that is where macOS users look for them.
///
/// The item labels are translated, but the application menu's own name is the
/// "rudbgen" wordmark and stays as it is. Rebuilt and re-installed whenever the
/// language changes, because gpui takes the menu bar by value.
pub(super) fn app_menus() -> Vec<Menu> {
    vec![
        Menu {
            name: APP_NAME.into(),
            items: vec![
                MenuItem::action(ts!("menu.about"), ShowAbout),
                MenuItem::action(ts!("menu.check_updates"), CheckUpdates),
                MenuItem::separator(),
                MenuItem::action(ts!("menu.settings"), OpenSettings),
                MenuItem::separator(),
                MenuItem::action(ts!("menu.mac.quit"), Quit),
            ],
            disabled: false,
        },
        Menu {
            name: ts!("menu.connection"),
            items: vec![
                MenuItem::action(ts!("menu.mac.new_connection"), NewConnection),
                MenuItem::separator(),
                MenuItem::action(ts!("menu.import_jdbgen"), ImportJdbgen),
            ],
            disabled: false,
        },
        Menu {
            name: ts!("menu.template"),
            items: vec![
                MenuItem::action(ts!("menu.open_template"), OpenTemplate),
                MenuItem::action(ts!("menu.save_template"), SaveTemplate),
                MenuItem::separator(),
                MenuItem::action(ts!("menu.toggle_live_preview"), ToggleLivePreview),
                MenuItem::action(ts!("menu.trigger_completion"), TriggerCompletion),
            ],
            disabled: false,
        },
        Menu {
            name: ts!("menu.view"),
            items: vec![
                MenuItem::action(ts!("menu.toggle_explorer"), ToggleExplorer),
                MenuItem::action(ts!("menu.toggle_inspector"), ToggleInspector),
            ],
            disabled: false,
        },
        Menu {
            name: ts!("menu.generate_menu"),
            items: vec![
                MenuItem::action(ts!("menu.preview"), Preview),
                MenuItem::action(ts!("menu.dry_run"), DryRun),
                MenuItem::separator(),
                MenuItem::action(ts!("menu.generate"), Generate),
                MenuItem::separator(),
                MenuItem::action(ts!("menu.abbreviations"), EditAbbreviations),
            ],
            disabled: false,
        },
    ]
}

/// Registers every shortcut the workspace listens for.
///
/// A binding here beats the focused view: gpui matches key bindings along the
/// whole dispatch path before it delivers the key event itself, so every chord
/// bound in this function is taken away from the template editor that will one
/// day be inside a pane. Only chords no editor claims are bound from here for
/// that reason.
pub(super) fn bind_shortcuts(cx: &mut App) {
    let modifier = if cfg!(target_os = "macos") {
        "cmd"
    } else {
        "ctrl"
    };

    cx.bind_keys(vec![
        KeyBinding::new(&format!("{modifier}-q"), Quit, None),
        KeyBinding::new(&format!("{modifier}-n"), NewConnection, Some(KEY_CONTEXT)),
        KeyBinding::new(&format!("{modifier}-,"), OpenSettings, Some(KEY_CONTEXT)),
        // `Ctrl+B` is what every editor with a sidebar binds it to, and unlike
        // an editing chord it has no contender inside a template editor.
        KeyBinding::new(&format!("{modifier}-b"), ToggleExplorer, Some(KEY_CONTEXT)),
        // The panel on the other edge, on the letter it starts with. Nothing
        // else in the shell claims it, and no editor does either.
        KeyBinding::new(&format!("{modifier}-i"), ToggleInspector, Some(KEY_CONTEXT)),
        // The one chord the run gets. `Ctrl+G` is "go" in every generator that
        // has ever had a shortcut for it, and no editor claims it — where
        // `Ctrl+R` would be a find-and-replace inside a template tab.
        // The other two commands are deliberately unbound: they are one press
        // of a button away, and a chord for each would take three from the
        // editor for a command nobody repeats.
        KeyBinding::new(&format!("{modifier}-g"), Generate, Some(KEY_CONTEXT)),
        // The template tab's three. `Ctrl+S` is the one chord an editor may
        // not claim for itself — it means "write this file" everywhere — and
        // the editor deliberately binds neither it nor `Ctrl+Space`, which is
        // what asks for a completion in every editor that has one. The preview
        // toggle takes the letter of the thing it shows.
        KeyBinding::new(&format!("{modifier}-s"), SaveTemplate, Some(KEY_CONTEXT)),
        KeyBinding::new(
            &format!("{modifier}-shift-p"),
            ToggleLivePreview,
            Some(KEY_CONTEXT),
        ),
        KeyBinding::new("ctrl-space", TriggerCompletion, Some(KEY_CONTEXT)),
        KeyBinding::new("escape", DismissDialog, Some(KEY_CONTEXT)),
    ]);
}

/// Copies the shipped templates into the configuration directory, offers the
/// two built-in sets and seeds the sample connection, once.
///
/// Every failure here is logged rather than fatal: an application whose
/// template directory could not be written is still an application, with a
/// template list the user fills in themselves.
pub(super) fn install_builtins() {
    match rudbgen_core::templates_dir() {
        Ok(dir) => match builtin_templates::install(&dir) {
            Ok(0) => {}
            Ok(count) => log::info!("copied {count} built-in templates into {}", dir.display()),
            Err(error) => log::error!("could not install the built-in templates: {error:#}"),
        },
        Err(error) => log::error!("no configuration directory for the templates: {error:#}"),
    }

    // The same idea one file over, and ahead of the template sets because a
    // template-sets.json that cannot be read must not cost the user their
    // sample connection: a first run gets the sample H2 database copied
    // somewhere writable and a connection that opens it, so that the connection
    // list is not empty before there is a server to point at. Only when
    // `connections.json` is missing entirely; see `sample_db`.
    sample_db::install();

    let mut sets = match TemplateSetStore::load() {
        Ok(sets) => sets,
        Err(error) => {
            log::error!("could not read template-sets.json: {error:#}");
            return;
        }
    };
    if builtin_templates::seed(&mut sets)
        && let Err(error) = sets.save()
    {
        log::error!("could not write template-sets.json: {error:#}");
    }
}

pub(super) fn run() {
    env_logger::init();

    // The half of `rugpui_shell::init` that needs no `App`: `apply_pending` and
    // `clean_leftovers` both read the identity through it, and with none
    // installed they log an error and no-op rather than panic. It has to run
    // ahead of both — and ahead of `current_exe()` moving under an install —
    // which is why it is the very first thing `main` does.
    rugpui_shell::init_process_identity(IDENTITY);

    // An update the previous run could only stage — because a JVM was loaded
    // into it and Windows will not let its files be renamed — is applied here,
    // synchronously, before the application exists and therefore before anything
    // can load a JVM into *this* process. It answers `true` only when it has
    // already spawned a fresh process on the new build, at which point the one
    // useful thing left to do is get out of its way. See `update::apply_pending`.
    if update::apply_pending() {
        return;
    }

    // The icon set has to be installed before the app runs: `svg()` resolves
    // every path through this source, and the default one answers `None`.
    // `LastWindowClosed` rather than the default, which is this only away from
    // macOS: there an app whose last window closes stays in the Dock, and
    // rudbgen has nothing to offer once its window is gone — no menu bar
    // command that opens a new one, no connection or template worth keeping
    // alive in the background. One rule on every platform is what the sibling
    // applications do too.
    let app = gpui_platform::application()
        .with_assets(icons::ICONS)
        .with_quit_mode(QuitMode::LastWindowClosed);
    app.run(|cx: &mut App| {
        // Everything `rugpui-shell` is not allowed to guess at, handed over
        // before anything that could read it runs. `set_strings` goes through
        // `ts!`, so the shell follows a language change without being told
        // again; `set_update_policy` is the two-line window onto the
        // `ignored_update` field of `settings.json`.
        rugpui_shell::init(IDENTITY, cx);
        rugpui_shell::set_strings(Box::new(AppStrings), cx);
        rugpui_shell::set_update_policy(Box::new(IgnoredUpdate), cx);

        if let Err(error) = rudbgen_core::init_secrets() {
            log::warn!("the OS keychain is unavailable: {error}");
        }

        // A self-update renames the copies it replaces aside instead of
        // deleting them — Windows cannot delete a running image, and one code
        // path for three platforms is worth more than an immediate unlink on
        // the two that could. This is the other half: the leftovers are swept
        // up on the next launch. On the background executor because a bundled
        // JRE or a `.app` bundle is a recursive delete of thousands of files
        // and nothing on screen depends on it.
        cx.background_executor()
            .spawn(async { update::clean_leftovers() })
            .detach();

        // Load the settings before the widget layer installs its default
        // palettes, then override those to match what the user configured.
        app_settings::init(cx);
        let settings = app_settings::current(cx);
        // Ahead of everything that renders a string — the menu bar included —
        // so nothing is ever built in the wrong language and then corrected.
        i18n::apply(settings.language.as_deref());

        rugpui::init(cx);
        // The inspector's Columns tab is a grid, and the grid binds its own
        // keys; without this the panel draws and cannot be walked.
        rugpui_grid::init(cx);
        // The template tab's buffer and both read-only previews are
        // `EditorView`s, which bind their own keys to the `Editor` context.
        rugpui_editor::init(cx);
        // After the widget layers, because they scope their bindings to key
        // contexts the shell's own bindings have to be able to outrank.
        bind_shortcuts(cx);
        cx.set_menus(app_menus());

        // The shipped templates and the two sets that name them, on the first
        // run that finds them missing. Neither ever overwrites anything the
        // user has: an edited template is left alone, and a set that was
        // deleted stays deleted (see `builtin_templates`). Cheap enough to do
        // on the UI thread — three `include_bytes!` blobs and one JSON file —
        // and it has to be done before the first window draws a template list.
        install_builtins();

        // Before the palettes are applied: the ids in the settings may well
        // name themes of the user's own.
        app_settings::reload_themes(cx);
        apply_themes(&settings, cx);
        // The same value `window_appearance` below reads, handed to the widget
        // layer so the widgets know whether to paint a background of their own;
        // see [`app_settings::window_tint`].
        set_window_tint(settings.window.background_opacity, cx);

        cx.on_action(|_: &Quit, cx: &mut App| cx.quit());
        // The window's geometry is only in memory until here; this is the one
        // write of `settings.json` the shell performs. Nothing in the closure
        // re-enters gpui — the file write is the whole of it — which is what
        // keeps it clear of the X11 backend's re-entrancy trap, the one rugpui's
        // vendored `client.rs` patch exists for. Quitting is no longer this
        // closure's business: gpui does it, from the quit mode set on the
        // application above, and it runs these observers first — so the
        // settings are on disk before the process starts winding down.
        cx.on_window_closed(|cx, _closed| {
            if cx.windows().is_empty() {
                app_settings::save(cx);
            }
        })
        .detach();

        let bounds = window_bounds(&settings.window, cx);
        // Read once, here: `appears_transparent` is what strips the platform
        // caption, and both Windows and macOS decide that when the window is
        // created. Changing the setting later cannot reach an open window,
        // which is why the settings dialog has to say a restart is needed.
        let titlebar = settings.window.titlebar;
        cx.open_window(
            WindowOptions {
                window_bounds: Some(bounds),
                titlebar: Some(TitlebarOptions {
                    title: Some(APP_NAME.into()),
                    appears_transparent: titlebar == TitlebarStyle::Custom,
                    // Ignored unless the caption is transparent; it moves the
                    // traffic lights AppKit keeps drawing into the title bar
                    // band the app puts in the caption's place.
                    traffic_light_position: (titlebar == TitlebarStyle::Custom)
                        .then_some(TRAFFIC_LIGHT_ORIGIN),
                }),
                // Only the Linux backends read this. `appears_transparent`
                // above means nothing to X11 and Wayland: the caption stays the
                // compositor's until the window asks for client-side
                // decorations outright. gpui falls back to server decorations
                // on its own when no compositor is present, and
                // [`draws_own_titlebar`] follows what the window actually got.
                window_decorations: (titlebar == TitlebarStyle::Custom)
                    .then_some(gpui::WindowDecorations::Client),
                app_id: Some(APP_ID.into()),
                // A translucent or blurred window needs the platform surface to
                // permit alpha; the body then tints its own background.
                window_background: window_appearance(&settings.window),
                ..Default::default()
            },
            |window, cx| {
                let workspace = cx.new(|cx| Workspace::new(titlebar, window, cx));
                let handle = workspace.read(cx).focus_handle.clone();
                window.focus(&handle, cx);
                apply_caption_theme(window, &theme(cx), cx);
                workspace
            },
        )
        .expect("failed to open the rudbgen window");

        cx.activate(true);
    });
}
