//! The rules of §9, each in a temporary directory.

mod common;

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use common::{Events, at, live, plan, read, table, template, tree};
use rudbgen_core::{AbbreviationRule, AbbreviationStore, OverwritePolicy};
use rudbgen_gen::{
    CancelToken, Decision, FileStatus, Overwrite, Progress, SkipReason, TemplatePart, TemplateSpec,
    dry_run, generate, preview,
};
use tempfile::TempDir;

/// A run of two tables × one template, and the directories it lives in.
struct Fixture {
    _dir: TempDir,
    templates: PathBuf,
    out: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let dir = tempfile::tempdir().expect("tempdir");
        let templates = dir.path().join("templates");
        let out = dir.path().join("out");
        std::fs::create_dir_all(&templates).expect("templates dir");
        Fixture {
            _dir: dir,
            templates,
            out,
        }
    }
}

fn quiet(_: Progress) {}

// --- rule 1: a parse error writes nothing -------------------------------

#[test]
fn one_broken_template_writes_no_file_at_all() {
    let fx = Fixture::new();
    let plan = plan(
        vec![table("T_ONE"), table("T_TWO")],
        vec![
            template(
                &fx.templates,
                "good.java",
                "class ${name} {}",
                "${name}.java",
            ),
            template(
                &fx.templates,
                "bad.java",
                "line one\nclass ${name\n",
                "${name}.txt",
            ),
        ],
        &fx.out,
    );

    let outcome = generate(&plan, Overwrite::Overwrite, &live(), &quiet);

    assert!(outcome.written.is_empty(), "{:?}", outcome.written);
    assert!(
        !fx.out.exists(),
        "the output directory was not even created"
    );
    assert_eq!(outcome.failed.len(), 1);
    let failure = &outcome.failed[0];
    assert_eq!(failure.template, "bad.java");
    assert_eq!(failure.table, None, "a parse error belongs to no table");
    assert!(
        failure.message.contains("line 2"),
        "the line is missing from {:?}",
        failure.message
    );
    assert!(
        failure.message.contains("template body"),
        "the part is missing from {:?}",
        failure.message
    );
}

#[test]
fn a_broken_output_name_is_named_as_such() {
    let fx = Fixture::new();
    let plan = plan(
        vec![table("T_ONE")],
        vec![template(
            &fx.templates,
            "one.java",
            "class ${name} {}",
            "${name.java",
        )],
        &fx.out,
    );

    let outcome = generate(&plan, Overwrite::Overwrite, &live(), &quiet);
    assert!(outcome.written.is_empty());
    assert!(
        outcome.failed[0].message.contains("output name"),
        "{:?}",
        outcome.failed[0].message
    );
}

#[test]
fn every_broken_template_is_reported_not_only_the_first() {
    let fx = Fixture::new();
    let plan = plan(
        vec![table("T_ONE")],
        vec![
            template(&fx.templates, "a.java", "${name", "${name}.java"),
            template(
                &fx.templates,
                "b.java",
                "${for:item=columns}",
                "${name}.java",
            ),
        ],
        &fx.out,
    );

    let outcome = generate(&plan, Overwrite::Overwrite, &live(), &quiet);
    assert_eq!(outcome.failed.len(), 2, "{:?}", outcome.failed);
}

#[test]
fn a_template_file_that_is_not_there_stops_the_run() {
    let fx = Fixture::new();
    let plan = plan(
        vec![table("T_ONE")],
        vec![TemplateSpec::new(
            "absent",
            fx.templates.join("absent.java"),
            "${name}.java",
        )],
        &fx.out,
    );

    let outcome = generate(&plan, Overwrite::Overwrite, &live(), &quiet);
    assert!(outcome.written.is_empty());
    assert!(
        outcome.failed[0].message.contains("cannot read template"),
        "{:?}",
        outcome.failed[0].message
    );
}

#[test]
fn a_template_file_that_is_not_utf8_stops_the_run() {
    let fx = Fixture::new();
    let path = fx.templates.join("latin1.java");
    std::fs::write(&path, b"class \xE0\xA4 {}").expect("write");
    let plan = plan(
        vec![table("T_ONE")],
        vec![TemplateSpec::new("latin1", path, "${name}.java")],
        &fx.out,
    );

    let outcome = generate(&plan, Overwrite::Overwrite, &live(), &quiet);
    assert!(outcome.written.is_empty());
    assert!(
        outcome.failed[0].message.contains("not valid UTF-8"),
        "{:?}",
        outcome.failed[0].message
    );
}

