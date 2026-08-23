//! jdbgen's `TemplateManagerTest`, case for case.

mod common;

use common::{Fixture, context, render, render_with};
use rudbgen_template::{AbbrRule, Abbreviations, RenderContext, Template};

/// The model of the Java test: one name that splits into four words.
fn map_obj() -> Fixture {
    Fixture::named("abc_def_ghi_jkl")
}

/// The second model, whose whole name is an abbreviation of its own.
fn map_obj2() -> Fixture {
    Fixture::named("mno_pqr_stu_vwx")
}

/// The abbreviation dictionary of the Java test.
fn abbreviations() -> Abbreviations {
    Abbreviations::from_rules([
        AbbrRule::word("def", "default"),
        AbbrRule::whole("mno_pqr_stu_vwx", "mnopqr"),
    ])
}

fn abbr_context() -> RenderContext {
    context().with_abbreviations(abbreviations())
}

// ------------------------------------------------------------- decorators

#[test]
fn a_placeholder_is_replaced_in_place() {
    assert_eq!(render("a${name}b", &map_obj()), "aabc_def_ghi_jklb");
}

#[test]
fn the_processors_rewrite_the_value() {
    let model = map_obj();
    assert_eq!(render("${name.suffix}", &model), "def_ghi_jkl");
    assert_eq!(render("${name.prefix}", &model), "abc_def_ghi");
    assert_eq!(render("${name.lower}", &model), "abc_def_ghi_jkl");
    assert_eq!(render("${name.upper}", &model), "ABC_DEF_GHI_JKL");
    assert_eq!(render("${name.pascal}", &model), "AbcDefGhiJkl");
    assert_eq!(render("${name.camel}", &model), "abcDefGhiJkl");
    assert_eq!(render("${name.snake}", &model), "abc_def_ghi_jkl");
    assert_eq!(render("${name.skewer}", &model), "abc-def-ghi-jkl");
    assert_eq!(
        render("${name.replace('ghi','mno')}", &model),
        "abc_def_mno_jkl"
    );
}

#[test]
fn the_abbr_processor_replaces_a_word_and_a_whole_name() {
    let ctx = abbr_context();
    assert_eq!(
        render_with("${name.abbr}", &map_obj(), &ctx),
        "abc_default_ghi_jkl"
    );
    assert_eq!(render_with("${name.abbr}", &map_obj2(), &ctx), "mnopqr");
}

#[test]
fn the_abbreviation_option_abbreviates_a_name_by_itself() {
    let ctx = abbr_context().with_apply_abbr(true);
    assert_eq!(
        render_with("${name}", &map_obj(), &ctx),
        "abc_default_ghi_jkl"
    );
    assert_eq!(render_with("${name}", &map_obj2(), &ctx), "mnopqr");

    // and does nothing at all while it is turned off
    let off = abbr_context();
    assert_eq!(render_with("${name}", &map_obj(), &off), "abc_def_ghi_jkl");
}

#[test]
fn processors_may_be_chained() {
    assert_eq!(render("${name.suffix.camel}", &map_obj()), "defGhiJkl");
}

#[test]
fn the_long_form_of_a_placeholder_reads_the_same_key() {
    assert_eq!(render("${item:key=name.camel}", &map_obj()), "abcDefGhiJkl");
}

// ------------------------------------------------------- extra decorators

#[test]
fn pad_size_pads_to_the_right_by_default() {
    let result = render("${item:key=name, padSize=20, padDir='right'}", &map_obj());
    assert_eq!(result.len(), 20);
    assert_eq!(result.trim_end(), "abc_def_ghi_jkl");

    let defaulted = render("${item:key=name, padSize=20}", &map_obj());
    assert_eq!(defaulted, result);
}

#[test]
fn pad_dir_left_pads_in_front_of_the_value() {
    let result = render("${item:key=name, padSize=20, padDir='left'}", &map_obj());
    assert_eq!(result.len(), 20);
    assert_eq!(result.trim_start(), "abc_def_ghi_jkl");
}

#[test]
fn quote_prepend_and_postpend_surround_the_value() {
    let model = map_obj();
    assert_eq!(
        render("${item:key=name, quote=\"'\"}", &model),
        "'abc_def_ghi_jkl'"
    );
    assert_eq!(
        render("${item:key=name, prepend='xyz_'}", &model),
        "xyz_abc_def_ghi_jkl"
    );
    assert_eq!(
        render("${item:key=name, postpend='_xyz'}", &model),
        "abc_def_ghi_jkl_xyz"
    );
}

// ------------------------------------------------------ control statements

