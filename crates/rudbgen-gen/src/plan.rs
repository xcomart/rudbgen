//! Everything a run needs, decided before it starts.

use std::path::{Path, PathBuf};

use rudbgen_core::{AbbreviationStore, GenerationProfile, TemplateRef};
use rudbgen_meta::Table;
use rudbgen_template::chrono::NaiveDateTime;
use rudbgen_template::{AbbrRule, Abbreviations, RenderContext};

use crate::error::Error;

/// One template of a run: a body and the name of what it renders to.
///
/// A template is two templates. The body lives in a file the user edits; the
/// output name lives in the profile beside it and is rendered per table with
/// the same engine, so `${package.replace('.','/')}/${name.suffix.pascal}.java`
/// creates directories.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TemplateSpec {
    /// What the template list calls it, and what a message names.
    pub name: String,
    /// The file the body is read from.
    pub file: PathBuf,
    /// The template rendered to get the output file name.
    pub out_template: String,
    /// The body, when the caller already has it.
    ///
    /// This is how the editor previews a tab it has not saved yet: with a
    /// `Some` here the file is never read, and [`TemplateSpec::file`] is only
    /// a label for messages.
    pub source: Option<String>,
}

impl TemplateSpec {
    /// A template read from a file.
    pub fn new(
        name: impl Into<String>,
        file: impl Into<PathBuf>,
        out_template: impl Into<String>,
    ) -> Self {
        TemplateSpec {
            name: name.into(),
            file: file.into(),
            out_template: out_template.into(),
            source: None,
        }
    }

    /// Use `source` as the body instead of reading the file.
    pub fn with_source(mut self, source: impl Into<String>) -> Self {
        self.source = Some(source.into());
        self
    }

    /// The saved template reference, whether it is ticked or not.
    pub fn from_ref(template: &TemplateRef) -> Self {
        TemplateSpec::new(&template.name, &template.file, &template.out_template)
    }
}

/// A whole generation run, decided.
///
/// The tables arrive **loaded**: reading them needs a JDBC session, and this
/// crate is deliberately usable without one (architecture document, §3). The
/// clock and the login user are fields rather than calls so that a preview
/// does not change while the user reads it, and so that a test renders the
/// same bytes twice.
#[derive(Clone, Debug)]
pub struct Plan {
    /// The tables the user ticked, in the order the files should be written.
    pub tables: Vec<Table>,
    /// The templates the user ticked.
    pub templates: Vec<TemplateSpec>,
    /// Where the rendered files go. Every output name is resolved against it
    /// and none may escape it.
    pub output_dir: PathBuf,
    /// The `${author}` of the run; see the crate documentation, rule 8.
    pub author: String,
    /// The custom variables, in the order the user typed them. A key repeated
    /// on disk is resolved first-wins, which is
    /// [`GenerationProfile::custom_vars`]' documented rule.
    pub custom_vars: Vec<(String, String)>,
    /// The dictionary the `abbr` processor looks names up in.
    pub abbreviations: Abbreviations,
    /// Whether a leading `name` abbreviates by itself.
    pub apply_abbr: bool,
    /// What `${user}` renders as.
    pub user: String,
    /// What `${date}` formats.
    pub now: NaiveDateTime,
}

impl Plan {
    /// A run of `templates` over `tables`, into `output_dir`.
    ///
    /// Everything else starts empty, except the clock and the user, which
    /// start at the machine's.
    pub fn new(
        tables: Vec<Table>,
        templates: Vec<TemplateSpec>,
        output_dir: impl Into<PathBuf>,
    ) -> Self {
        // `RenderContext::new()` is where the login user is worked out; asking
        // it here keeps that rule in the engine, which is the crate that has
        // to match jdbgen's `${user}`.
        let defaults = RenderContext::new();
        Plan {
            tables,
            templates,
            output_dir: output_dir.into(),
            author: String::new(),
            custom_vars: Vec::new(),
            abbreviations: Abbreviations::new(),
            apply_abbr: false,
            user: defaults.user,
            now: defaults.now,
        }
    }