#[test]
fn an_unsaved_editor_buffer_is_used_instead_of_the_file() {
    let fx = Fixture::new();
    let spec =
        template(&fx.templates, "one.java", "on disk", "${name}.java").with_source("in the editor");
    let plan = plan(vec![table("T_ONE")], vec![spec], &fx.out);

    generate(&plan, Overwrite::Overwrite, &live(), &quiet);
    assert_eq!(read(&fx.out, "T_ONE.java"), "in the editor");
}

// --- rule 2: the output name ---------------------------------------------

#[test]
fn an_output_name_with_directories_in_it_creates_them() {
    let fx = Fixture::new();
    let mut plan = plan(
        vec![table("T_USER_ACCOUNT")],
        vec![template(
            &fx.templates,
            "model.java",
            "class ${name.pascal} {}",
            "${package.replace('.','/')}/${name.pascal}.java",
        )],
        &fx.out,
    );
    plan.custom_vars = vec![("package".to_string(), "com.abc.sample".to_string())];

    let outcome = generate(&plan, Overwrite::Overwrite, &live(), &quiet);

    assert!(outcome.is_ok(), "{outcome:?}");
    assert_eq!(tree(&fx.out), ["com/abc/sample/TUserAccount.java"]);
    assert_eq!(
        outcome.written,
        vec![at(&fx.out, "com/abc/sample/TUserAccount.java")]
    );
}

#[test]
fn an_output_name_that_leaves_the_output_directory_is_refused() {
    let fx = Fixture::new();
    std::fs::create_dir_all(&fx.out).expect("out dir");
    let plan = plan(
        vec![table("T_ONE"), table("T_TWO")],
        vec![template(
            &fx.templates,
            "escape.java",
            "class ${name} {}",
            "../${name}.java",
        )],
        &fx.out,
    );

    let outcome = generate(&plan, Overwrite::Overwrite, &live(), &quiet);

    assert!(outcome.written.is_empty());
    assert_eq!(outcome.failed.len(), 2, "one per table");
    assert_eq!(outcome.failed[0].table.as_deref(), Some("T_ONE"));
    assert_eq!(outcome.failed[0].path, None);
    assert!(
        outcome.failed[0]
            .message
            .contains("leaves the output directory"),
        "{:?}",
        outcome.failed[0].message
    );
    assert!(tree(&fx.out).is_empty(), "nothing was written anywhere");
    assert!(
        !fx.out.parent().unwrap().join("T_ONE.java").exists(),
        "the escape landed outside"
    );
}

#[test]
fn an_absolute_output_name_is_refused() {
    let fx = Fixture::new();
    let absolute = if cfg!(windows) {
        "C:/tmp/x.java"
    } else {
        "/tmp/x.java"
    };
    let plan = plan(
        vec![table("T_ONE")],
        vec![template(
            &fx.templates,
            "abs.java",
            "class ${name} {}",
            absolute,
        )],
        &fx.out,
    );

    let outcome = generate(&plan, Overwrite::Overwrite, &live(), &quiet);
    assert!(outcome.written.is_empty());
    assert!(
        outcome.failed[0].message.contains("absolute path"),
        "{:?}",
        outcome.failed[0].message
    );
}

#[test]
fn an_output_name_that_renders_empty_is_refused() {
    let fx = Fixture::new();
    let plan = plan(
        vec![table("T_ONE")],
        vec![template(
            &fx.templates,
            "empty.java",
            "class ${name} {}",
            "${nosuchfield}",
        )],
        &fx.out,
    );

    let outcome = generate(&plan, Overwrite::Overwrite, &live(), &quiet);
    assert!(outcome.written.is_empty());
    assert!(
        outcome.failed[0].message.contains("rendered empty"),
        "{:?}",
        outcome.failed[0].message
    );
}

// --- the three policies ---------------------------------------------------

#[test]
fn the_overwrite_policy_replaces_what_is_there() {
    let fx = Fixture::new();
    std::fs::create_dir_all(&fx.out).expect("out dir");
    std::fs::write(at(&fx.out, "T_ONE.java"), "hand written").expect("existing file");

    let plan = plan(
        vec![table("T_ONE")],
        vec![template(
            &fx.templates,
            "one.java",
            "class ${name} {}",
            "${name}.java",
        )],
        &fx.out,
    );

    let outcome = generate(&plan, Overwrite::Overwrite, &live(), &quiet);

    assert!(outcome.is_ok(), "{outcome:?}");
    assert_eq!(read(&fx.out, "T_ONE.java"), "class T_ONE {}");
    assert!(outcome.skipped.is_empty());
}

