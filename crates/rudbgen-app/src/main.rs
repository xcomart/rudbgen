//! rudbgen application entry point. Window behavior lives in [`workspace`].

use gpui::actions;
mod abbreviation_dialog;
mod app_settings;
mod builtin_templates;
mod connection;
mod connection_dialog;
mod driver_manager;
mod explorer;
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
        /// Opens the connection dialog.
        NewConnection,
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
    ]
);

mod workspace;

use workspace::editor_theme_for;

fn main() {
    workspace::run();
}
