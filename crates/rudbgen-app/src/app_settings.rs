//! The application-wide settings state.
//!
//! [`AppSettings`] loaded from disk lives in a gpui global so that every view
//! reads one consistent snapshot. The settings dialog replaces the global and
//! saves to disk when the user applies changes; everything else only reads.
//!
//! The window geometry is the one part that flows the other way: the shell
//! records where the window is as it is moved and resized (see
//! [`record_window_geometry`]), and [`save`] writes the result out once, when
//! the last window closes. Writing it as it changes would put a file write in
//! the middle of a resize drag — a lot of syscalls to record a number nobody
//! reads until the next start. The *shape* of that geometry is
//! [`ruui_shell::WindowGeometry`]; what is here is the two lines that move it
//! in and out of [`WindowState`], which is rudbgen's own.
//!
//! # Two snapshots
//!
//! [`current`] is what is on disk (or will be, at the next save); [`effective`]
//! is what the window is drawn from. They differ only while the settings dialog
//! is showing unsaved edits — a palette being tried on, a font being compared —
//! which it publishes through [`set_preview`]. Keeping the preview *beside* the
//! persisted settings rather than writing it into them is what makes cancelling
//! free: dropping the override is the revert, and a window closed mid-dialog
//! still saves the settings the user last committed to.

use anyhow::Result;
use gpui::{App, Global, Hsla, SharedString};
use rudbgen_core::{AppSettings, WindowState};
use ruui::{ThemeDirs, theme_store};
use ruui_shell::{WindowGeometry, monospace_family};

/// The font family every piece of code text in rudbgen should render with.
///
/// The user's configured family — [`effective`], not [`current`], so a face
/// being previewed in the settings dialog shows up before it is saved — or,
/// absent one, the per-OS monospace default from
/// [`ruui_shell::monospace_family`]. This is the app layer keeping the promise
/// [`rudbgen_core::AppSettings::editor_font_family`] documents: a `None` there
/// means "the per-OS monospace default chosen by the app layer".
///
/// Rendering code should call this rather than `monospace_family` directly:
/// `monospace_family` only ever answers the fallback, never the user's
/// choice.
pub fn editor_font(cx: &App) -> SharedString {
    effective(cx)
        .editor_font_family
        .map(SharedString::from)
        .unwrap_or_else(|| monospace_family(cx))
}

/// Global wrapper holding the current [`AppSettings`].
pub struct CurrentSettings(pub AppSettings);

impl Global for CurrentSettings {}

/// Global wrapper holding unsaved settings the window is drawn from.
///
/// Installed while the settings dialog previews an edit and removed again when
/// it closes; see [`set_preview`].
struct PreviewSettings(AppSettings);

impl Global for PreviewSettings {}

/// The saved placement in `state`, or `None` when it carries no position.
///
/// `None` is a first run, or a window that was never moved: the platform picks
/// the placement then, and the caller centres the saved *size* on the active
/// display rather than guessing at coordinates.
pub fn saved_geometry(state: &WindowState) -> Option<WindowGeometry> {
    WindowGeometry::saved(state.x, state.y, state.width, state.height, state.maximized)
}

/// Writes `geometry` into `state`, leaving its appearance alone.
///
/// The opacity, the blur and the title bar style sitting beside the placement
/// are the user's choices and are never written back from the window they were
/// applied to.
fn apply_geometry(geometry: WindowGeometry, state: &mut WindowState) {
    state.x = Some(geometry.x);
    state.y = Some(geometry.y);
    state.width = geometry.width;
    state.height = geometry.height;
    state.maximized = geometry.maximized;
}

/// Whether `state` already records `geometry`.
fn records_geometry(geometry: WindowGeometry, state: &WindowState) -> bool {
    saved_geometry(state) == Some(geometry)
}

/// Install the settings global from disk. Call once at start-up.
///
/// A file that cannot be read falls back to defaults; the app must start
/// regardless of what is on disk.
pub fn init(cx: &mut App) {
    let settings = AppSettings::load().unwrap_or_else(|err| {
        log::warn!("starting with default settings: {err:#}");
        AppSettings::default()
    });
    cx.set_global(CurrentSettings(settings));
}

