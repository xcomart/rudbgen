//! jdbgen's `TemplateManagerSyntaxTest`, case for case: the parts of the
//! syntax the original test does not reach.

mod common;

use common::{Fixture, context, render_with, row};
use rudbgen_template::{AbbrRule, Abbreviations, Model, RenderContext, Template};

/// The custom variables of the Java test.
fn customs() -> RenderContext {
    context().with_var("project", "jdbgen")
}

fn render(template: &str, model: &dyn Model) -> String {
    render_with(template, model, &customs())
}

// ------------------------------------------------------------- processors

#[test]
fn the_remaining_case_processors_rewrite_the_value() {
    let model = Fixture::named("abc_def_ghi");
    assert_eq!(render("${name.screaming}", &model), "ABC_DEF_GHI");
    // 'kebab' is the second name of 'skewer'
    assert_eq!(render("${name.kebab}", &model), "abc-def-ghi");
    assert_eq!(
        render("${name.skewer}", &model),
        render("${name.kebab}", &model)
    );
}

#[test]
fn prefix_and_suffix_keep_a_value_without_an_underscore() {
    let model = Fixture::named("plain");
    assert_eq!(render("${name.prefix}", &model), "plain");
    assert_eq!(render("${name.suffix}", &model), "plain");
}

#[test]
fn prefix_cuts_at_the_last_and_suffix_at_the_first_underscore() {
    let model = Fixture::named("a_b_c");
    assert_eq!(render("${name.prefix}", &model), "a_b");
    assert_eq!(render("${name.suffix}", &model), "b_c");
}

#[test]
fn processor_names_are_case_insensitive() {
    let model = Fixture::named("abc_def");
    assert_eq!(render("${name.PASCAL}", &model), "AbcDef");
    assert_eq!(render("${name.Camel}", &model), "abcDef");
}

#[test]
fn a_processor_chain_is_applied_from_left_to_right() {
    let model = Fixture::named("tb_user_account");
    // suffix first, then the case conversion of what is left
    assert_eq!(render("${name.suffix.pascal}", &model), "UserAccount");
    // the other way round the underscore is gone before 'suffix' sees it
    assert_eq!(render("${name.pascal.suffix}", &model), "TbUserAccount");
}

#[test]
fn abbreviations_are_applied_per_word_and_keep_the_separators() {
    let ctx = customs().with_abbreviations(Abbreviations::from_rules([
        AbbrRule::word("usr", "user"),
        AbbrRule::word("acct", "account"),
        AbbrRule {
            enabled: false,
            ..AbbrRule::word("tb", "table")
        },
        AbbrRule::whole("tb_sys", "system"),
    ]));

    // '-' and '_' both separate words and are kept where they were,
    // 'tb' is not replaced because its rule is turned off
    assert_eq!(
        render_with("${name.abbr}", &Fixture::named("tb-usr_acct"), &ctx),
        "tb-user_account"
    );
    // a whole name rule wins over the per word ones
    assert_eq!(
        render_with("${name.abbr}", &Fixture::named("TB_SYS"), &ctx),
        "system"
    );
    // a name without any known word is handed through unchanged
    assert_eq!(
        render_with("${name.abbr}", &Fixture::named("other_name"), &ctx),
        "other_name"
    );
}

#[test]
fn word_rules_match_ignoring_the_case_of_the_identifier() {
    // D10, the one deliberate break from jdbgen, which matched lower case
    // segments only and therefore never fired on a real column name
    let ctx =
        customs().with_abbreviations(Abbreviations::from_rules([AbbrRule::word("usr", "user")]));

    assert_eq!(
        render_with("${name.abbr}", &Fixture::named("TB_USR"), &ctx),
        "TB_user"
    );
}

// -------------------------------------------------------------- attributes