#[test]
fn the_skip_policy_leaves_what_is_there_and_writes_the_rest() {
    let fx = Fixture::new();
    std::fs::create_dir_all(&fx.out).expect("out dir");
    std::fs::write(at(&fx.out, "T_ONE.java"), "hand written").expect("existing file");

    let plan = plan(
        vec![table("T_ONE"), table("T_TWO")],
        vec![template(
            &fx.templates,
            "one.java",
            "class ${name} {}",
            "${name}.java",
        )],
        &fx.out,
    );

    let outcome = generate(&plan, Overwrite::Skip, &live(), &quiet);

    assert_eq!(read(&fx.out, "T_ONE.java"), "hand written");
    assert_eq!(read(&fx.out, "T_TWO.java"), "class T_TWO {}");
    assert_eq!(outcome.written, vec![at(&fx.out, "T_TWO.java")]);
    assert_eq!(outcome.skipped.len(), 1);
    assert_eq!(outcome.skipped[0].reason, SkipReason::ExistingFile);
    assert_eq!(outcome.skipped[0].table, "T_ONE");
    assert!(outcome.is_ok(), "a skip is not a failure");
    assert_eq!(outcome.handled(), 2);
}

/// Every answer the *ask* policy takes, and what the run does with it.
fn ask_run(answers: Vec<Decision>) -> (Fixture, rudbgen_gen::Outcome, Vec<PathBuf>) {
    let fx = Fixture::new();
    std::fs::create_dir_all(&fx.out).expect("out dir");
    for name in ["T_ONE.java", "T_TWO.java", "T_THREE.java"] {
        std::fs::write(at(&fx.out, name), "hand written").expect("existing file");
    }

    let plan = plan(
        vec![table("T_ONE"), table("T_TWO"), table("T_THREE")],
        vec![template(
            &fx.templates,
            "one.java",
            "class ${name} {}",
            "${name}.java",
        )],
        &fx.out,
    );

    let asked: Arc<Mutex<Vec<PathBuf>>> = Arc::new(Mutex::new(Vec::new()));
    let seen = Arc::clone(&asked);
    let answers = Mutex::new(answers.into_iter());
    let policy = Overwrite::Ask(Box::new(move |path: &Path| {
        seen.lock().unwrap().push(path.to_path_buf());
        answers
            .lock()
            .unwrap()
            .next()
            .expect("the run asked more often than the test answers")
    }));

    let outcome = generate(&plan, policy, &live(), &quiet);
    let questions = asked.lock().unwrap().clone();
    (fx, outcome, questions)
}

#[test]
fn the_ask_policy_asks_once_per_conflict() {
    let (fx, outcome, asked) = ask_run(vec![
        Decision::Overwrite,
        Decision::Skip,
        Decision::Overwrite,
    ]);

    assert_eq!(asked.len(), 3, "one question per file");
    assert_eq!(asked[0], at(&fx.out, "T_ONE.java"));
    assert_eq!(read(&fx.out, "T_ONE.java"), "class T_ONE {}");
    assert_eq!(read(&fx.out, "T_TWO.java"), "hand written");
    assert_eq!(read(&fx.out, "T_THREE.java"), "class T_THREE {}");
    assert_eq!(outcome.written.len(), 2);
    assert_eq!(outcome.skipped[0].reason, SkipReason::UserSkipped);
}

#[test]
fn overwrite_all_stops_the_asking() {
    let (fx, outcome, asked) = ask_run(vec![Decision::OverwriteAll]);

    assert_eq!(asked.len(), 1, "the answer settled the rest of the run");
    assert_eq!(outcome.written.len(), 3);
    assert_eq!(read(&fx.out, "T_THREE.java"), "class T_THREE {}");
}

