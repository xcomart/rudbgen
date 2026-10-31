//! A template set packed into one file together with its templates' bodies —
//! what *Export set…* writes and *Import set…* reads.
//!
//! A [`TemplateSet`] names its templates by path, and a path means nothing on
//! another machine. A bundle carries the bodies instead, so a set can be handed
//! to a colleague, checked into a repository or moved to a new computer as one
//! file:
//!
//! ```json
//! {
//!   "format": "rudbgen-template-set",
//!   "version": 1,
//!   "name": "Java + MyBatis",
//!   "templates": [
//!     {
//!       "name": "Java Model",
//!       "file": "java_model.java",
//!       "out_template": "${name.suffix.pascal}.java",
//!       "selected": true,
//!       "content": "package ${package};\n…"
//!     }
//!   ],
//!   "variables": [["package", "com.example"]]
//! }
//! ```
//!
//! JSON, because a template is UTF-8 text by rule (the generator refuses
//! anything else), a bundle stays readable and diffable, and no archive format
//! has to be taken on for it. `variables` — the custom variables of the
//! connection it was exported from — is optional: a set's templates usually
//! read `${package}` and its like, and a set without them renders incomplete,
//! but whether to include them is the exporter's choice.
//!
//! `version` changes only when a reader of an older version would misread the
//! file; a field added alongside the others does not change it, and unknown
//! keys are ignored. A file carrying a newer version is refused rather than
//! half-read.
//!
//! Importing never overwrites a template file: the bodies land in a new
//! directory under `<config>/templates`, named after the set, with a number
//! added when that name is taken. The file names come from the bundle, so
//! they are cut down to a bare, portable name first — a bundle is a file from
//! elsewhere, and `../../.bashrc` is not a template name.

use std::collections::HashSet;
use std::fmt;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::paths::{strip_bom, write_atomic};
use crate::profile::TemplateRef;
use crate::template_sets::TemplateSet;

/// The value of a bundle's `format` key, which tells it from any other JSON.
pub const BUNDLE_FORMAT: &str = "rudbgen-template-set";

/// The ending a bundle's suggested file name carries.
///
/// Still `.json` at the end, so that any editor opens one.
pub const BUNDLE_EXTENSION: &str = "rudbgen-set.json";

/// The largest bundle file that is read.
///
/// Far above any real set — the built-in ones are a few kilobytes — and low
/// enough that picking a database dump by mistake is refused before it is
/// read into memory.
pub const MAX_BUNDLE_BYTES: u64 = 32 * 1024 * 1024;

/// A template set with its templates' bodies, as one bundle file holds it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TemplateBundle {
    /// Always [`BUNDLE_FORMAT`].
    pub format: String,
    /// Format version; see [`TemplateBundle::CURRENT_VERSION`].
    pub version: u32,
    /// The set's name.
    #[serde(default)]
    pub name: String,
    /// The templates, in the set's order.
    #[serde(default)]
    pub templates: Vec<BundledTemplate>,
    /// Custom variables to go with the set, in the order they were typed.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub variables: Vec<(String, String)>,
}

/// One template of a bundle: a [`TemplateRef`] with the file's body in place
/// of its path.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BundledTemplate {
    /// Name shown in the template list.
    #[serde(default)]
    pub name: String,
    /// The template file's own name, without any directory.
    #[serde(default)]
    pub file: String,
    /// Template rendered to get the output file name.
    #[serde(default)]
    pub out_template: String,
    /// Whether the set ships the template ticked.
    #[serde(default = "ticked")]
    pub selected: bool,
    /// The template's body.
    pub content: String,
}

/// A template missing its `selected` key is ticked, as a template added to a
/// list by hand is.
fn ticked() -> bool {
    true
}

/// Why a bundle could not be made, read or installed.
///
/// An enum rather than an `anyhow` chain because each case is a sentence the
/// application translates, naming the template or the file it is about.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BundleError {
    /// A template of the set being exported could not be read.
    ReadTemplate {
        /// The template's name in the list.
        template: String,
        /// The file that was opened.
        path: PathBuf,
        /// The system's reason.
        message: String,
    },
    /// A template of the set being exported is not UTF-8 text.
    NotUtf8 {
        /// The template's name in the list.
        template: String,
        /// The file that was read.
        path: PathBuf,
    },
    /// The file is not a bundle: not JSON, not tagged as one, or tagged but
    /// shaped wrong.
    NotABundle,
    /// The file was written by a newer rudbgen in a format this one cannot read.
    Newer {
        /// The version the file carries.
        version: u32,
    },
    /// The bundle holds no templates.
    Empty,
    /// The file is larger than [`MAX_BUNDLE_BYTES`].
    TooLarge,
    /// Reading the bundle or writing a file failed.
    Io {
        /// The file or directory the failure is about.
        path: PathBuf,
        /// The system's reason.
        message: String,
    },
}

