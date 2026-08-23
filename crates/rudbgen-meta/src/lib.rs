//! The table model: `DESCRIBE` and the four custom queries turned into
//! something a template renders against.
//!
//! ```no_run
//! use rudbgen_core::DriverDef;
//! use rudbgen_meta::MetaReader;
//! use rudbgen_template::{RenderContext, Template};
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! # let session: rudbgen_jdbc::Session = unimplemented!();
//! # let driver: DriverDef = DriverDef::default();
//! let reader = MetaReader::new(&session, &driver);
//! let schema = reader.schemas()?.remove(0);
//! for entry in reader.tables(&schema, false)? {
//!     let table = reader.table(&entry)?;
//!     let template = Template::parse("class ${name.pascal} { }")?;
//!     print!("{}", template.render(&table, &RenderContext::new())?);
//! }
//! # Ok(()) }
//! ```
//!
//! # Where this sits
//!
//! It knows `rudbgen-jdbc` (the bridge), `rudbgen-core` (the driver
//! definition the custom queries live in) and `rudbgen-template` (the trait it
//! implements). It does **not** know gpui, and must not learn it: everything
//! here is tested against a real H2 through a real JVM without a window, which
//! is the whole reason the model is a crate of its own (architecture.md §3).
//!
//! Nothing here caches. Every call is a round trip, and the application owns
//! the cache — only it knows what a refresh means and which connection a
//! cached table belongs to.
//!
//! # jdbgen compatibility
//!
//! A template written for jdbgen has to render against this model unedited, so
//! **jdbgen's member names are kept even where Rust would spell them
//! differently**: `type`, `notKeys`, `isKey`, `typeName`, `typeString`,
//! `defVal`, `dataType`, `jdbcType`, `javaType`. So are the derivations behind
//! them, including the ones that look like mistakes — `TIMESTAMP` maps to
//! `String`, `DECIMAL` to `Integer`, a `CLOB` gets no length, and `IS_KEY` is
//! read as a *number*, so a custom query answering `'Y'` marks no key. See
//! [`sqltypes`] and [`query::to_int`] for why each one stays.
//!
//! Two things are deliberately not jdbgen's:
//!
//! * **The model is richer** (architecture.md D8). `imports`, `exports`,
//!   `indexes`, `precision`, `scale`, `autoIncrement` and a column's `fk` are
//!   new. Not one jdbgen field changed meaning to make room for them.
//! * **A column-comment query does not erase the comments it does not
//!   return** (architecture.md §6). jdbgen overwrote every comment with the
//!   query's answer, so a query that names one column blanked all the others.
//!
//! The smaller departures are documented where they are made:
//! [`MetaReader::schemas`] on the placeholder catalog, [`MetaReader::tables`]
//! on a table list that answers no catalog of its own, and
//! [`MetaReader::table`] on filtering a column list that a wildcard widened.

#![warn(missing_docs)]

mod error;
pub mod model;
pub mod query;
mod reader;
pub mod sqltypes;

pub use error::{Error, Result};
pub use model::{
    Column, ForeignKey, ForeignKeyRef, Index, KIND_TABLE, KIND_VIEW, KeyColumn, Schema, Table,
    TableRef, table_kind,
};
pub use query::{substitute, to_int};
pub use reader::{DEFAULT_CATALOG, DEFAULT_SCHEMA, MetaReader};