#[test]
fn skip_all_stops_the_asking() {
    let (fx, outcome, asked) = ask_run(vec![Decision::SkipAll]);

    assert_eq!(asked.len(), 1);
    assert!(outcome.written.is_empty());
    assert_eq!(outcome.skipped.len(), 3);
    assert_eq!(
        outcome.skipped[0].reason,
        SkipReason::UserSkipped,
        "the first was answered"
    );
    assert_eq!(
        outcome.skipped[2].reason,
        SkipReason::ExistingFile,
        "the rest were settled"
    );
    assert_eq!(read(&fx.out, "T_ONE.java"), "hand written");
}

#[test]
fn cancel_from_the_question_stops_the_run() {
    let (fx, outcome, asked) = ask_run(vec![Decision::Overwrite, Decision::Cancel]);

    assert_eq!(asked.len(), 2);
    assert!(outcome.cancelled);
    assert!(!outcome.is_ok());
    assert_eq!(
        outcome.written,
        vec![at(&fx.out, "T_ONE.java")],
        "what was written before the cancel stays written"
    );
    assert_eq!(read(&fx.out, "T_TWO.java"), "hand written");
}

#[test]
fn a_file_that_is_not_there_is_never_asked_about() {
    let fx = Fixture::new();
    let plan = plan(
        vec![table("T_ONE")],
        vec![template(
            &fx.templates,
            "one.java",
            "class ${name} {}",
            "${name}.java",
        )],
        &fx.out,
    );
    let policy = Overwrite::Ask(Box::new(|_: &Path| {
        panic!("nothing was there to ask about")
    }));

    let outcome = generate(&plan, policy, &live(), &quiet);
    assert_eq!(outcome.written.len(), 1);
}

#[test]
fn the_saved_policy_is_what_the_run_uses() {
    let fx = Fixture::new();
    std::fs::create_dir_all(&fx.out).expect("out dir");
    std::fs::write(at(&fx.out, "T_ONE.java"), "hand written").expect("existing file");

    let plan = plan(
        vec![table("T_ONE")],
        vec![template(
            &fx.templates,
            "one.java",
            "class ${name} {}",
            "${name}.java",
        )],
        &fx.out,
    );

    let policy = Overwrite::from_policy(OverwritePolicy::Skip, |_| Decision::Overwrite);
    let outcome = generate(&plan, policy, &live(), &quiet);
    assert_eq!(outcome.skipped.len(), 1);
    assert_eq!(read(&fx.out, "T_ONE.java"), "hand written");
}

// --- rule 6: cancelling ---------------------------------------------------

#[test]
fn a_cancelled_run_stops_at_a_file_boundary_and_keeps_what_it_wrote() {
    let fx = Fixture::new();
    let plan = plan(
        vec![table("T_ONE"), table("T_TWO"), table("T_THREE")],
        vec![template(
            &fx.templates,
            "one.java",
            "class ${name} {}",
            "${name}.java",
        )],
        &fx.out,
    );

    let cancel = CancelToken::new();
    let handle = cancel.handle();
    let outcome = generate(&plan, Overwrite::Overwrite, &cancel, &move |event| {
        // Cancel from inside the run, the way the progress dialog does.
        if let Progress::File { index: 1, .. } = event {
            handle.cancel();
        }
    });

    assert!(outcome.cancelled);
    assert_eq!(outcome.written, vec![at(&fx.out, "T_ONE.java")]);
    assert_eq!(tree(&fx.out), ["T_ONE.java"], "no half-written second file");
}

#[test]
fn a_token_that_is_already_cancelled_writes_nothing() {
    let fx = Fixture::new();
    let plan = plan(
        vec![table("T_ONE")],
        vec![template(
            &fx.templates,
            "one.java",
            "class ${name} {}",
            "${name}.java",
        )],
        &fx.out,
    );

    let cancel = CancelToken::new();
    cancel.cancel();
    let outcome = generate(&plan, Overwrite::Overwrite, &cancel, &quiet);

    assert!(outcome.cancelled);
    assert!(outcome.written.is_empty());
    assert!(!fx.out.exists());
}

// --- progress -------------------------------------------------------------