impl fmt::Display for BundleError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ReadTemplate {
                template,
                path,
                message,
            } => write!(
                f,
                "template {template:?} could not be read from {}: {message}",
                path.display()
            ),
            Self::NotUtf8 { template, path } => write!(
                f,
                "template {template:?} in {} is not valid UTF-8",
                path.display()
            ),
            Self::NotABundle => f.write_str("the file is not a rudbgen template set"),
            Self::Newer { version } => write!(
                f,
                "the file is a version {version} template set, newer than this build reads"
            ),
            Self::Empty => f.write_str("the template set holds no templates"),
            Self::TooLarge => write!(
                f,
                "the file is larger than {} MiB",
                MAX_BUNDLE_BYTES / 1024 / 1024
            ),
            Self::Io { path, message } => write!(f, "{}: {message}", path.display()),
        }
    }
}

impl std::error::Error for BundleError {}

/// What [`TemplateBundle::install`] wrote.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Installed {
    /// The new directory the templates were written into.
    pub directory: PathBuf,
    /// The bundle's templates, each pointing at its file in
    /// [`Installed::directory`] by absolute path.
    pub templates: Vec<TemplateRef>,
}

impl TemplateBundle {
    /// Format version written by this build, and the newest one it reads.
    pub const CURRENT_VERSION: u32 = 1;

    /// Packs `set` with the bodies of its templates.
    ///
    /// `resolve` turns a stored template path into the file to open — the app
    /// layer's resolution, since a stored path may be relative to the
    /// configuration directory. A byte order mark is dropped, as the generator
    /// drops it.
    ///
    /// # Errors
    ///
    /// The first template that cannot be read, or that is not UTF-8, stops the
    /// packing and is named; a bundle missing a template would import as a set
    /// that silently lost it.
    pub fn pack(
        set: &TemplateSet,
        resolve: impl Fn(&Path) -> PathBuf,
        variables: Vec<(String, String)>,
    ) -> Result<Self, BundleError> {
        let mut templates = Vec::with_capacity(set.templates.len());
        for template in &set.templates {
            let path = resolve(&template.file);
            let bytes = fs::read(&path).map_err(|err| BundleError::ReadTemplate {
                template: template.name.clone(),
                path: path.clone(),
                message: err.to_string(),
            })?;
            let content = String::from_utf8(strip_bom(&bytes).to_vec()).map_err(|_| {
                BundleError::NotUtf8 {
                    template: template.name.clone(),
                    path: path.clone(),
                }
            })?;
            let file = path
                .file_name()
                .or_else(|| template.file.file_name())
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_default();
            templates.push(BundledTemplate {
                name: template.name.clone(),
                file,
                out_template: template.out_template.clone(),
                selected: template.selected,
                content,
            });
        }
        Ok(Self {
            format: BUNDLE_FORMAT.to_string(),
            version: Self::CURRENT_VERSION,
            name: set.name.clone(),
            templates,
            variables,
        })
    }

    /// The file name a save dialog should offer for this bundle.
    pub fn suggested_file_name(&self) -> String {
        format!(
            "{}.{BUNDLE_EXTENSION}",
            portable_name(&self.name, "template-set")
        )
    }

    /// The bundle as the file holds it: pretty-printed JSON, ending in a
    /// newline.
    pub fn to_json(&self) -> Vec<u8> {
        // Serializing strings, booleans and a number cannot fail.
        let mut json = serde_json::to_vec_pretty(self).unwrap_or_default();
        json.push(b'\n');
        json
    }

    /// Writes the bundle to `path`, atomically.
    ///
    /// # Errors
    ///
    /// Fails when the file cannot be written.
    pub fn write(&self, path: &Path) -> Result<(), BundleError> {
        write_atomic(path, &self.to_json()).map_err(|err| BundleError::Io {
            path: path.to_path_buf(),
            message: format!("{err:#}"),
        })
    }