/// The two directories `ruui`'s theme store reads and writes.
///
/// The widget kit has no configuration directory of its own and never guesses
/// at one — which is the point of it not knowing what application it is drawn
/// into — so this is where rudbgen's answer is given, once, from
/// `rudbgen-core`'s paths. Every call into
/// [`theme_store`](ruui::theme_store) that touches the disk takes it.
///
/// # Errors
///
/// Fails when no configuration directory can be determined for the current
/// user, which is what `rudbgen-core` reports when there is no home directory.
pub fn theme_dirs() -> Result<ThemeDirs> {
    Ok(ThemeDirs {
        ui_themes: rudbgen_core::ui_themes_dir()?,
        editor_themes: Some(rudbgen_core::editor_themes_dir()?),
    })
}

/// The same two directories, with empty paths where there is no configuration
/// directory at all.
///
/// [`theme_dirs`] is the fallible answer, and every caller that is about to
/// *write* wants it. This is for the two catalogues the settings dialog builds
/// at construction, which have to exist before anyone asks them for anything:
/// an empty path holds no palettes and refuses every write, which is the same
/// outcome as the error, reported at the moment the user can see it rather than
/// while a dialog is being assembled.
pub fn theme_dirs_or_empty() -> ThemeDirs {
    theme_dirs().unwrap_or_else(|err| {
        log::warn!("cannot locate the theme directories: {err:#}");
        ThemeDirs {
            ui_themes: std::path::PathBuf::new(),
            editor_themes: None,
        }
    })
}

/// Reads both theme directories and installs what they hold.
///
/// Called at start-up and again after every change rudbgen makes to the files,
/// since the two registries are swapped whole rather than edited in place. A
/// configuration directory that cannot be located is logged and no more:
/// nothing here is worth refusing to draw a window over, and the built-in
/// palettes are still there.
pub fn reload_themes(cx: &mut App) {
    match theme_dirs() {
        Ok(dirs) => theme_store::reload(&dirs, cx),
        Err(err) => log::warn!("cannot locate the theme directories: {err:#}"),
    }
}

/// A snapshot of the current settings.
pub fn current(cx: &App) -> AppSettings {
    cx.try_global::<CurrentSettings>()
        .map(|g| g.0.clone())
        .unwrap_or_default()
}

/// Replace the settings global. The caller is responsible for persistence and
/// for re-applying the settings to open windows.
pub fn replace(settings: AppSettings, cx: &mut App) {
    cx.set_global(CurrentSettings(settings));
}

/// A snapshot of the settings the interface should currently be drawn from.
///
/// The preview, while the settings dialog is showing one, and otherwise
/// [`current`]. Everything that *renders* from the settings reads this;
/// everything that *persists* them reads [`current`].
pub fn effective(cx: &App) -> AppSettings {
    cx.try_global::<PreviewSettings>()
        .map(|preview| preview.0.clone())
        .unwrap_or_else(|| current(cx))
}

/// Show `settings` without saving them.
///
/// The settings dialog calls this on every edit that is visible before it is
/// committed. Nothing is written to disk and [`current`] is untouched, so
/// [`clear_preview`] is all it takes to put the window back.
pub fn set_preview(settings: AppSettings, cx: &mut App) {
    cx.set_global(PreviewSettings(settings));
}

/// Drop the preview, if there is one, so [`effective`] answers [`current`]
/// again.
///
/// Idempotent: the dialog closes by more paths than it opens by, and every one
/// of them ends here.
pub fn clear_preview(cx: &mut App) {
    if cx.has_global::<PreviewSettings>() {
        cx.remove_global::<PreviewSettings>();
    }
}

