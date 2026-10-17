//! What a template cannot ask of the engine, however it is written.
//!
//! Everything here used to take the whole process down rather than fail: a
//! stack overflow and an allocation that cannot be served both abort, and the
//! live preview of the template editor parses and renders on every keystroke.

mod common;

use common::{Fixture, context, render};
use rudbgen_template::Template;

/// The nesting limit of `if`s and `for`s.
const MAX_DEPTH: usize = 64;

/// A model with a name and a one element collection, so that a loop renders
/// its body once.
fn model() -> Fixture {
    Fixture::named("tb_user").with_list("rows", vec![Fixture::named("a")])
}

/// A model whose collection holds a model like it, `depth` times over, so
/// that `depth` nested loops all have something to walk.
fn deep_model(depth: usize) -> Fixture {
    let mut model = Fixture::named("a");
    for _ in 0..depth {
        model = Fixture::named("a").with_list("rows", vec![model]);
    }
    model
}

/// `depth` levels of `if` and `for` around one `${name}`: every `stride`th
/// level a `for`, the others an `if`.
fn nested(depth: usize, stride: usize) -> String {
    let is_for = |level: usize| level % stride == stride - 1;
    let mut source = String::new();
    for level in 0..depth {
        source.push_str(if is_for(level) {
            "${for:key=rows}"
        } else {
            "${if:key=name, notEquals='x'}"
        });
    }
    source.push_str("${name}");
    for level in (0..depth).rev() {
        source.push_str(if is_for(level) {
            "${endfor}"
        } else {
            "${endif}"
        });
    }
    source
}

#[test]
fn statements_nested_up_to_the_limit_parse_and_render() {
    // on the test thread's default stack, in the unoptimised build: the case
    // the limit is measured against
    for stride in [1, 2, usize::MAX] {
        let template =
            Template::parse(&nested(MAX_DEPTH, stride)).expect("deep but within the limit");
        assert_eq!(
            template.render(&deep_model(MAX_DEPTH), &context()).unwrap(),
            "a"
        );
        assert_eq!(template.spans().len(), MAX_DEPTH + 1);
    }
}

#[test]
fn statements_nested_beyond_the_limit_are_a_parse_error() {
    let error = Template::parse(&nested(MAX_DEPTH + 1, 2)).expect_err("one level too deep");
    assert!(error.message.contains("nested more than 64"), "{error}");
    assert!(
        error.span.is_some(),
        "the statement that went too deep is marked"
    );
}

#[test]
fn tens_of_thousands_of_unclosed_ifs_are_a_parse_error_and_not_a_crash() {
    let source = "${if:key=name, equals='x'}".repeat(50_000);
    let error = Template::parse(&source).expect_err("nothing is closed");
    assert!(error.message.contains("nested more than"), "{error}");

    let source = "${for:key=rows}".repeat(50_000);
    let error = Template::parse(&source).expect_err("nothing is closed");
    assert!(error.message.contains("nested more than"), "{error}");
}

#[test]
fn a_long_elif_chain_is_not_a_deep_one() {
    // a hundred thousand elifs are siblings, not levels: they parse, render,
    // list their spans and are dropped without recursing once per elif
    const ELIFS: usize = 100_000;
    let mut source = String::from("${if:key=name, equals='nope'}0");
    for n in 1..=ELIFS {
        source.push_str(&format!("${{elif:key=name, equals='tb_{n}'}}{n}"));
    }
    source.push_str("${else}none${endif}");

    let template = Template::parse(&source).expect("an elif chain has no depth");
    assert_eq!(template.spans().len(), 2 * (ELIFS + 1) + 1);
    let last = Fixture::named(&format!("tb_{ELIFS}"));
    assert_eq!(
        template.render(&last, &context()).unwrap(),
        ELIFS.to_string()
    );
    assert_eq!(template.render(&model(), &context()).unwrap(), "none");
    drop(template);
}

#[test]
fn an_else_before_an_elif_still_loses_its_body() {
    // jdbgen's oddity survives the elif chain becoming a list
    let source = "${if:key=name, equals='x'}if${else}else${elif:key=name, equals='y'}elif${endif}";
    assert_eq!(render(source, &model()), "");
    assert_eq!(render(source, &Fixture::named("y")), "elif");
}

#[test]
fn a_width_up_to_the_limit_is_honoured() {
    let padded = render("[${item:key=name, padSize=4096}]", &model());
    assert_eq!(padded.len(), 4096 + 2);
    assert_eq!(
        render(
            "${for:key=rows, indent=4096, inStr='\\n'}x${endfor}",
            &model()
        ),
        "x",
        "one element writes no separator"
    );
    assert_eq!(
        render("[${item:key=name, padSize=-99999999999}]", &model()),
        "[tb_user]",
        "a negative width writes no blanks and costs nothing"
    );
}

#[test]
fn a_width_beyond_the_limit_is_a_parse_error_and_not_an_abort() {
    for source in [
        "${item:key=name, padSize=4097}",
        "${item:key=name, padSize=9999999999}",
        "${user:padSize=9999999999}",
        "${for:key=rows, indent=9999999999}x${endfor}",
    ] {
        let error = Template::parse(source).expect_err(source);
        assert!(error.message.contains("is too large"), "{source}: {error}");
        assert!(error.span.is_some(), "{source}: the statement is marked");
    }
}