    /// Reads a bundle file.
    ///
    /// # Errors
    ///
    /// Fails when the file cannot be read or is too large, and as
    /// [`TemplateBundle::parse`] does.
    pub fn read(path: &Path) -> Result<Self, BundleError> {
        let io_error = |err: io::Error| BundleError::Io {
            path: path.to_path_buf(),
            message: err.to_string(),
        };
        let size = fs::metadata(path).map_err(io_error)?.len();
        if size > MAX_BUNDLE_BYTES {
            return Err(BundleError::TooLarge);
        }
        Self::parse(&fs::read(path).map_err(io_error)?)
    }

    /// Parses a bundle from the bytes of its file.
    ///
    /// A UTF-8 byte order mark is tolerated. The `format` key is checked before
    /// anything else, so a theme or a `connections.json` picked by mistake is
    /// *not a template set* rather than a complaint about a missing field.
    ///
    /// # Errors
    ///
    /// [`BundleError::NotABundle`] for anything that is not a bundle,
    /// [`BundleError::Newer`] for one from a newer build, and
    /// [`BundleError::Empty`] for one with no templates in it.
    pub fn parse(bytes: &[u8]) -> Result<Self, BundleError> {
        let value: serde_json::Value =
            serde_json::from_slice(strip_bom(bytes)).map_err(|_| BundleError::NotABundle)?;
        if value.get("format").and_then(serde_json::Value::as_str) != Some(BUNDLE_FORMAT) {
            return Err(BundleError::NotABundle);
        }
        let version = value
            .get("version")
            .and_then(serde_json::Value::as_u64)
            .ok_or(BundleError::NotABundle)?;
        if version > u64::from(Self::CURRENT_VERSION) {
            return Err(BundleError::Newer {
                version: u32::try_from(version).unwrap_or(u32::MAX),
            });
        }
        let bundle: Self = serde_json::from_value(value).map_err(|_| BundleError::NotABundle)?;
        if bundle.templates.is_empty() {
            return Err(BundleError::Empty);
        }
        Ok(bundle)
    }

    /// The directory [`TemplateBundle::install`] would create under
    /// `templates_dir` if it ran now.
    ///
    /// For the import prompt to show; the install picks again, in case the
    /// name was taken in between.
    pub fn install_directory(&self, templates_dir: &Path) -> PathBuf {
        let base = portable_name(&self.name, "imported");
        (1..)
            .map(|n| templates_dir.join(numbered(&base, n)))
            .find(|candidate| !candidate.exists())
            .unwrap_or_else(|| templates_dir.join(base))
    }

    /// Writes the templates into a new directory under `templates_dir` and
    /// answers where they went.
    ///
    /// The directory is named after the set, with ` (2)`, ` (3)`… added while
    /// that name is taken, and created with a call that fails if it exists —
    /// so nothing already on disk is ever written over. File names are made
    /// portable and unique within the directory, ignoring case, since two
    /// names that differ only in case are one file on Windows and macOS.
    ///
    /// # Errors
    ///
    /// Fails when the directory cannot be created or a file cannot be written.
    /// The new directory is removed again then, so a failed import leaves
    /// nothing behind.
    pub fn install(&self, templates_dir: &Path) -> Result<Installed, BundleError> {
        fs::create_dir_all(templates_dir).map_err(|err| BundleError::Io {
            path: templates_dir.to_path_buf(),
            message: err.to_string(),
        })?;
        let base = portable_name(&self.name, "imported");
        let mut n = 1;
        let directory = loop {
            let candidate = templates_dir.join(numbered(&base, n));
            match fs::create_dir(&candidate) {
                Ok(()) => break candidate,
                Err(err) if err.kind() == io::ErrorKind::AlreadyExists => n += 1,
                Err(err) => {
                    return Err(BundleError::Io {
                        path: candidate,
                        message: err.to_string(),
                    });
                }
            }
        };

        let mut taken = HashSet::new();
        let mut templates = Vec::with_capacity(self.templates.len());
        for (index, template) in self.templates.iter().enumerate() {
            let fallback = format!("template{}.txt", index + 1);
            let wanted = if template.file.trim().is_empty() {
                portable_name(&template.name, &fallback)
            } else {
                portable_name(&template.file, &fallback)
            };
            let file = unique_file_name(&wanted, &mut taken);
            let path = directory.join(&file);
            if let Err(err) = write_atomic(&path, template.content.as_bytes()) {
                // Ours alone: created by this call, holding only what it wrote.
                let _ = fs::remove_dir_all(&directory);
                return Err(BundleError::Io {
                    path,
                    message: format!("{err:#}"),
                });
            }
            templates.push(TemplateRef {
                name: if template.name.trim().is_empty() {
                    file.clone()
                } else {
                    template.name.clone()
                },
                file: path,
                out_template: template.out_template.clone(),
                selected: template.selected,
            });
        }
        Ok(Installed {
            directory,
            templates,
        })
    }

