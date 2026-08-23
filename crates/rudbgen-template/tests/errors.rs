//! jdbgen's `TemplateManagerErrorTest`, case for case.
//!
//! A broken template has to say what is wrong with it and where, instead of
//! failing somewhere deep inside the engine.

mod common;

use common::{Fixture, context};
use rudbgen_template::{ParseError, RenderError, Template};

/// A model with one member, one number and one collection.
fn model() -> Fixture {
    Fixture::named("tb_user")
        .with_int("no", 3)
        .with_list("rows", vec![Fixture::named("a"), Fixture::named("b")])
}

/// The failure of parsing `template`.
fn parse_failure(template: &str) -> ParseError {
    Template::parse(template).expect_err("this template must not parse")
}

/// The failure of rendering `template` against the model.
fn render_failure(template: &str) -> RenderError {
    Template::parse(template)
        .expect("this template parses")
        .render(&model(), &context())
        .expect_err("this template must not render")
}

// ------------------------------------------------------------ parse errors

#[test]
fn an_unknown_placeholder_type_is_reported_with_its_name() {
    let error = parse_failure("${nosuch:key=name}");
    assert!(error.message.contains("nosuch"), "{error}");
    assert!(error.message.contains("Unknown template"), "{error}");
}

#[test]
fn a_placeholder_that_is_never_closed_is_reported() {
    let error = parse_failure("select ${item:key=name from dual");
    assert!(error.message.contains("'}' not found"), "{error}");
}

#[test]
fn a_for_statement_that_is_never_closed_is_reported() {
    let error = parse_failure("${for:key=rows}${item:key=name}");
    assert!(
        error.message.contains("for statements not closed"),
        "{error}"
    );
}

#[test]
fn an_if_statement_that_is_never_closed_is_reported() {
    let error = parse_failure("${if:key=name, equals='x'}yes");
    assert!(
        error.message.contains("if statements not closed"),
        "{error}"
    );
}

#[test]
fn an_elif_that_is_never_closed_is_reported() {
    let error = parse_failure("${if:key=name, equals='x'}yes${elif:key=name, equals='y'}no");
    assert!(
        error.message.contains("if statements not closed"),
        "{error}"
    );
}

#[test]
fn a_misspelled_condition_is_refused_instead_of_silently_holding() {
    // 'startWith' is not 'startsWith' - without the check the condition would
    // simply be ignored and the branch always rendered
    let error = parse_failure("${if:key=name, startWith='tb_'}yes${endif}");
    assert!(error.message.contains("Unknown if condition"), "{error}");
}

#[test]
fn a_misspelled_condition_of_an_elif_is_refused_as_well() {
    let error = parse_failure("${if:key=name, equals='x'}a${elif:key=name, containz='y'}b${endif}");
    assert!(error.message.contains("Unknown if condition"), "{error}");
}

#[test]
fn a_dangling_escape_character_is_reported() {
    let error = parse_failure(r"${item:key=name, prepend=a\}");
    assert!(
        error.message.contains("Dangling escape character"),
        "{error}"
    );
}

#[test]
fn an_attribute_with_two_values_is_reported() {
    let error = parse_failure("${item:key=name=other}");
    assert!(
        error.message.contains("Name value pair not matched"),
        "{error}"
    );
}

#[test]
fn an_attribute_without_a_name_is_reported() {
    let error = parse_failure("${item:key=name, ='x'}");
    assert!(
        error.message.contains("Name value pair not matched"),
        "{error}"
    );
}

#[test]
fn the_reported_line_is_the_line_the_error_is_on() {
    let error = parse_failure("first\nsecond\n${nosuch:key=name}\nfourth");
    assert_eq!(
        error.line, 2,
        "the line is counted from zero, so the third line is 2"
    );
    // and the span points at the placeholder itself
    let span = error.span.expect("the placeholder is known");
    assert_eq!(
        &"first\nsecond\n${nosuch:key=name}\nfourth"[span.start..span.end],
        "${nosuch:key=name}"
    );
}

#[test]
fn the_message_points_at_the_text_the_error_was_found_in() {
    let error = parse_failure("${nosuch:key=name} and the rest of the line");
    assert!(
        error.message.contains("and the rest of the line"),
        "the user has to be able to find the place: {error}"
    );
}

// ----------------------------------------------------------- render errors

#[test]
fn a_placeholder_without_a_key_is_reported() {
    let error = render_failure("${for:instr=','}x${endfor}");
    assert!(
        error.message.contains("'key' or 'item' is required"),
        "{error}"
    );
}

#[test]
fn an_if_without_a_key_is_reported() {
    let error = render_failure("${if:equals='x'}yes${endif}");
    assert!(
        error.message.contains("'key' or 'item' is required"),
        "{error}"
    );
}

#[test]
fn an_unknown_string_processor_is_reported_with_the_valid_ones() {
    let error = render_failure("${name.capitalize}");
    assert!(error.message.contains("capitalize"), "{error}");
    assert!(
        error.message.contains("camel") && error.message.contains("replace"),
        "the valid processors are listed: {error}"
    );
}

#[test]
fn a_for_over_something_the_model_does_not_have_is_reported() {
    let error = render_failure("${for:key=nowhere}${item:key=name}${endfor}");
    assert!(error.message.contains("nowhere"), "{error}");
    assert!(error.message.contains("Model has no"), "{error}");
}

#[test]
fn contains_on_a_value_that_is_neither_a_collection_nor_a_text_is_reported() {
    let error = render_failure("${if:key=no, contains='3'}yes${endif}");
    assert!(error.message.contains("collection"), "{error}");
}

#[test]
fn a_padding_that_is_no_number_is_reported() {
    let error = render_failure("${item:key=name, padSize=wide}");
    assert!(error.message.contains("wide"), "{error}");
}

#[test]
fn an_indent_that_is_no_number_is_reported() {
    let error = render_failure("${for:key=rows, indent=two}${item:key=name}${endfor}");
    assert!(error.message.contains("two"), "{error}");
}

#[test]
fn an_invalid_date_format_is_reported() {
    let error = render_failure("${date:yyyy-bb}");
    assert!(error.message.to_lowercase().contains("pattern"), "{error}");
}

#[test]
fn a_render_error_knows_the_line_it_is_on() {
    let error = render_failure("one\ntwo\n${item:key=name, padSize=wide}");
    assert_eq!(error.line(), Some(2));
}
