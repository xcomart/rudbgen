//! What a template renders against.
//!
//! jdbgen reads its values off a Java object by reflection; here a model
//! answers one key at a time. The key is always a single name - `${a.b}` is a
//! processor chain, never a nested path - so a model never has to walk a path.

use std::borrow::Cow;
use std::collections::HashMap;

/// One value a [`Model`] answers with.
///
/// [`Value::Null`] means the same as no answer at all: jdbgen cannot tell a
/// field that is `null` from a field that is not there, and neither can a
/// template.
pub enum Value<'a> {
    /// Text, borrowed from the model wherever that is possible.
    Str(Cow<'a, str>),
    /// A whole number, rendered in decimal.
    Int(i64),
    /// A flag, rendered as `true` or `false`.
    Bool(bool),
    /// A collection, which is what `${for}` walks and what `contains` looks
    /// into.
    List(Vec<&'a dyn Model>),
    /// No value, which is indistinguishable from an unknown key.
    Null,
}

impl<'a> Value<'a> {
    /// The value as text, the way jdbgen's `String.valueOf` would write it.
    ///
    /// A collection has no text form in jdbgen either - Java would print the
    /// element objects - so the element names are written instead, which is
    /// the only part of an element a template can name in that position.
    pub fn to_text(&self) -> Cow<'_, str> {
        match self {
            Value::Str(s) => Cow::Borrowed(s.as_ref()),
            Value::Int(i) => Cow::Owned(i.to_string()),
            Value::Bool(b) => Cow::Borrowed(if *b { "true" } else { "false" }),
            Value::List(items) => {
                let names: Vec<String> = items
                    .iter()
                    .map(|m| match m.get("name") {
                        Some(v) => v.to_text().into_owned(),
                        None => String::new(),
                    })
                    .collect();
                Cow::Owned(format!("[{}]", names.join(", ")))
            }
            Value::Null => Cow::Borrowed(""),
        }
    }

    /// Whether this is [`Value::Null`], which counts as no answer.
    pub fn is_null(&self) -> bool {
        matches!(self, Value::Null)
    }
}

impl std::fmt::Debug for Value<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Value::Str(s) => write!(f, "Str({s:?})"),
            Value::Int(i) => write!(f, "Int({i})"),
            Value::Bool(b) => write!(f, "Bool({b})"),
            Value::List(items) => write!(f, "List(<{} elements>)", items.len()),
            Value::Null => write!(f, "Null"),
        }
    }
}

/// A table, a column or anything else a template is rendered against.
///
/// The loop counter of `${for}` is **not** part of this trait: the renderer
/// hands `no` to the element it is rendering, so a model needs no interior
/// mutability and a whole metadata tree can be rendered through a shared
/// reference. A model that answers `no` itself is still asked outside of a
/// loop.
pub trait Model {
    /// The value of `key`, or nothing when this model has no such member.
    fn get(&self, key: &str) -> Option<Value<'_>>;
}

impl Model for HashMap<String, String> {
    fn get(&self, key: &str) -> Option<Value<'_>> {
        HashMap::get(self, key).map(|v| Value::Str(Cow::Borrowed(v.as_str())))
    }
}

impl<T: Model + ?Sized> Model for &T {
    fn get(&self, key: &str) -> Option<Value<'_>> {
        (**self).get(key)
    }
}

impl<T: Model + ?Sized> Model for Box<T> {
    fn get(&self, key: &str) -> Option<Value<'_>> {
        (**self).get(key)
    }
}