    /// The bundle's variables that `existing` does not already name.
    ///
    /// Keys are compared trimmed, as the variable table saves them; a key the
    /// connection already has keeps its own value, and a key the bundle names
    /// twice is taken the first time, as the lookup would.
    pub fn new_variables(&self, existing: &[(String, String)]) -> Vec<(String, String)> {
        let mut seen: HashSet<String> = existing
            .iter()
            .map(|(key, _)| key.trim().to_string())
            .collect();
        self.variables
            .iter()
            .filter_map(|(key, value)| {
                let key = key.trim();
                (!key.is_empty() && seen.insert(key.to_string()))
                    .then(|| (key.to_string(), value.clone()))
            })
            .collect()
    }
}

/// Device names Windows reserves, whatever the extension after them.
const WINDOWS_DEVICES: [&str; 22] = [
    "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8",
    "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
];

/// `raw` cut down to one file or directory name that every platform accepts,
/// or `fallback` when nothing usable is left.
///
/// Only the last path component is kept, with both separators counted, so a
/// name cannot climb out of the directory it is written to. Characters
/// Windows refuses become `_`, trailing dots and spaces (which Windows drops)
/// go, and a reserved device name gets a `_` in front.
fn portable_name(raw: &str, fallback: &str) -> String {
    let last = raw.rsplit(['/', '\\']).next().unwrap_or_default();
    let replaced: String = last
        .chars()
        .map(|c| {
            if c.is_control() || matches!(c, '<' | '>' | ':' | '"' | '|' | '?' | '*') {
                '_'
            } else {
                c
            }
        })
        .collect();
    let name = replaced.trim().trim_end_matches(['.', ' ']);
    if name.is_empty() || name == "." || name == ".." {
        return fallback.to_string();
    }
    let stem = name.split('.').next().unwrap_or_default().trim_end();
    if WINDOWS_DEVICES
        .iter()
        .any(|device| device.eq_ignore_ascii_case(stem))
    {
        return format!("_{name}");
    }
    name.to_string()
}

/// `name` as the `n`th of its kind: itself first, then `name (2)` and on.
fn numbered(name: &str, n: usize) -> String {
    if n <= 1 {
        name.to_string()
    } else {
        format!("{name} ({n})")
    }
}

