//! The three templates rudbgen ships, and the two sets that name them.
//!
//! jdbgen's `templates/*.{java,xml,php}` are the built-in set (architecture
//! document, §2.2). They are compiled into the binary with
//! [`include_bytes!`] — bytes and not `str`, because their CRLF line endings
//! are load bearing: the engine takes a template's line ending from its first
//! newline, so a copy normalised to LF would silently change the line endings
//! of every file generated from it.
//!
//! Two rules run through everything here, and both are about not arguing with
//! the user:
//!
//! * a template file that already exists in the configuration directory is
//!   **never** overwritten (§5), so an edit of a shipped template survives an
//!   update;
//! * the built-in sets are offered **once**. [`TemplateSetStore::builtins_seeded`]
//!   records that they were, so a user who deletes "Java + MyBatis" is not
//!   handed it back on the next launch.

use std::io;
use std::path::{Path, PathBuf};

use rudbgen_core::{TemplateRef, TemplateSet, TemplateSetStore};
use uuid::{Uuid, uuid};

/// One shipped template: the file name it is written under, its bytes, the
/// name the template list shows, and the output name it renders through.
pub struct Builtin {
    /// File name inside `<config>/templates`.
    pub file: &'static str,
    /// The template itself, byte for byte as jdbgen ships it.
    pub bytes: &'static [u8],
    /// What the template list calls it.
    ///
    /// Not translated: it is a name the user may rename, and one that is
    /// written into `connections.json` the moment a set is applied — a label
    /// that changed with the interface language would leave a profile carrying
    /// a name from whichever language it was saved in.
    pub name: &'static str,
    /// jdbgen's default output name template.
    pub out_template: &'static str,
}

/// The three templates, in the order the built-in sets list them.
pub const BUILTINS: [Builtin; 3] = [
    Builtin {
        file: "java_model.java",
        bytes: include_bytes!("../../../templates/java_model.java"),
        name: "Java Model",
        out_template: "${name.suffix.pascal}Model.java",
    },
    Builtin {
        file: "mybatis_mapper.xml",
        bytes: include_bytes!("../../../templates/mybatis_mapper.xml"),
        name: "MyBatis Mapper",
        out_template: "${name.suffix.camel}-mapper.xml",
    },
    Builtin {
        file: "php_ci.php",
        bytes: include_bytes!("../../../templates/php_ci.php"),
        name: "PHP CodeIgniter",
        out_template: "${name.suffix.lower}_ci_model.php",
    },
];

/// Identifier of the built-in "Java + MyBatis" set.
///
/// Fixed rather than generated, so that a store seeded by one build and read by
/// the next recognises the same set — and so that a user who has renamed it
/// still has the set they renamed rather than a duplicate beside it.
pub const JAVA_MYBATIS: Uuid = uuid!("2f0d2f5e-6c1a-4c2e-9a1f-3d3f5b7c9e01");

/// Identifier of the built-in "PHP CodeIgniter" set.
pub const PHP_CI: Uuid = uuid!("2f0d2f5e-6c1a-4c2e-9a1f-3d3f5b7c9e02");

/// Name of the directory the templates are copied into, under the config
/// directory.
///
/// The same segment [`rudbgen_core::templates_dir`] appends; spelled again here
/// because this module is what writes the *relative* paths a profile stores
/// (§5), and those are relative to the configuration directory rather than to
/// the template directory.
pub const TEMPLATES_DIR: &str = "templates";

/// Copies the shipped templates into `dir`, never over one that is there.
///
/// Answers the number of files it actually wrote, which is three on a first run
/// and zero on every run after it.
///
/// # Errors
///
/// Fails when the directory cannot be created or a file cannot be written. A
/// caller that cannot do anything about it logs it: the application is still
/// usable, with a template list the user fills in themselves.
pub fn install(dir: &Path) -> io::Result<usize> {
    std::fs::create_dir_all(dir)?;
    let mut written = 0;
    for builtin in &BUILTINS {
        let path = dir.join(builtin.file);
        if path.exists() {
            continue;
        }
        std::fs::write(&path, builtin.bytes)?;
        written += 1;
    }
    Ok(written)
}

/// The path a profile stores for one shipped template.
///
/// Relative to the configuration directory, which is §5's rule for a path below
/// it; [`crate::generate_pane::resolve_template`] is what turns it back into
/// something the generator can open.
fn builtin_path(builtin: &Builtin) -> PathBuf {
    Path::new(TEMPLATES_DIR).join(builtin.file)
}

/// One template row of a built-in set, ticked.
fn reference(builtin: &Builtin) -> TemplateRef {
    TemplateRef {
        name: builtin.name.to_string(),
        file: builtin_path(builtin),
        out_template: builtin.out_template.to_string(),
        selected: true,
    }
}