/// Records where the window is, without touching the disk.
///
/// Called from the shell's window-bounds observer, so it runs on every move and
/// every step of a resize drag. Nothing is written and no global is marked dirty
/// unless a value actually changed: the observer fires far more often than the
/// rounded geometry differs, and a dirty global would schedule a repaint of a
/// window that is already repainting itself.
pub fn record_window_geometry(geometry: WindowGeometry, cx: &mut App) {
    let Some(settings) = cx.try_global::<CurrentSettings>() else {
        return;
    };
    if records_geometry(geometry, &settings.0.window) {
        return;
    }
    apply_geometry(geometry, &mut cx.global_mut::<CurrentSettings>().0.window);
}

/// Writes the settings global to `settings.json`.
///
/// Reports rather than propagates: the callers are shutdown paths, where there
/// is no longer a window to show a failure in and nothing useful to do about one
/// either.
pub fn save(cx: &App) {
    if let Err(error) = current(cx).save() {
        log::warn!("could not save the settings: {error:#}");
    }
}

/// Applies the configured window opacity to a background fill.
///
/// **At most one such fill may cover any given pixel**, and between them they
/// must leave no pixel of the body uncovered. The window surface starts out
/// fully transparent, so a single translucent fill lets the desktop (or the
/// acrylic blur behind the window) show through. A second one on top does not:
/// gpui's Windows renderer blends the alpha channel additively
/// (`SrcBlendAlpha = ONE, DestBlendAlpha = ONE`), so two fills of, say, 0.75 and
/// 0.62 saturate the surface alpha at 1.0 and the window goes opaque. That is
/// why the toolbar and the status bar paint their surface untinted.
///
/// Two fills go through here, and they tile the body rather than stack: the
/// explorer's surface under the sidebar, and the body's background over the work
/// area beside it. The row holding the two paints nothing itself, so neither one
/// ever lands on a pixel the other already has. Splitting it that way is what
/// lets the blur carry on behind the sidebar instead of stopping at its edge.
///
/// What sits *over* the work area's fill would each be a second fill on the same
/// pixels, so while the window is translucent the result grid and the ERD and
/// query-builder canvases paint no background at all: they ask
/// [`ruui::window_translucent`] and skip it, leaving the fill below as the
/// only tinted one. Tinting them instead of skipping is the trap this whole
/// comment is about.
///
/// The SQL editor is the deliberate exception and stays opaque, translucent
/// window or not: code is read a character at a time and a desktop behind it is
/// the wrong place for contrast to go. It costs the blur the editor's share of
/// the window, which is the trade. Nothing about the additive alpha above
/// constrains it — an *opaque* fill over a tinted one has no saturation problem,
/// it simply wins.
///
/// The opacity itself lives in a widget-layer global, so that the leaves which
/// have to agree with this can reach it; the shell pushes it there with
/// [`ruui::set_window_tint`] at start-up and on a settings *save*. Which
/// means this follows neither [`current`] nor [`effective`] directly, and in
/// particular does not follow a preview — deliberately. The fill is only half of
/// what makes a window translucent: the other half is the platform surface being
/// told to permit alpha, which happens in
/// [`gpui::Window::set_background_appearance`] and only when the settings are
/// saved. Tinting ahead of that would compose against an opaque surface and
/// merely darken the window, which is a worse answer than not previewing at all.
pub fn window_tint(color: Hsla, cx: &App) -> Hsla {
    // Deferred to the widget layer, which is where the leaves that have to agree
    // with this can reach it; `current` and the global are set from the same
    // value at the same moment.
    ruui_shell::window_tint(color, cx)
}

#[cfg(test)]
mod tests {
    use rudbgen_core::TitlebarStyle;

    use super::*;

    /// A placement that is nothing like the defaults, so a value left behind by
    /// mistake shows up as itself.
    fn geometry() -> WindowGeometry {
        WindowGeometry {
            x: 120,
            y: 60,
            width: 1600,
            height: 1000,
            maximized: true,
        }
    }

    #[test]
    fn a_placement_survives_the_trip_through_the_settings() {
        let mut state = WindowState {
            background_opacity: 0.8,
            background_blur: true,
            titlebar: TitlebarStyle::System,
            ..WindowState::default()
        };
        apply_geometry(geometry(), &mut state);

        assert_eq!(saved_geometry(&state), Some(geometry()));
        // The appearance is the user's and must not have been touched.
        assert_eq!(state.background_opacity, 0.8);
        assert!(state.background_blur);
        assert_eq!(state.titlebar, TitlebarStyle::System);
    }