#[test]
fn attribute_names_are_case_insensitive() {
    let model = Fixture::named("abc");
    assert_eq!(
        render("${item:KEY=name, PrePend='[', POSTPEND=']'}", &model),
        "[abc]"
    );
}

#[test]
fn quoted_attribute_values_may_hold_commas_and_escapes() {
    let model = Fixture::named("abc");
    assert_eq!(
        render("${item:key=name, prepend='a,b|'}", &model),
        "a,b|abc"
    );
    assert_eq!(
        render(r"${item:key=name, postpend='\n\t'}", &model),
        "abc\n\t"
    );
    // an escaped quote does not end the value
    assert_eq!(
        render(r"${item:key=name, prepend='it\'s '}", &model),
        "it's abc"
    );
}

#[test]
fn a_value_wrapped_in_parentheses_keeps_them() {
    let model = Fixture::named("abc");
    // '(' only groups the commas, it is not a quote character
    assert_eq!(
        render("${item:key=name, prepend=(a,b)}", &model),
        "(a,b)abc"
    );
}

#[test]
fn quote_surrounds_the_value_and_is_overridden_per_side() {
    let model = Fixture::named("abc");
    assert_eq!(render("${item:key=name, quote='\"'}", &model), "\"abc\"");
    // 'prepend' replaces the opening quote only
    assert_eq!(
        render("${item:key=name, quote='\"', prepend='<'}", &model),
        "<abc\""
    );
    assert_eq!(
        render("${item:key=name, quote='\"', postpend='>'}", &model),
        "\"abc>"
    );
}

#[test]
fn padding_counts_double_byte_characters_twice() {
    // two Hangul syllables, four EUC-KR bytes
    let model = Fixture::new().with("name", "가나");
    assert_eq!(
        render("${item:key=name, padSize=10}", &model),
        "가나      ",
        "a double byte character occupies two columns of a fixed width font"
    );
}

#[test]
fn a_value_longer_than_the_padding_is_not_cut_off() {
    let model = Fixture::named("abcdefghij");
    assert_eq!(render("${item:key=name, padSize=4}", &model), "abcdefghij");
}

#[test]
fn the_decorations_surround_the_value_before_it_is_padded() {
    let model = Fixture::named("abc");
    // '(abc)' is five characters, so five spaces are left of ten
    assert_eq!(
        render(
            "${item:key=name, prepend='(', postpend=')', padSize=10}",
            &model
        ),
        "(abc)     "
    );
    assert_eq!(
        render(
            "${item:key=name, prepend='(', postpend=')', padSize=10, padDir='left'}",
            &model
        ),
        "     (abc)"
    );
}

#[test]
fn a_key_may_also_be_written_as_the_item_attribute() {
    assert_eq!(
        render("${item:item=name.upper}", &Fixture::named("abc")),
        "ABC"
    );
}

// ------------------------------------------------------------------ values

#[test]
fn a_key_the_model_does_not_answer_falls_back_to_the_custom_variables() {
    assert_eq!(
        render("${item:key=project.upper}", &Fixture::named("abc")),
        "JDBGEN"
    );
}

#[test]
fn an_unknown_key_renders_as_nothing() {
    assert_eq!(
        render("[${item:key=nowhere}]", &Fixture::named("abc")),
        "[]"
    );
}

#[test]
fn an_unknown_key_is_reported_as_a_warning() {
    let template = Template::parse("[${item:key=nowhere}]").unwrap();
    let mut diags = rudbgen_template::Diagnostics::new();
    let text = template
        .render_diagnosed(&Fixture::named("abc"), &customs(), &mut diags)
        .unwrap();

    assert_eq!(text, "[]");
    assert_eq!(diags.warnings().len(), 1);
    assert_eq!(diags.warnings()[0].key, "nowhere");
    // the editor marks the placeholder, not the whole line
    assert_eq!(
        &"[${item:key=nowhere}]"[diags.warnings()[0].span.start..diags.warnings()[0].span.end],
        "${item:key=nowhere}"
    );
}