#[test]
fn the_progress_events_come_in_one_order_and_one_per_file() {
    let fx = Fixture::new();
    std::fs::create_dir_all(&fx.out).expect("out dir");
    std::fs::write(at(&fx.out, "T_TWO.java"), "hand written").expect("existing file");

    let plan = plan(
        vec![table("T_ONE"), table("T_TWO")],
        vec![template(
            &fx.templates,
            "one.java",
            "class ${name} {}",
            "${name}.java",
        )],
        &fx.out,
    );

    let events = Events::new();
    generate(&plan, Overwrite::Skip, &live(), &events.sink());

    assert_eq!(
        events.labels(),
        [
            "started 2",
            "parsed 1",
            "file 1/2 T_ONE Written",
            "file 2/2 T_TWO Skipped(ExistingFile)",
            "finished 1",
        ]
    );

    // The last event carries the same summary the call answers with.
    let Some(Progress::Finished(outcome)) = events.all().last().cloned() else {
        panic!("the run did not finish");
    };
    assert_eq!(outcome.written, vec![at(&fx.out, "T_ONE.java")]);
}

#[test]
fn a_parse_error_reports_started_and_finished_and_nothing_between() {
    let fx = Fixture::new();
    let plan = plan(
        vec![table("T_ONE")],
        vec![template(
            &fx.templates,
            "bad.java",
            "${name",
            "${name}.java",
        )],
        &fx.out,
    );

    let events = Events::new();
    generate(&plan, Overwrite::Overwrite, &live(), &events.sink());

    assert_eq!(events.labels(), ["started 1", "finished 0"]);
}

#[test]
fn a_failed_pair_is_reported_without_a_path() {
    let fx = Fixture::new();
    let plan = plan(
        vec![table("T_ONE")],
        vec![template(
            &fx.templates,
            "one.java",
            "class ${name} {}",
            "../out.java",
        )],
        &fx.out,
    );

    let events = Events::new();
    generate(&plan, Overwrite::Overwrite, &live(), &events.sink());

    let Some(Progress::File { path, status, .. }) = events.all().into_iter().nth(2) else {
        panic!("no file event");
    };
    assert_eq!(path, None);
    assert!(matches!(status, FileStatus::Failed(_)));
}

// --- rule 7: diagnostics --------------------------------------------------

#[test]
fn an_unknown_field_is_collected_and_still_renders() {
    let fx = Fixture::new();
    let plan = plan(
        vec![table("T_ONE")],
        vec![template(
            &fx.templates,
            "one.java",
            "class ${name} { ${nosuchfield} }",
            "${name}.java",
        )],
        &fx.out,
    );

    let outcome = generate(&plan, Overwrite::Overwrite, &live(), &quiet);

    assert!(outcome.is_ok(), "a warning is not a failure");
    assert_eq!(read(&fx.out, "T_ONE.java"), "class T_ONE {  }");
    assert_eq!(outcome.diagnostics.len(), 1);
    let found = &outcome.diagnostics[0];
    assert_eq!(found.table, "T_ONE");
    assert_eq!(found.template, "one.java");
    assert_eq!(found.part, TemplatePart::Body);
    assert_eq!(found.warning.key, "nosuchfield");
}

#[test]
fn a_warning_of_the_output_name_is_told_apart_from_one_of_the_body() {
    let fx = Fixture::new();
    let plan = plan(
        vec![table("T_ONE")],
        vec![template(
            &fx.templates,
            "one.java",
            "class ${name} {}",
            "${nosuchfield}${name}.java",
        )],
        &fx.out,
    );

    let outcome = generate(&plan, Overwrite::Overwrite, &live(), &quiet);
    assert_eq!(outcome.diagnostics.len(), 1);
    assert_eq!(outcome.diagnostics[0].part, TemplatePart::OutputName);
    assert_eq!(read(&fx.out, "T_ONE.java"), "class T_ONE {}");
}

// --- rule 8: the author ---------------------------------------------------

#[test]
fn the_author_of_the_form_is_what_the_template_renders() {
    let fx = Fixture::new();
    let mut plan = plan(
        vec![table("T_ONE")],
        vec![template(
            &fx.templates,
            "one.java",
            "// ${author} / ${item:key=author} / ${user}",
            "${name}.java",
        )],
        &fx.out,
    );
    plan.author = "comart".to_string();
    plan.custom_vars = vec![("author".to_string(), "from the table".to_string())];

    generate(&plan, Overwrite::Overwrite, &live(), &quiet);

    assert_eq!(read(&fx.out, "T_ONE.java"), "// comart / comart / tester");
}

// --- rule 9: the abbreviations --------------------------------------------