    /// The run a saved [`GenerationProfile`] describes: its **ticked**
    /// templates, its output directory, its author and its variables.
    ///
    /// The abbreviation dictionary is not part of the profile — it is one
    /// global store (see [`abbreviations_of`]) — so a caller that wants it
    /// adds it afterwards.
    ///
    /// # Errors
    ///
    /// [`Error::NoOutputDir`] when the profile names no output directory. A
    /// relative one is left relative: resolving it is the application's rule,
    /// not this crate's.
    pub fn from_profile(profile: &GenerationProfile, tables: Vec<Table>) -> Result<Self, Error> {
        let output_dir = profile.output_dir.clone().ok_or(Error::NoOutputDir)?;
        let templates = profile
            .templates
            .iter()
            .filter(|template| template.selected)
            .map(TemplateSpec::from_ref)
            .collect();
        Ok(Plan {
            author: profile.author.clone(),
            custom_vars: profile.custom_vars.clone(),
            ..Plan::new(tables, templates, output_dir)
        })
    }

    /// Take the dictionary and the switch from the saved rules.
    pub fn with_abbreviations(mut self, store: &AbbreviationStore) -> Self {
        self.abbreviations = abbreviations_of(store);
        self.apply_abbr = store.apply_to_names;
        self
    }

    /// Pin the clock and the user, which is what a test and a preview want.
    pub fn with_clock(mut self, now: NaiveDateTime, user: impl Into<String>) -> Self {
        self.now = now;
        self.user = user.into();
        self
    }

    /// How many files this run would write if nothing were in its way.
    pub fn total(&self) -> usize {
        self.tables.len() * self.templates.len()
    }

    /// The context every render of this run uses.
    ///
    /// Public because the editor's live preview renders through the engine
    /// directly and has to do it with the run's variables, or the preview is
    /// not a preview of the run.
    ///
    /// The custom variables go in first-wins, and a non-empty
    /// [`Plan::author`] is then written over the `author` key: the form field
    /// beats the variable table (rule 8). An empty author leaves the variable
    /// alone, because a field nobody filled in must not erase a value
    /// somebody typed.
    pub fn context(&self) -> RenderContext {
        let mut ctx = RenderContext::new()
            .with_abbreviations(self.abbreviations.clone())
            .with_apply_abbr(self.apply_abbr)
            .with_now(self.now)
            .with_user(self.user.clone());
        for (key, value) in &self.custom_vars {
            ctx.custom_vars
                .entry(key.clone())
                .or_insert_with(|| value.clone());
        }
        if !self.author.is_empty() {
            ctx.custom_vars
                .insert("author".to_string(), self.author.clone());
        }
        ctx
    }

    /// The output directory, for messages.
    pub(crate) fn output_dir(&self) -> &Path {
        &self.output_dir
    }
}

