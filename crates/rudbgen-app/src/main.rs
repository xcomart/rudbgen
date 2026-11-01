#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

//! rudbgen application entry point. Window behavior lives in [`workspace`].

use gpui::actions;
mod abbreviation_dialog;
mod app_settings;
mod builtin_templates;
mod cli;
mod connection;
mod connection_dialog;
mod driver_manager;
mod explorer;
#[cfg(test)]
mod frame_profile;
mod generate_job;
mod generate_pane;
mod i18n;
mod icons;
mod import_dialog;
mod inspector;
mod maven;
mod palette;
mod pane_item;
mod preview_pane;
mod prompt;
mod sample_db;
mod settings_dialog;
mod template_pane;
mod template_syntax;
mod variable_palette;

// Compiles `locales/*.yml` into the binary and defines the machinery `t!`
// expands to, which is why it has to sit in the crate root. `fallback = "en"`
// is per key, not per locale: a string a translator has not got to yet shows
// in English while the rest of that language stays translated.
rust_i18n::i18n!("locales", fallback = "en");

actions!(
    rudbgen,
    [
        /// Leaves the application.
        Quit,
        /// Opens the connection dialog on a new, empty profile.
        NewConnection,
        /// Opens the connection dialog on the saved profiles, to edit them.
        ManageConnections,
        /// Opens the settings dialog.
        OpenSettings,
        /// Opens the about box.
        ShowAbout,
        /// Asks GitHub whether there is a newer release.
        CheckUpdates,
        /// Closes whatever overlay is on top, innermost first.
        DismissDialog,
        /// Shows and hides the explorer sidebar.
        ToggleExplorer,
        /// Shows and hides the inspector panel.
        ToggleInspector,
        /// Runs the generator over the ticked tables and templates.
        Generate,
        /// Renders one table × template pair into the Preview tab.
        Preview,
        /// Renders every pair into memory, writing nothing.
        DryRun,
        /// Creates an empty template file where the user says, and opens it.
        NewTemplate,
        /// Opens a template file in a tab of its own.
        OpenTemplate,
        /// Writes the template tab that is on top back to its file.
        SaveTemplate,
        /// Shows and hides the live preview beside the template being edited.
        ToggleLivePreview,
        /// Offers what may be written where the caret is.
        TriggerCompletion,
        /// Opens the abbreviation rules editor.
        EditAbbreviations,
        /// Opens the jdbgen import wizard.
        ImportJdbgen,
        /// Brings the work tab right of the one on top to the front.
        NextTab,
        /// Brings the work tab left of the one on top to the front.
        PrevTab,
        /// Closes the work tab on top, asking first when it holds edits.
        CloseTab,
        /// Moves the keyboard onto the work area's tab strip.
        FocusTabs,
        /// Moves the caret to the next problem of the template on top.
        NextDiagnostic,
        /// Moves the caret to the previous problem of the template on top.
        PrevDiagnostic,
        /// Adds the template on top to the connection's generation list.
        AddToGeneration,
        /// Saves whatever the dialog on top is editing (`Ctrl+S` inside it).
        SaveDialog,
        /// Adds a blank profile inside the connection dialog (`Ctrl+N` there).
        NewProfile,
    ]
);

/// Brings the work tab at the zero-based index carried by the action to the
/// front.
#[derive(Clone, PartialEq, Default, Debug, gpui::Action)]
#[action(namespace = rudbgen, no_json)]
struct SelectTab(
    /// Zero-based index of the tab, counted along the strip.
    usize,
);

mod workspace;

use workspace::editor_theme_for;

fn main() {
    workspace::run();
}