/// The two sets the application ships: `Java + MyBatis` and `PHP CodeIgniter`.
pub fn sets() -> Vec<TemplateSet> {
    vec![
        TemplateSet {
            id: JAVA_MYBATIS,
            name: "Java + MyBatis".to_string(),
            templates: vec![reference(&BUILTINS[0]), reference(&BUILTINS[1])],
        },
        TemplateSet {
            id: PHP_CI,
            name: "PHP CodeIgniter".to_string(),
            templates: vec![reference(&BUILTINS[2])],
        },
    ]
}

/// Puts the built-in sets into `store`, once ever.
///
/// Answers whether it changed anything, which is what tells the caller to
/// write the file. Both conditions have to hold: the store must be empty — a
/// store with sets of the user's own is not a first run whatever the flag says
/// — and the flag must be clear.
pub fn seed(store: &mut TemplateSetStore) -> bool {
    if store.builtins_seeded || !store.sets.is_empty() {
        // Still worth recording: a store that already carries sets has plainly
        // been used, and offering the built-ins the day it is emptied would be
        // the same surprise the flag exists to prevent.
        if !store.builtins_seeded && !store.sets.is_empty() {
            store.builtins_seeded = true;
            return true;
        }
        return false;
    }
    store.sets = sets();
    store.builtins_seeded = true;
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_shipped_templates_keep_their_line_endings() {
        // The engine reads a template's line ending from its first newline, so
        // a copy normalised to LF would change every file generated from it.
        // `.gitattributes` marks `templates/**` as binary for this reason;
        // this is the assertion that says so out loud.
        for builtin in &BUILTINS {
            assert!(
                builtin.bytes.windows(2).any(|pair| pair == b"\r\n"),
                "{} lost its CRLF line endings",
                builtin.file
            );
            assert!(
                !builtin.bytes.is_empty(),
                "{} is empty — is the file still there?",
                builtin.file
            );
        }
    }

    #[test]
    fn installing_never_writes_over_an_edited_template() {
        let dir = tempfile::tempdir().expect("tempdir");
        let root = dir.path().join("templates");

        assert_eq!(install(&root).expect("first run"), BUILTINS.len());
        for builtin in &BUILTINS {
            let written = std::fs::read(root.join(builtin.file)).expect("read");
            assert_eq!(written, builtin.bytes, "{} was rewritten", builtin.file);
        }

        // The user edits one and the application is launched again.
        std::fs::write(root.join(BUILTINS[0].file), b"mine\r\n").expect("write");
        assert_eq!(install(&root).expect("second run"), 0);
        assert_eq!(
            std::fs::read(root.join(BUILTINS[0].file)).expect("read"),
            b"mine\r\n",
            "an edited template was overwritten"
        );
    }

    #[test]
    fn the_built_in_sets_are_offered_once_and_never_again() {
        let mut store = TemplateSetStore::default();
        assert!(seed(&mut store));
        assert_eq!(store.sets.len(), 2);
        assert!(store.builtins_seeded);
        assert_eq!(store.sets[0].id, JAVA_MYBATIS);
        assert_eq!(store.sets[1].id, PHP_CI);

        // Nothing more to do on the next launch.
        assert!(!seed(&mut store));

        // The user deletes both, and they stay deleted.
        store.sets.clear();
        assert!(!seed(&mut store));
        assert!(store.sets.is_empty());
    }

    #[test]
    fn a_store_with_sets_of_its_own_is_marked_rather_than_seeded() {
        // A store written before the flag existed, carrying a set the user
        // made: the built-ins are not pushed into it, and emptying it later
        // must not bring them back either.
        let mut store = TemplateSetStore {
            sets: vec![TemplateSet::new("mine", Vec::new())],
            ..TemplateSetStore::default()
        };
        assert!(seed(&mut store), "the flag has to be written down");
        assert_eq!(store.sets.len(), 1, "the built-ins were pushed in anyway");
        assert!(store.builtins_seeded);

        store.sets.clear();
        assert!(!seed(&mut store));
        assert!(store.sets.is_empty());
    }

    #[test]
    fn every_built_in_set_names_a_template_the_installer_writes() {
        let files: Vec<PathBuf> = BUILTINS.iter().map(builtin_path).collect();
        for set in sets() {
            assert!(!set.templates.is_empty(), "{} is empty", set.name);
            for template in &set.templates {
                assert!(
                    files.contains(&template.file),
                    "{} names {}, which is not shipped",
                    set.name,
                    template.file.display()
                );
                assert!(template.selected, "a built-in set ships an unticked row");
                assert!(!template.out_template.is_empty());
                assert!(
                    template.file.is_relative(),
                    "a shipped path has to be relative to the config directory"
                );
            }
        }
    }
}
