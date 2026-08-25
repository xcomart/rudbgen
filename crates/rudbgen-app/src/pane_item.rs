//! What one tab of the work area shows.
//!
//! The split layout itself is [`ruui_shell::pane`]: a binary tree whose leaves
//! are panes and whose panes are strips of tabs, generic over what a tab is
//! because none of its rules depend on that. This module supplies the missing
//! half — rudbgen's own answer to "what is a tab" — and the one lookup over it
//! the shell around the strip needs.
//!
//! The work area uses [`Pane`] — the tab strip's list and its active index —
//! but not [`PaneTree`](ruui_shell::PaneTree): the template tab splits itself
//! down the middle rather than splitting the *pane*, so the tree's own
//! operations are the shell's and go unused here.
//!
//! # Why a tab is an enum
//!
//! A tab is an enum rather than a boxed trait object because the window has to
//! know what it is looking at anyway: the inspector shows a variable palette
//! beside a template tab and a table's columns beside the Generate tab, and a
//! `Box<dyn Panel>` would only push that decision into a downcast. A new kind
//! of tab is one variant here plus one arm where the window renders the active
//! tab; the tree stays untouched.
//!
//! Nothing here carries a view. A variant carries the tab's *identity* instead
//! — which is what the lookups below match on and what a reopened tab is
//! recognised by — and the window keeps the views beside the strip. That is
//! what lets a template tab outlive a reconnection: the strip is rebuilt when
//! the connection changes and the buffers are not.

use std::path::{Path, PathBuf};

use gpui::SharedString;
use ruui_shell::Pane;

/// What one tab of a pane shows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PaneItem {
    /// The Generate tab: template set, template list, options (§4.4).
    ///
    /// Permanent and unique. It is the tab the whole run is configured in, and
    /// a second one would be a second answer to "what will this write".
    Generate,
    /// One template document, open in the editor (§4.5).
    ///
    /// Keyed by the file it was opened from, which is what
    /// [`WorkTabs::template_of`] matches on: opening the same template twice is
    /// a navigation to the tab that already has it, not a second buffer over
    /// one file.
    Template {
        /// The file on disk, as the template list named it.
        file: PathBuf,
        /// What the tab is labelled — the template's name, not its path.
        title: SharedString,
        /// Whether the buffer has edits `Ctrl+S` has not written yet.
        ///
        /// The tab shows it as a marker, and it is what
        /// [`PaneItem::blocks_close`] answers from.
        dirty: bool,
    },
    /// A dry run of one table × template pair, rendered to memory (§9).
    Preview {
        /// What the tab is labelled: the file the pair would have written.
        title: SharedString,
    },
}

/// The lookups rudbgen's own tabs answer to.
///
/// An extension trait rather than inherent methods, because the strip itself is
/// [`ruui_shell::Pane`] and every one of these is a question about
/// [`PaneItem`]. Both are [`Pane::position`] with a predicate, which is the
/// hook the shell leaves for exactly this.
pub trait WorkTabs {
    /// The index of the tab editing `file`, if one is open here.
    ///
    /// Matched on the path rather than on the title: two templates of the same
    /// name in two directories are two documents, and one template reached
    /// through two names — a relative path and its absolute form — is one. The
    /// caller resolves the path before asking; see `rudbgen-core`'s rule for
    /// paths under the configuration directory (architecture document, §5).
    fn template_of(&self, file: &Path) -> Option<usize>;

    /// The index of the preview tab, if it is open.
    fn preview(&self) -> Option<usize>;
}

impl WorkTabs for Pane<PaneItem> {
    fn template_of(&self, file: &Path) -> Option<usize> {
        self.position(|item| match item {
            PaneItem::Template { file: open, .. } => open == file,
            PaneItem::Generate | PaneItem::Preview { .. } => false,
        })
    }

    fn preview(&self) -> Option<usize> {
        self.position(|item| matches!(item, PaneItem::Preview { .. }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// One template tab, named after its file.
    fn template(file: &str, dirty: bool) -> PaneItem {
        PaneItem::Template {
            file: PathBuf::from(file),
            title: SharedString::from(file.to_string()),
            dirty,
        }
    }

    #[test]
    fn a_pane_finds_one_template_by_its_file_and_the_preview_by_its_kind() {
        let mut pane = Pane::new();
        pane.push(template("model.java", false));
        pane.push(PaneItem::Generate);
        pane.push(template("mapper.xml", true));

        assert_eq!(pane.template_of(Path::new("model.java")), Some(0));
        assert_eq!(pane.template_of(Path::new("mapper.xml")), Some(2));
        // A template nobody has open, and the Generate tab, are not templates
        // of any file.
        assert_eq!(pane.template_of(Path::new("php.php")), None);
        assert_eq!(pane.preview(), None);

        pane.push(PaneItem::Preview {
            title: "T_ALBUM.java".into(),
        });
        assert_eq!(pane.preview(), Some(3));
    }
}