#[test]
fn the_saved_rules_rewrite_the_generated_names() {
    let fx = Fixture::new();
    let store = AbbreviationStore {
        apply_to_names: true,
        rules: vec![AbbreviationRule {
            enabled: true,
            whole_name: false,
            abbreviation: "USR".to_string(),
            replacement: "user".to_string(),
        }],
        ..AbbreviationStore::default()
    };

    let plan = plan(
        vec![table("TB_USR")],
        vec![template(
            &fx.templates,
            "one.java",
            "class ${name.suffix.pascal} {}",
            "${name.suffix.pascal}.java",
        )],
        &fx.out,
    )
    .with_abbreviations(&store);

    let outcome = generate(&plan, Overwrite::Overwrite, &live(), &quiet);

    // D10: the word rule fires on an upper-case identifier.
    assert!(outcome.is_ok(), "{outcome:?}");
    assert_eq!(tree(&fx.out), ["User.java"]);
    assert_eq!(read(&fx.out, "User.java"), "class User {}");
}

#[test]
fn the_rules_do_nothing_while_the_switch_is_off() {
    let fx = Fixture::new();
    let store = AbbreviationStore {
        apply_to_names: false,
        rules: vec![AbbreviationRule {
            enabled: true,
            whole_name: false,
            abbreviation: "USR".to_string(),
            replacement: "user".to_string(),
        }],
        ..AbbreviationStore::default()
    };

    let plan = plan(
        vec![table("TB_USR")],
        vec![template(
            &fx.templates,
            "one.java",
            "class ${name.suffix.pascal} {}",
            "${name.suffix.pascal}.java",
        )],
        &fx.out,
    )
    .with_abbreviations(&store);

    generate(&plan, Overwrite::Overwrite, &live(), &quiet);
    assert_eq!(tree(&fx.out), ["Usr.java"]);
}

// --- the dry run ----------------------------------------------------------

#[test]
fn a_dry_run_renders_everything_and_writes_nothing() {
    let fx = Fixture::new();
    std::fs::create_dir_all(&fx.out).expect("out dir");
    std::fs::write(at(&fx.out, "T_ONE.java"), "hand written").expect("existing file");

    let plan = plan(
        vec![table("T_ONE"), table("T_TWO")],
        vec![template(
            &fx.templates,
            "one.java",
            "class ${name} { ${nosuchfield} }",
            "${name}.java",
        )],
        &fx.out,
    );

    let events = Events::new();
    let result = dry_run(&plan, &live(), &events.sink());

    assert_eq!(result.files.len(), 2);
    assert_eq!(result.files[0].path, at(&fx.out, "T_ONE.java"));
    assert_eq!(result.files[0].content, "class T_ONE {  }");
    assert!(result.files[0].exists, "it would replace a file");
    assert!(!result.files[1].exists);
    assert_eq!(result.diagnostics.len(), 2, "one warning per table");
    assert!(result.failed.is_empty());

    assert_eq!(
        tree(&fx.out),
        ["T_ONE.java"],
        "the disk is exactly as it was"
    );
    assert_eq!(read(&fx.out, "T_ONE.java"), "hand written");
    assert_eq!(
        events.labels(),
        [
            "started 2",
            "parsed 1",
            "file 1/2 T_ONE Written",
            "file 2/2 T_TWO Written",
            "finished 2",
        ]
    );
}

#[test]
fn a_dry_run_of_a_broken_template_writes_nothing_either() {
    let fx = Fixture::new();
    let plan = plan(
        vec![table("T_ONE")],
        vec![template(
            &fx.templates,
            "bad.java",
            "${name",
            "${name}.java",
        )],
        &fx.out,
    );

    let result = dry_run(&plan, &live(), &quiet);
    assert!(result.files.is_empty());
    assert_eq!(result.failed.len(), 1);
    assert!(!fx.out.exists());
}

#[test]
fn a_dry_run_can_be_cancelled_too() {
    let fx = Fixture::new();
    let plan = plan(
        vec![table("T_ONE"), table("T_TWO")],
        vec![template(
            &fx.templates,
            "one.java",
            "class ${name} {}",
            "${name}.java",
        )],
        &fx.out,
    );

    let cancel = CancelToken::new();
    cancel.cancel();
    let result = dry_run(&plan, &cancel, &quiet);

    assert!(result.cancelled);
    assert!(result.files.is_empty());
}

// --- the preview ----------------------------------------------------------