/// The model of the Java control statement test: five named elements and one
/// plain value.
fn collection_model() -> Fixture {
    Fixture::new()
        .with_list(
            "collection",
            (0..5)
                .map(|i| Fixture::named(&format!("sample{i}")))
                .collect(),
        )
        .with("single", "SINGLE_VALUE")
}

#[test]
fn a_for_statement_repeats_its_body_over_the_collection() {
    let model = collection_model();
    assert_eq!(
        render("${for:key=collection}${item:key=name}${endfor}", &model),
        "sample0sample1sample2sample3sample4"
    );
    assert_eq!(
        render(
            "${for:key=collection, instr=','}${item:key=name}${endfor}",
            &model
        ),
        "sample0,sample1,sample2,sample3,sample4"
    );
    assert_eq!(
        render(
            "${for:key=collection, instr=',\n', indent=4}${item:key=name}${endfor}",
            &model
        ),
        "sample0,\n    sample1,\n    sample2,\n    sample3,\n    sample4"
    );
    assert_eq!(
        render(
            "${for:key=collection, skipList='sample2'}${item:key=name}${endfor}",
            &model
        ),
        "sample0sample1sample3sample4"
    );
}

#[test]
fn super_reaches_the_model_the_loop_started_from() {
    assert_eq!(
        render(
            "${for:key=collection}${super:key=single.suffix}${endfor}",
            &collection_model()
        ),
        "VALUEVALUEVALUEVALUEVALUE"
    );
}

#[test]
fn the_equals_condition_compares_the_whole_value() {
    let model = collection_model();
    assert_eq!(
        render(
            "${if:key=single, equals='SINGLE_VALUE'}True${endif}",
            &model
        ),
        "True"
    );
    assert_eq!(
        render("${if:key=single, equals='DUMMY'}True${endif}", &model),
        ""
    );
    assert_eq!(
        render(
            "${if:key=single, notEquals='SINGLE_VALUE'}True${endif}",
            &model
        ),
        ""
    );
    assert_eq!(
        render("${if:key=single, notEquals='DUMMY'}True${endif}", &model),
        "True"
    );
}

#[test]
fn the_starts_with_and_ends_with_conditions_look_at_the_ends() {
    let model = collection_model();
    assert_eq!(
        render("${if:key=single, startsWith='SINGLE'}True${endif}", &model),
        "True"
    );
    assert_eq!(
        render("${if:key=single, startsWith='DUMMY'}True${endif}", &model),
        ""
    );
    assert_eq!(
        render("${if:key=single, endsWith='VALUE'}True${endif}", &model),
        "True"
    );
    assert_eq!(
        render("${if:key=single, endsWith='DUMMY'}True${endif}", &model),
        ""
    );
}

#[test]
fn the_contains_condition_reads_a_collection_and_a_comma_separated_text() {
    let model = collection_model();
    assert_eq!(
        render(
            "${if:key=collection, contains='sample1'}True${endif}",
            &model
        ),
        "True"
    );
    assert_eq!(
        render(
            "${if:key=collection, contains='sample9'}True${endif}",
            &model
        ),
        ""
    );
    assert_eq!(
        render(
            "${if:key=single.suffix.lower, contains='sample, value'}True${endif}",
            &model
        ),
        "True"
    );
    assert_eq!(
        render(
            "${if:key=single.suffix.lower, contains='sample, proc'}True${endif}",
            &model
        ),
        ""
    );
}

#[test]
fn the_not_contains_condition_is_the_opposite_of_contains() {
    let model = collection_model();
    assert_eq!(
        render(
            "${if:key=collection, notcontains='sample1'}True${endif}",
            &model
        ),
        ""
    );
    assert_eq!(
        render(
            "${if:key=collection, notcontains='sample9'}True${endif}",
            &model
        ),
        "True"
    );
    assert_eq!(
        render(
            "${if:key=single.suffix.lower, notcontains='sample, value'}True${endif}",
            &model
        ),
        ""
    );
    assert_eq!(
        render(
            "${if:key=single.suffix.lower, notcontains='sample, proc'}True${endif}",
            &model
        ),
        "True"
    );
}

#[test]
fn the_matches_condition_is_a_regular_expression() {
    let model = collection_model();
    assert_eq!(
        render(
            "${if:key=single.lower, matches='[a-z]+_[a-z]+'}True${endif}",
            &model
        ),
        "True"
    );
    assert_eq!(
        render(
            "${if:key=single, matches='[a-z]+_[a-z]+'}True${endif}",
            &model
        ),
        ""
    );
}

