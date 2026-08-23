//! The oddities of jdbgen's syntax that the assets depend on.
//!
//! Nothing here is a nice rule; every one of them is what jdbgen does, checked
//! against jdbgen itself (`tests/golden/QuirkMain.java.txt` renders the same
//! list through the Java engine). A template in the wild may lean on any of
//! them, so they are pinned rather than fixed.

mod common;

use common::{Fixture, context, render};
use rudbgen_template::Template;

/// The model the Java run used: one name and two elements whose names differ
/// only in their case.
fn model() -> Fixture {
    Fixture::named("abc_def").with_list("rows", vec![Fixture::named("a"), Fixture::named("A")])
}

#[test]
fn a_placeholder_ends_at_the_first_brace_even_inside_a_quoted_value() {
    // the value is cut off at the '}', the rest of it stays text
    assert_eq!(
        render("${item:key=name, prepend='}'}", &model()),
        "'abc_def'}"
    );
}

#[test]
fn an_upper_case_keyword_is_read_as_a_member_lookup() {
    // ${ELSE} is not the else branch: it is a lookup of a member called ELSE,
    // which renders as nothing, so both branches end up in the output
    assert_eq!(
        render(
            "${if:key=name, equals='abc_def'}a${ELSE}b${endif}",
            &model()
        ),
        "ab"
    );
}

#[test]
fn a_template_ending_in_a_bare_placeholder_start_drops_it() {
    assert_eq!(render("abc${", &model()), "abc");
}

#[test]
fn a_literal_keeps_the_white_space_inside_its_quotes() {
    assert_eq!(render("[${'  '}]", &model()), "[  ]");
    assert_eq!(render("${'a'}${'b'}", &model()), "ab");
}

#[test]
fn white_space_around_the_type_and_the_attributes_is_ignored() {
    assert_eq!(render("${ item : key = name }", &model()), "abc_def");
}

#[test]
fn a_repeated_attribute_keeps_its_last_value() {
    assert_eq!(
        render("${item:key=name, prepend='a', prepend='b'}", &model()),
        "babc_def"
    );
}

#[test]
fn a_processor_with_arguments_may_be_followed_by_another_one() {
    assert_eq!(
        render("${name.replace('_','-').upper}", &model()),
        "ABC-DEF"
    );
    assert_eq!(render("${name.suffix.suffix}", &model()), "def");
}

#[test]
fn the_skip_list_compares_the_case_of_a_name() {
    assert_eq!(
        render(
            "${for:key=rows, skipList='A'}${item:key=name}${endfor}",
            &model()
        ),
        "a"
    );
}

#[test]
fn a_member_is_looked_up_with_the_case_the_template_writes() {
    assert_eq!(render("[${item:key=NAME}]", &model()), "[]");
}

#[test]
fn a_padding_that_is_negative_or_too_small_pads_nothing() {
    assert_eq!(render("${item:key=name, padSize=-5}", &model()), "abc_def");
}

#[test]
fn the_padding_direction_is_case_insensitive() {
    assert_eq!(
        render("${item:key=name, padDir='LEFT', padSize=10}", &model()),
        "   abc_def"
    );
}

#[test]
fn author_takes_the_decorations_of_its_placeholder() {
    assert_eq!(
        render("${author:prepend='<', postpend='>'}", &model()),
        "<John Doe>"
    );
}

#[test]
fn an_author_that_was_never_set_renders_nothing_at_all() {
    // not even its decorations, which is how jdbgen tells a value that is
    // there from one that is not
    let ctx = rudbgen_template::RenderContext::new().with_user("tester");
    let text = Template::parse("[${author:prepend='<', postpend='>'}]")
        .unwrap()
        .render(&model(), &ctx)
        .unwrap();
    assert_eq!(text, "[]");
}

#[test]
fn a_literal_that_is_never_closed_is_reported_as_a_missing_brace() {
    // the open quote swallows the '}' as well, so the placeholder is the one
    // that is unterminated
    let error = Template::parse("${'unclosed}").unwrap_err();
    assert!(error.message.contains("'}' not found"), "{error}");
}

#[test]
fn the_statements_of_a_template_are_handed_out_with_their_positions() {
    // this is what the editor highlights and marks diagnostics on
    let source = "-- ${name} --\n${for:key=rows}${item:key=name}${endfor}";
    let spans = Template::parse(source).unwrap().spans();

    let text: Vec<&str> = spans
        .iter()
        .map(|s| &source[s.span.start..s.span.end])
        .collect();
    assert_eq!(
        text,
        vec![
            "-- ",
            "${name}",
            " --\n",
            "${for:key=rows}",
            "${item:key=name}"
        ]
    );
    assert_eq!(spans[3].span.line, 1, "the loop starts on the second line");
}

#[test]
fn the_context_may_be_reused_for_every_table_of_a_schema() {
    let template = Template::parse("${name.pascal}").unwrap();
    let ctx = context();
    let names: Vec<String> = ["tb_user", "tb_role"]
        .iter()
        .map(|name| template.render(&Fixture::named(name), &ctx).unwrap())
        .collect();
    assert_eq!(names, vec!["TbUser".to_string(), "TbRole".to_string()]);
}