#[test]
fn a_preview_renders_one_pair_and_says_where_it_would_go() {
    let fx = Fixture::new();
    let plan = plan(
        vec![table("T_ONE"), table("T_TWO")],
        vec![
            template(&fx.templates, "a.java", "class ${name} {}", "${name}.java"),
            template(&fx.templates, "b.xml", "<t>${name}</t>", "${name}.xml"),
        ],
        &fx.out,
    );

    let preview = preview(&plan, 1, 1).expect("a pair that exists");

    assert_eq!(preview.path, at(&fx.out, "T_TWO.xml"));
    assert_eq!(preview.content, "<t>T_TWO</t>");
    assert!(preview.diagnostics.is_empty());
    assert!(!fx.out.exists(), "a preview writes nothing");
}

#[test]
fn a_preview_hands_back_the_unknown_fields() {
    let fx = Fixture::new();
    let plan = plan(
        vec![table("T_ONE")],
        vec![template(
            &fx.templates,
            "a.java",
            "class ${name} { ${nosuchfield} }",
            "${name}.java",
        )],
        &fx.out,
    );

    let preview = preview(&plan, 0, 0).expect("a pair that exists");
    assert_eq!(preview.diagnostics.len(), 1);
    assert_eq!(preview.diagnostics[0].key, "nosuchfield");
}

#[test]
fn a_preview_of_a_pair_that_is_not_there_is_an_error() {
    let fx = Fixture::new();
    let plan = plan(
        vec![table("T_ONE")],
        vec![template(&fx.templates, "a.java", "x", "${name}.java")],
        &fx.out,
    );

    assert_eq!(
        preview(&plan, 9, 0),
        Err(rudbgen_gen::Error::NoSuchTable(9))
    );
    assert_eq!(
        preview(&plan, 0, 9),
        Err(rudbgen_gen::Error::NoSuchTemplate(9))
    );
}

#[test]
fn a_preview_of_a_broken_template_says_which_line() {
    let fx = Fixture::new();
    let plan = plan(
        vec![table("T_ONE")],
        vec![template(
            &fx.templates,
            "a.java",
            "line one\nline ${two",
            "${name}.java",
        )],
        &fx.out,
    );

    let error = preview(&plan, 0, 0).expect_err("a template that does not parse");
    assert!(error.to_string().contains("line 2"), "{error}");
}

// --- the whole run --------------------------------------------------------

#[test]
fn every_table_meets_every_template() {
    let fx = Fixture::new();
    let plan = plan(
        vec![table("T_ONE"), table("T_TWO")],
        vec![
            template(&fx.templates, "a.java", "class ${name} {}", "${name}.java"),
            template(&fx.templates, "b.xml", "<t>${name}</t>", "xml/${name}.xml"),
        ],
        &fx.out,
    );

    let outcome = generate(&plan, Overwrite::Overwrite, &live(), &quiet);

    assert!(outcome.is_ok(), "{outcome:?}");
    assert_eq!(plan.total(), 4);
    assert_eq!(
        tree(&fx.out),
        ["T_ONE.java", "T_TWO.java", "xml/T_ONE.xml", "xml/T_TWO.xml"]
    );
}

#[test]
fn no_temporary_file_is_left_behind() {
    let fx = Fixture::new();
    let plan = plan(
        vec![table("T_ONE")],
        vec![template(
            &fx.templates,
            "one.java",
            "class ${name} {}",
            "${name}.java",
        )],
        &fx.out,
    );

    generate(&plan, Overwrite::Overwrite, &live(), &quiet);
    assert_eq!(tree(&fx.out), ["T_ONE.java"]);
}

#[test]
fn a_destination_that_cannot_be_written_is_a_failure_and_not_a_panic() {
    let fx = Fixture::new();
    // A directory where the file should be: the write fails, the run does not.
    std::fs::create_dir_all(at(&fx.out, "T_ONE.java")).expect("a directory in the way");

    let plan = plan(
        vec![table("T_ONE"), table("T_TWO")],
        vec![template(
            &fx.templates,
            "one.java",
            "class ${name} {}",
            "${name}.java",
        )],
        &fx.out,
    );

    let outcome = generate(&plan, Overwrite::Overwrite, &live(), &quiet);

    assert_eq!(outcome.failed.len(), 1);
    assert_eq!(outcome.failed[0].table.as_deref(), Some("T_ONE"));
    assert_eq!(outcome.failed[0].path, Some(at(&fx.out, "T_ONE.java")));
    assert_eq!(
        outcome.written,
        vec![at(&fx.out, "T_TWO.java")],
        "the run carries on to the next pair"
    );
}
