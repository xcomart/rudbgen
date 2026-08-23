//! The models the ported jdbgen tests are rendered against.
//!
//! jdbgen reaches its values by reflection and its tests therefore use a
//! handful of throwaway classes and a `HashMap`. One model type covers all of
//! them here: a list of named fields, each of which is text, a number or a
//! collection of further models.

#![allow(dead_code)]

use std::borrow::Cow;

use rudbgen_template::{Model, RenderContext, Template, Value};

/// One field of a [`Fixture`].
pub enum Field {
    /// Text.
    Text(String),
    /// A number, which is what a `no` or a `nullable` column flag is.
    Int(i64),
    /// A collection, which `${for}` walks.
    List(Vec<Fixture>),
    /// A member that is there but holds nothing, which is jdbgen's `null`.
    Null,
}

/// A model of named fields.
#[derive(Default)]
pub struct Fixture {
    fields: Vec<(String, Field)>,
}

impl Fixture {
    pub fn new() -> Fixture {
        Fixture::default()
    }

    /// A model whose `name` is `name`, which is what most fixtures are.
    pub fn named(name: &str) -> Fixture {
        Fixture::new().with("name", name)
    }

    pub fn with(mut self, key: &str, value: &str) -> Fixture {
        self.fields
            .push((key.to_string(), Field::Text(value.to_string())));
        self
    }

    pub fn with_int(mut self, key: &str, value: i64) -> Fixture {
        self.fields.push((key.to_string(), Field::Int(value)));
        self
    }

    pub fn with_null(mut self, key: &str) -> Fixture {
        self.fields.push((key.to_string(), Field::Null));
        self
    }

    pub fn with_list(mut self, key: &str, items: Vec<Fixture>) -> Fixture {
        self.fields.push((key.to_string(), Field::List(items)));
        self
    }
}

impl Model for Fixture {
    fn get(&self, key: &str) -> Option<Value<'_>> {
        let (_, field) = self.fields.iter().find(|(name, _)| name == key)?;
        Some(match field {
            Field::Text(text) => Value::Str(Cow::Borrowed(text)),
            Field::Int(value) => Value::Int(*value),
            Field::Null => Value::Null,
            Field::List(items) => {
                Value::List(items.iter().map(|item| item as &dyn Model).collect())
            }
        })
    }
}

/// A model with the `name` and `type` of jdbgen's syntax test `Row`.
pub fn row(name: &str, kind: &str) -> Fixture {
    Fixture::named(name).with("type", kind)
}

/// The context the ported tests render with: the `author` custom variable of
/// the Java tests, a fixed clock and a fixed login user.
pub fn context() -> RenderContext {
    RenderContext::new()
        .with_var("author", "John Doe")
        .with_user("tester")
        .with_now(
            rudbgen_template::chrono::NaiveDate::from_ymd_opt(2024, 3, 7)
                .unwrap()
                .and_hms_opt(13, 45, 6)
                .unwrap(),
        )
}

/// Parse and render in one step, the way the Java tests do.
pub fn render(template: &str, model: &dyn Model) -> String {
    render_with(template, model, &context())
}

/// Parse and render with a context of your own.
pub fn render_with(template: &str, model: &dyn Model, ctx: &RenderContext) -> String {
    Template::parse(template)
        .unwrap_or_else(|e| panic!("template must parse: {e}"))
        .render(model, ctx)
        .unwrap_or_else(|e| panic!("template must render: {e}"))
}