#[test]
fn a_member_that_holds_nothing_reads_as_an_unknown_one() {
    // jdbgen cannot tell a null field from a missing one, and neither can a
    // template: both fall back to the custom variables
    let model = Fixture::new().with_null("project");
    assert_eq!(render("${item:key=project}", &model), "jdbgen");
}

#[test]
fn text_outside_of_a_placeholder_is_copied_verbatim() {
    assert_eq!(
        render("-- ${name} --\nend", &Fixture::named("abc")),
        "-- abc --\nend"
    );
}

#[test]
fn a_literal_is_followed_by_the_rest_of_the_template() {
    // this is how a template writes a '${' of its own
    assert_eq!(
        render("${'${name}'} is ${name}", &Fixture::named("abc")),
        "${name} is abc"
    );
}

#[test]
fn an_empty_literal_stands_for_empty_text() {
    let model = Fixture::named("abc");
    // '${""}' is how a template writes nothing at a place a placeholder would
    // otherwise be read - it is a literal, not a broken placeholder
    assert_eq!(render("[${''}]", &model), "[]");
    assert_eq!(render("[${\"\"}]", &model), "[]");
    assert_eq!(render("[${''}${name}${\"\"}]", &model), "[abc]");
}

// ---------------------------------------------------------------- if / elif

#[test]
fn the_else_branch_is_rendered_when_the_condition_fails() {
    let model = row("abc", "VIEW");
    assert_eq!(
        render(
            "${if:key=type, equals='TABLE'}table${else}view${endif}",
            &model
        ),
        "view"
    );
    assert_eq!(
        render(
            "${if:key=type, equals='VIEW'}table${else}view${endif}",
            &model
        ),
        "table"
    );
}

#[test]
fn an_elif_chain_picks_the_first_matching_branch() {
    let tpl = concat!(
        "${if:key=type, equals='TABLE'}T",
        "${elif:key=type, equals='VIEW'}V",
        "${elif:key=type, equals='SYNONYM'}S",
        "${else}?${endif}"
    );
    assert_eq!(render(tpl, &row("a", "TABLE")), "T");
    assert_eq!(render(tpl, &row("a", "VIEW")), "V");
    assert_eq!(render(tpl, &row("a", "SYNONYM")), "S");
    assert_eq!(render(tpl, &row("a", "SEQUENCE")), "?");
}

#[test]
fn an_elif_chain_without_an_else_renders_nothing_when_no_branch_matches() {
    let tpl = "[${if:key=type, equals='TABLE'}T${elif:key=type, equals='VIEW'}V${endif}]";
    assert_eq!(render(tpl, &row("a", "SEQUENCE")), "[]");
    assert_eq!(render(tpl, &row("a", "VIEW")), "[V]");
}

#[test]
fn the_negated_conditions_are_the_opposite_of_their_counterparts() {
    let model = Fixture::named("tb_user");
    assert_eq!(
        render("${if:key=name, notstartswith='tb_'}x${endif}", &model),
        ""
    );
    assert_eq!(
        render("${if:key=name, notstartswith='vw_'}x${endif}", &model),
        "x"
    );
    assert_eq!(
        render("${if:key=name, notendswith='user'}x${endif}", &model),
        ""
    );
    assert_eq!(
        render("${if:key=name, notendswith='role'}x${endif}", &model),
        "x"
    );
    assert_eq!(
        render("${if:key=name, notmatches='[a-z_]+'}x${endif}", &model),
        ""
    );
    assert_eq!(
        render("${if:key=name, notmatches='[0-9]+'}x${endif}", &model),
        "x"
    );
}

#[test]
fn value_is_another_name_of_the_equals_condition() {
    let model = row("abc", "TABLE");
    assert_eq!(
        render("${if:key=type, value='TABLE'}x${endif}", &model),
        "x"
    );
    assert_eq!(render("${if:key=type, value='VIEW'}x${endif}", &model), "");
}