#[test]
fn every_condition_of_an_if_has_to_hold() {
    let model = collection_model();
    assert_eq!(
        render(
            "${if:key=single, startsWith='SINGLE', endsWith='VALUE'}True${endif}",
            &model
        ),
        "True"
    );
    assert_eq!(
        render(
            "${if:key=single, startsWith='SINGLE', endsWith='DUMMY'}True${endif}",
            &model
        ),
        ""
    );
}

// ---------------------------------------------------------------- others

#[test]
fn author_user_and_date_render_their_own_values() {
    let model = map_obj();
    assert_eq!(render("${author}", &model), "John Doe");
    assert_eq!(render("${user}", &model), "tester");
    assert_eq!(render("${date:yyyy-MM}", &model), "2024-03");
}

#[test]
fn a_literal_is_copied_without_being_read() {
    let text = "Test sample with ${author}";
    assert_eq!(render(&format!("${{'{text}'}}"), &map_obj()), text);
}

#[test]
fn a_date_without_a_format_falls_back_to_the_default_one() {
    let model = map_obj();
    assert_eq!(render("${date}", &model), "2024-03-07");
    assert_eq!(render("${date:}", &model), "2024-03-07");
    // an explicit format still wins
    assert_eq!(render("${date:yyyy}", &model), "2024");
}

#[test]
fn escape_characters_inside_a_literal_are_consumed() {
    let model = map_obj();
    // template text: ${'It\'s a test'}
    assert_eq!(render(r"${'It\'s a test'}", &model), "It's a test");
    // template text: ${'a\\b'}
    assert_eq!(render(r"${'a\\b'}", &model), r"a\b");
    // template text: ${"say \"hi\""}
    assert_eq!(render(r#"${"say \"hi\""}"#, &model), "say \"hi\"");
}

#[test]
fn replace_collects_its_arguments_quoted_or_bare() {
    let model = map_obj();
    assert_eq!(
        render("${name.replace('_','-')}", &model),
        "abc-def-ghi-jkl"
    );
    assert_eq!(render("${name.replace(_, -)}", &model), "abc-def-ghi-jkl");
    assert_eq!(
        render("${name.replace(ghi, 'xyz')}", &model),
        "abc_def_xyz_jkl"
    );
    assert_eq!(
        render("${name.replace('ghi', xyz)}", &model),
        "abc_def_xyz_jkl"
    );
}

#[test]
fn replace_with_too_few_arguments_says_so() {
    let template = Template::parse("${name.replace(ghi)}").unwrap();
    let error = template.render(&map_obj(), &context()).unwrap_err();
    assert!(
        error.message.contains("replace"),
        "unexpected message: {error}"
    );
}

#[test]
fn a_placeholder_without_a_key_reports_the_missing_attribute() {
    let template = Template::parse("${item:padSize=10}").unwrap();
    let error = template.render(&map_obj(), &context()).unwrap_err();
    assert!(
        error.message.to_lowercase().contains("key"),
        "unexpected message: {error}"
    );
}

#[test]
fn a_for_statement_numbers_its_elements() {
    let model = Fixture::new().with_list(
        "nums",
        (0..3)
            .map(|i| Fixture::named(&format!("sample{i}")))
            .collect(),
    );

    assert_eq!(
        render("${for:key=nums}${item:key=no}${endfor}", &model),
        "123"
    );
    assert_eq!(
        render(
            "${for:key=nums, instr=','}${item:key=no}:${item:key=name}${endfor}",
            &model
        ),
        "1:sample0,2:sample1,3:sample2"
    );
}

#[test]
fn a_leading_line_break_still_tells_the_line_end() {
    let model = Fixture::new().with_list(
        "collection",
        (0..3)
            .map(|i| Fixture::named(&format!("sample{i}")))
            .collect(),
    );

    assert_eq!(
        render(
            "\n${for:key=collection, instr='\n'}${item:key=name}${endfor}",
            &model
        ),
        "\nsample0\nsample1\nsample2"
    );
}

#[test]
fn a_multi_line_separator_is_indented_per_fragment() {
    let model = Fixture::new().with_list(
        "collection",
        (0..3)
            .map(|i| Fixture::named(&format!("sample{i}")))
            .collect(),
    );

    let result = render(
        "${for:key=collection, instr=',\n+\n', indent=2}${item:key=name}${endfor}",
        &model,
    );
    assert_eq!(result, "sample0,\n  +\n  sample1,\n  +\n  sample2");
    assert!(
        !result.contains('\r'),
        "carriage returns must not be injected"
    );
}