/// `wanted`, or the first `stem (n).ext` after it that `taken` does not hold
/// ignoring case; the answer is added to `taken`.
fn unique_file_name(wanted: &str, taken: &mut HashSet<String>) -> String {
    let (stem, extension) = match wanted.rfind('.') {
        Some(dot) if dot > 0 => wanted.split_at(dot),
        _ => (wanted, ""),
    };
    (1..)
        .map(|n| format!("{}{extension}", numbered(stem, n)))
        .find(|candidate| taken.insert(candidate.to_lowercase()))
        .unwrap_or_else(|| wanted.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn set_with(dir: &Path, files: &[(&str, &str)]) -> TemplateSet {
        let templates = files
            .iter()
            .map(|(file, content)| {
                let path = dir.join(file);
                fs::write(&path, content).unwrap();
                TemplateRef {
                    name: format!("{file} template"),
                    file: path,
                    out_template: format!("${{name}}.{file}"),
                    selected: true,
                }
            })
            .collect();
        TemplateSet::new("Java + MyBatis", templates)
    }

    #[test]
    fn a_packed_set_carries_every_body_and_reads_back_the_same() {
        let dir = tempfile::tempdir().unwrap();
        let set = set_with(
            dir.path(),
            &[("model.java", "class ${name} {}\n"), ("mapper.xml", "<x/>")],
        );
        let vars = vec![("package".to_string(), "com.example".to_string())];
        let bundle = TemplateBundle::pack(&set, Path::to_path_buf, vars.clone()).unwrap();
        assert_eq!(bundle.format, BUNDLE_FORMAT);
        assert_eq!(bundle.name, "Java + MyBatis");
        assert_eq!(bundle.templates.len(), 2);
        assert_eq!(bundle.templates[0].file, "model.java");
        assert_eq!(bundle.templates[0].content, "class ${name} {}\n");
        assert_eq!(bundle.variables, vars);

        let back = TemplateBundle::parse(&bundle.to_json()).unwrap();
        assert_eq!(back, bundle);
    }

    #[test]
    fn packing_resolves_the_stored_path_and_drops_a_bom() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("t.txt"), b"\xEF\xBB\xBFbody").unwrap();
        let set = TemplateSet::new(
            "s",
            vec![TemplateRef {
                name: "t".into(),
                file: PathBuf::from("t.txt"),
                out_template: String::new(),
                selected: false,
            }],
        );
        let root = dir.path().to_path_buf();
        let bundle = TemplateBundle::pack(&set, |file| root.join(file), Vec::new()).unwrap();
        assert_eq!(bundle.templates[0].content, "body");
        assert!(!bundle.templates[0].selected);
        // No variables, no key: a bundle without them reads as it was written.
        assert!(
            !String::from_utf8(bundle.to_json())
                .unwrap()
                .contains("variables")
        );
    }

    #[test]
    fn a_template_that_cannot_be_packed_is_named() {
        let dir = tempfile::tempdir().unwrap();
        let mut set = set_with(dir.path(), &[("ok.txt", "fine")]);
        set.templates.push(TemplateRef {
            name: "gone".into(),
            file: dir.path().join("missing.txt"),
            ..TemplateRef::default()
        });
        match TemplateBundle::pack(&set, Path::to_path_buf, Vec::new()) {
            Err(BundleError::ReadTemplate { template, .. }) => assert_eq!(template, "gone"),
            other => panic!("expected a read error, got {other:?}"),
        }

        fs::write(dir.path().join("latin1.txt"), b"caf\xE9").unwrap();
        let set = TemplateSet::new(
            "s",
            vec![TemplateRef {
                name: "latin1".into(),
                file: dir.path().join("latin1.txt"),
                ..TemplateRef::default()
            }],
        );
        match TemplateBundle::pack(&set, Path::to_path_buf, Vec::new()) {
            Err(BundleError::NotUtf8 { template, .. }) => assert_eq!(template, "latin1"),
            other => panic!("expected a UTF-8 error, got {other:?}"),
        }
    }

    #[test]
    fn only_a_tagged_file_of_a_known_version_with_templates_is_a_bundle() {
        assert_eq!(
            TemplateBundle::parse(b"not json"),
            Err(BundleError::NotABundle)
        );
        assert_eq!(
            TemplateBundle::parse(br#"{"version":1,"templates":[]}"#),
            Err(BundleError::NotABundle)
        );
        assert_eq!(
            TemplateBundle::parse(br#"{"format":"rudbgen-template-set","version":2}"#),
            Err(BundleError::Newer { version: 2 })
        );
        assert_eq!(
            TemplateBundle::parse(
                br#"{"format":"rudbgen-template-set","version":1,"templates":[]}"#
            ),
            Err(BundleError::Empty)
        );
        // A template without its body is shaped wrong, not empty.
        assert_eq!(
            TemplateBundle::parse(
                br#"{"format":"rudbgen-template-set","version":1,"templates":[{"name":"x"}]}"#
            ),
            Err(BundleError::NotABundle)
        );
    }

    #[test]
    fn a_hand_written_bundle_fills_the_rest_in_and_ignores_unknown_keys() {
        let bundle = TemplateBundle::parse(
            "\u{feff}{\"format\":\"rudbgen-template-set\",\"version\":1,\"future\":true,\
             \"templates\":[{\"content\":\"x\"}]}"
                .as_bytes(),
        )
        .unwrap();
        assert_eq!(bundle.name, "");
        assert!(bundle.templates[0].selected);
        assert_eq!(bundle.templates[0].file, "");
        assert!(bundle.variables.is_empty());
    }

    #[test]
    fn reading_refuses_a_file_too_large_to_be_a_set() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("big.json");
        let file = fs::File::create(&path).unwrap();
        file.set_len(MAX_BUNDLE_BYTES + 1).unwrap();
        assert_eq!(TemplateBundle::read(&path), Err(BundleError::TooLarge));
    }

    fn bundle(name: &str, files: &[&str]) -> TemplateBundle {
        TemplateBundle {
            format: BUNDLE_FORMAT.to_string(),
            version: 1,
            name: name.to_string(),
            templates: files
                .iter()
                .map(|file| BundledTemplate {
                    name: format!("{file}!"),
                    file: (*file).to_string(),
                    out_template: "${name}".to_string(),
                    selected: true,
                    content: format!("body of {file}"),
                })
                .collect(),
            variables: Vec::new(),
        }
    }

    #[test]
    fn installing_writes_a_new_directory_and_never_over_an_old_one() {
        let dir = tempfile::tempdir().unwrap();
        let first = bundle("Java", &["model.java"]).install(dir.path()).unwrap();
        assert_eq!(first.directory, dir.path().join("Java"));
        assert_eq!(
            fs::read_to_string(dir.path().join("Java/model.java")).unwrap(),
            "body of model.java"
        );
        assert_eq!(first.templates[0].file, dir.path().join("Java/model.java"));
        assert_eq!(first.templates[0].name, "model.java!");

        let preview = bundle("Java", &["model.java"]).install_directory(dir.path());
        assert_eq!(preview, dir.path().join("Java (2)"));
        let second = bundle("Java", &["model.java"]).install(dir.path()).unwrap();
        assert_eq!(second.directory, preview);
        assert_eq!(
            fs::read_to_string(dir.path().join("Java/model.java")).unwrap(),
            "body of model.java",
            "the first import's file is untouched"
        );
    }

    #[test]
    fn file_names_from_a_bundle_cannot_leave_the_directory() {
        let dir = tempfile::tempdir().unwrap();
        let installed = bundle(
            "../../escape",
            &["../../.bashrc", "..\\..\\evil.txt", "", "a/b/c.txt"],
        )
        .install(dir.path())
        .unwrap();
        assert_eq!(installed.directory, dir.path().join("escape"));
        let names: Vec<_> = installed
            .templates
            .iter()
            .map(|t| {
                t.file
                    .strip_prefix(&installed.directory)
                    .unwrap()
                    .to_path_buf()
            })
            .collect();
        assert_eq!(
            names,
            [
                PathBuf::from(".bashrc"),
                PathBuf::from("evil.txt"),
                // No file name: the template's own name stands in.
                PathBuf::from("!"),
                PathBuf::from("c.txt"),
            ]
        );
    }

    #[test]
    fn names_are_made_portable_and_unique_ignoring_case() {
        assert_eq!(portable_name("a:b?.java", "x"), "a_b_.java");
        assert_eq!(portable_name("Model. ", "x"), "Model");
        assert_eq!(portable_name("con.java", "x"), "_con.java");
        assert_eq!(portable_name("Com1", "x"), "_Com1");
        assert_eq!(portable_name("..", "x"), "x");
        assert_eq!(portable_name("   ", "x"), "x");

        let mut taken = HashSet::new();
        assert_eq!(unique_file_name("Model.java", &mut taken), "Model.java");
        assert_eq!(unique_file_name("model.java", &mut taken), "model (2).java");
        assert_eq!(unique_file_name("MODEL.java", &mut taken), "MODEL (3).java");
        assert_eq!(unique_file_name(".env", &mut taken), ".env");
        assert_eq!(unique_file_name(".env", &mut taken), ".env (2)");
    }

    #[test]
    fn a_failed_install_leaves_nothing_behind() {
        let dir = tempfile::tempdir().unwrap();
        // Longer than any filesystem allows a name to be, so the second write
        // fails after the first has landed.
        let mut broken = bundle("Set", &["one.txt", "two.txt"]);
        broken.templates[1].file = format!("{}.txt", "a".repeat(300));
        assert!(matches!(
            broken.install(dir.path()),
            Err(BundleError::Io { .. })
        ));
        assert!(!dir.path().join("Set").exists());
    }

    #[test]
    fn only_variables_the_connection_lacks_are_added() {
        let mut bundle = bundle("s", &["a"]);
        bundle.variables = vec![
            ("package".into(), "com.other".into()),
            (" module ".into(), "core".into()),
            ("module".into(), "second".into()),
            ("  ".into(), "blank".into()),
        ];
        let existing = vec![("package".to_string(), "com.mine".to_string())];
        assert_eq!(
            bundle.new_variables(&existing),
            vec![("module".to_string(), "core".to_string())]
        );
    }

    #[test]
    fn the_suggested_file_name_is_the_set_name_made_portable() {
        let bundle = bundle("Java: MyBatis", &["a"]);
        assert_eq!(
            bundle.suggested_file_name(),
            "Java_ MyBatis.rudbgen-set.json"
        );
        assert_eq!(
            TemplateBundle {
                name: String::new(),
                ..bundle
            }
            .suggested_file_name(),
            "template-set.rudbgen-set.json"
        );
    }
}