#[test]
fn every_condition_but_matches_ignores_the_case() {
    let model = Fixture::named("TB_User");
    assert_eq!(
        render("${if:key=name, equals='tb_user'}x${endif}", &model),
        "x"
    );
    assert_eq!(
        render("${if:key=name, startswith='tb_'}x${endif}", &model),
        "x"
    );
    assert_eq!(
        render("${if:key=name, endswith='USER'}x${endif}", &model),
        "x"
    );
    // 'matches' is the only case sensitive one
    assert_eq!(
        render("${if:key=name, matches='tb_user'}x${endif}", &model),
        ""
    );
    assert_eq!(
        render("${if:key=name, matches='TB_User'}x${endif}", &model),
        "x"
    );
}

#[test]
fn matches_compares_the_whole_value() {
    let model = Fixture::named("abc_def");
    assert_eq!(
        render("${if:key=name, matches='abc'}x${endif}", &model),
        "",
        "a partial match is no match"
    );
    assert_eq!(
        render("${if:key=name, matches='abc.*'}x${endif}", &model),
        "x"
    );
}

#[test]
fn the_key_of_an_if_may_carry_processors() {
    let model = Fixture::named("TB_USER");
    assert_eq!(
        render(
            "${if:key=name.lower.suffix, equals='user'}x${endif}",
            &model
        ),
        "x"
    );
}

#[test]
fn an_if_may_be_nested_inside_another_one() {
    let tpl = concat!(
        "${if:key=type, equals='TABLE'}",
        "${if:key=name, startswith='tb_'}both${else}outer${endif}",
        "${else}none${endif}"
    );
    assert_eq!(render(tpl, &row("tb_a", "TABLE")), "both");
    assert_eq!(render(tpl, &row("a", "TABLE")), "outer");
    assert_eq!(render(tpl, &row("tb_a", "VIEW")), "none");
}

// --------------------------------------------------------------------- for

#[test]
fn an_empty_collection_renders_nothing_at_all() {
    let model = Fixture::new().with_list("rows", vec![]);
    assert_eq!(
        render(
            "[${for:key=rows, instr=','}${item:key=name}${endfor}]",
            &model
        ),
        "[]"
    );
}

#[test]
fn a_skip_list_drops_every_name_it_holds() {
    let model = Fixture::new().with_list(
        "rows",
        vec![
            Fixture::named("a"),
            Fixture::named("b"),
            Fixture::named("c"),
            Fixture::named("d"),
        ],
    );
    assert_eq!(
        render(
            "${for:key=rows, instr=',', skipList='b, c'}${item:key=name}${endfor}",
            &model
        ),
        "a,d",
        "the skip list is a comma separated list, blanks around a name included"
    );
}

#[test]
fn the_separator_is_indented_to_the_column_the_loop_starts_in() {
    let model = Fixture::new().with_list("rows", vec![Fixture::named("a"), Fixture::named("b")]);
    // the loop starts behind four characters, so the second line lines up with
    // the first one plus the two extra columns of 'indent'
    assert_eq!(
        render(
            r"x\n--- ${for:key=rows, instr=',\n', indent=2}${item:key=name}${endfor}"
                .replace(r"x\n", "x\n")
                .as_str(),
            &model
        ),
        "x\n--- a,\n      b"
    );
}

#[test]
fn a_nested_loop_reaches_the_outer_model_through_super() {
    let leaf = Fixture::named("leaf");
    let child = Fixture::named("child").with_list("children", vec![leaf]);
    let parent = Fixture::named("parent").with_list("children", vec![child]);
    let model = Fixture::new().with_list("children", vec![parent]);

    let result = render(
        concat!(
            "${for:key=children}",
            "${for:key=children}",
            "${super:key=name}/${item:key=name}",
            "${endfor}${endfor}"
        ),
        &model,
    );
    assert_eq!(result, "parent/child");
}