    #[test]
    fn a_state_without_a_position_has_no_saved_placement() {
        // A first run: the size is known, the coordinates are not, and the
        // caller has to centre rather than place.
        let state = WindowState::default();
        assert_eq!(state.x, None);
        assert_eq!(saved_geometry(&state), None);

        let half_placed = WindowState {
            x: Some(10),
            ..WindowState::default()
        };
        assert_eq!(saved_geometry(&half_placed), None);
    }

    #[test]
    fn recording_the_same_placement_twice_changes_nothing() {
        // The guard `record_window_geometry` relies on, tested without an `App`:
        // a window that is repainting but has not moved must not dirty the
        // settings global.
        let mut state = WindowState::default();
        apply_geometry(geometry(), &mut state);
        assert!(records_geometry(geometry(), &state));

        let moved = WindowGeometry {
            x: 121,
            ..geometry()
        };
        assert!(!records_geometry(moved, &state));
    }

    /// The whole of the settings dialog's live preview, and its undo: an
    /// override that hides the saved settings from everything that draws, and
    /// nothing at all from what saves.
    #[gpui::test]
    fn a_preview_hides_the_saved_settings_until_it_is_dropped(cx: &mut gpui::TestAppContext) {
        cx.update(|cx| {
            cx.set_global(CurrentSettings(AppSettings::default()));
            assert_eq!(effective(cx).theme, current(cx).theme);

            let previewed = AppSettings {
                theme: "dracula".to_string(),
                ui_font_size: 20.0,
                ..current(cx)
            };
            set_preview(previewed, cx);
            assert_eq!(effective(cx).theme, "dracula");
            assert_eq!(effective(cx).ui_font_size, 20.0);
            // And nothing of it reached what would be written to disk.
            assert_eq!(current(cx).theme, "one-dark");
            assert_eq!(current(cx).ui_font_size, 14.0);

            // Cancelling is the absence of the override, not a second copy.
            clear_preview(cx);
            assert_eq!(effective(cx).theme, "one-dark");
            assert_eq!(effective(cx).ui_font_size, 14.0);
            // Every path that closes the dialog ends here, so it has to be safe
            // to run twice.
            clear_preview(cx);
            assert_eq!(effective(cx).theme, "one-dark");
        });
    }

    /// `editor_font` is the one rendering code is meant to call: a configured
    /// family wins, and its absence falls through to the same OS default
    /// `monospace_family` answers.
    #[gpui::test]
    fn editor_font_prefers_the_configured_family_and_falls_back_to_monospace(
        cx: &mut gpui::TestAppContext,
    ) {
        cx.update(|cx| {
            cx.set_global(CurrentSettings(AppSettings {
                editor_font_family: Some("Custom Face".to_string()),
                ..AppSettings::default()
            }));
            assert_eq!(editor_font(cx), "Custom Face");

            cx.set_global(CurrentSettings(AppSettings {
                editor_font_family: None,
                ..AppSettings::default()
            }));
            assert_eq!(editor_font(cx), monospace_family(cx));
        });
    }

    /// A preview must not survive the settings being replaced under it either:
    /// saving replaces the global and the dialog drops the override, and the
    /// two together have to leave one answer.
    #[gpui::test]
    fn saving_and_dropping_the_preview_agree(cx: &mut gpui::TestAppContext) {
        cx.update(|cx| {
            cx.set_global(CurrentSettings(AppSettings::default()));
            let edited = AppSettings {
                theme: "gruvbox-dark".to_string(),
                ..current(cx)
            };
            set_preview(edited.clone(), cx);
            replace(edited, cx);
            clear_preview(cx);
            assert_eq!(effective(cx).theme, "gruvbox-dark");
            assert_eq!(current(cx).theme, "gruvbox-dark");
        });
    }
}