/// The engine's dictionary, built from the saved rules.
///
/// The two types say the same thing in two vocabularies — the store is what
/// `abbreviations.json` holds, the dictionary is what the `abbr` processor
/// looks a name up in — and this is the one place that translates. Rules that
/// are switched off are dropped by
/// [`Abbreviations::from_rules`](rudbgen_template::Abbreviations::from_rules);
/// whole-name rules land in their own table, where one of them ends the
/// replacement rather than rewriting a word of it.
pub fn abbreviations_of(store: &AbbreviationStore) -> Abbreviations {
    Abbreviations::from_rules(store.rules.iter().map(|rule| AbbrRule {
        enabled: rule.enabled,
        whole_name: rule.whole_name,
        abbr: rule.abbreviation.clone(),
        replace_to: rule.replacement.clone(),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use rudbgen_core::AbbreviationRule;

    fn store(rules: Vec<AbbreviationRule>) -> AbbreviationStore {
        AbbreviationStore {
            apply_to_names: true,
            rules,
            ..AbbreviationStore::default()
        }
    }

    fn rule(abbreviation: &str, replacement: &str) -> AbbreviationRule {
        AbbreviationRule {
            enabled: true,
            whole_name: false,
            abbreviation: abbreviation.to_string(),
            replacement: replacement.to_string(),
        }
    }

    #[test]
    fn the_form_author_beats_a_variable_of_the_same_name() {
        let mut plan = Plan::new(Vec::new(), Vec::new(), "/out");
        plan.custom_vars = vec![("author".to_string(), "from the table".to_string())];
        plan.author = "from the form".to_string();
        assert_eq!(
            plan.context().custom_vars.get("author").map(String::as_str),
            Some("from the form")
        );
    }

    #[test]
    fn an_empty_author_leaves_the_variable_alone() {
        let mut plan = Plan::new(Vec::new(), Vec::new(), "/out");
        plan.custom_vars = vec![("author".to_string(), "from the table".to_string())];
        assert_eq!(
            plan.context().custom_vars.get("author").map(String::as_str),
            Some("from the table")
        );
    }

    #[test]
    fn a_repeated_variable_is_resolved_first_wins() {
        let mut plan = Plan::new(Vec::new(), Vec::new(), "/out");
        plan.custom_vars = vec![
            ("package".to_string(), "com.first".to_string()),
            ("package".to_string(), "com.second".to_string()),
        ];
        assert_eq!(
            plan.context()
                .custom_vars
                .get("package")
                .map(String::as_str),
            Some("com.first")
        );
    }

    #[test]
    fn only_the_ticked_templates_take_part() {
        let profile = GenerationProfile {
            templates: vec![
                TemplateRef {
                    name: "on".to_string(),
                    file: PathBuf::from("on.java"),
                    out_template: "${name}.java".to_string(),
                    selected: true,
                },
                TemplateRef {
                    name: "off".to_string(),
                    file: PathBuf::from("off.java"),
                    out_template: "${name}.txt".to_string(),
                    selected: false,
                },
            ],
            output_dir: Some(PathBuf::from("/out")),
            author: "comart".to_string(),
            custom_vars: vec![("package".to_string(), "com.abc".to_string())],
        };

        let plan = Plan::from_profile(&profile, Vec::new()).expect("a profile with an output dir");
        assert_eq!(
            plan.templates.iter().map(|t| &t.name).collect::<Vec<_>>(),
            ["on"]
        );
        assert_eq!(plan.author, "comart");
        assert_eq!(plan.output_dir, PathBuf::from("/out"));
    }

    #[test]
    fn a_profile_without_an_output_directory_is_no_plan() {
        let profile = GenerationProfile::default();
        assert_eq!(
            Plan::from_profile(&profile, Vec::new()).unwrap_err(),
            Error::NoOutputDir
        );
    }

    #[test]
    fn the_saved_rules_become_the_dictionary_the_engine_uses() {
        let dict = abbreviations_of(&store(vec![
            rule("usr", "user"),
            AbbreviationRule {
                enabled: false,
                ..rule("acct", "account")
            },
            AbbreviationRule {
                whole_name: true,
                ..rule("TB_SYS", "System")
            },
        ]));

        // D10: a word rule fires on an upper-case identifier.
        assert_eq!(dict.apply("TB_USR"), "TB_user");
        assert_eq!(dict.apply("TB_ACCT"), "TB_ACCT", "a rule that is off");
        assert_eq!(dict.apply("tb_sys"), "System", "a whole-name rule wins");
    }

    #[test]
    fn the_switch_travels_with_the_dictionary() {
        let plan = Plan::new(Vec::new(), Vec::new(), "/out")
            .with_abbreviations(&store(vec![rule("usr", "user")]));
        assert!(plan.apply_abbr);
        assert!(!plan.abbreviations.is_empty());

        let off = Plan::new(Vec::new(), Vec::new(), "/out")
            .with_abbreviations(&AbbreviationStore::default());
        assert!(!off.apply_abbr);
        assert!(off.abbreviations.is_empty());
    }
}