#[test]
fn the_item_number_follows_the_position_in_the_collection() {
    let model = Fixture::new().with_list(
        "rows",
        vec![
            Fixture::named("a"),
            Fixture::named("b"),
            Fixture::named("c"),
        ],
    );
    assert_eq!(
        render(
            "${for:key=rows}${item:key=no}${item:key=name}${endfor}",
            &model
        ),
        "1a2b3c"
    );
}

#[test]
fn the_item_number_counts_the_rendered_elements_only() {
    let model = Fixture::new().with_list(
        "rows",
        vec![
            Fixture::named("a"),
            Fixture::named("b"),
            Fixture::named("c"),
            Fixture::named("d"),
        ],
    );
    // 'b' is skipped, so 'c' is the second element that is rendered - a
    // numbered column list would otherwise have a hole in it
    assert_eq!(
        render(
            "${for:key=rows, skipList='b'}${item:key=no}${item:key=name}${endfor}",
            &model
        ),
        "1a2c3d"
    );
}

#[test]
fn a_decoration_of_the_loop_body_is_applied_to_every_element() {
    let model = Fixture::new().with_list("rows", vec![Fixture::named("a"), Fixture::named("b")]);
    assert_eq!(
        render(
            "${for:key=rows, instr=', '}${item:key=name, quote=\"'\"}${endfor}",
            &model
        ),
        "'a', 'b'"
    );
}

#[test]
fn an_if_inside_a_loop_is_evaluated_per_element() {
    let model = Fixture::new().with_list(
        "rows",
        vec![row("a", "TABLE"), row("b", "VIEW"), row("c", "TABLE")],
    );
    assert_eq!(
        render(
            "${for:key=rows}${if:key=type, equals='TABLE'}${item:key=name} ${endif}${endfor}",
            &model
        ),
        "a c "
    );
}

#[test]
fn contains_looks_at_the_names_of_a_collection() {
    let model = Fixture::named("parent").with_list(
        "children",
        vec![Fixture::named("id"), Fixture::named("name")],
    );

    assert_eq!(
        render("${if:key=children, contains='ID'}x${endif}", &model),
        "x",
        "the element names are compared ignoring the case"
    );
    assert_eq!(
        render("${if:key=children, contains='other'}x${endif}", &model),
        ""
    );
    assert_eq!(
        render("${if:key=children, notcontains='other'}x${endif}", &model),
        "x"
    );
}

// ------------------------------------------------------------------ others

#[test]
fn the_line_end_of_the_template_is_used_by_the_engine_itself() {
    let model = Fixture::new().with_list("rows", vec![Fixture::named("a"), Fixture::named("b")]);

    let windows = render(
        &r"x@${for:key=rows, instr=',\n'}${item:key=name}${endfor}".replace('@', "\r\n"),
        &model,
    );
    assert_eq!(
        windows, "x\r\na,\r\nb",
        "a template written on windows keeps its carriage returns"
    );

    let unix = render(
        &r"x@${for:key=rows, instr=',\n'}${item:key=name}${endfor}".replace('@', "\n"),
        &model,
    );
    assert_eq!(unix, "x\na,\nb");
}

#[test]
fn a_template_may_be_applied_to_more_than_one_model() {
    let template = Template::parse("${name.upper}").unwrap();
    let ctx = customs();
    assert_eq!(template.render(&Fixture::named("a"), &ctx).unwrap(), "A");
    assert_eq!(template.render(&Fixture::named("b"), &ctx).unwrap(), "B");
}

#[test]
fn the_date_format_may_also_be_written_as_an_attribute() {
    let model = Fixture::named("a");
    assert_eq!(render("${date:format=yyyy}", &model), "2024");
    assert_eq!(
        render("${date:format=yyyy, prepend='[', postpend=']'}", &model),
        "[2024]"
    );
}
