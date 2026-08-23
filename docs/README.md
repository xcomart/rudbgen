# rudbgen documentation

rudbgen reads a database's metadata through its JDBC driver and renders it
through templates. These pages cover how to install it, how to drive it, and
the language the templates are written in.

| Page | What it covers |
|:---|:---|
| [Installation](installation.md) | Downloads for the three platforms, where your data is kept, the keychain, updating and uninstalling |
| [User interface guide](ui-guide.md) | The window, the explorer, the inspector, the Generate tab, preview and dry run, the shortcuts |
| [Template reference](template-reference.md) | The template language: statements, processors, the model's fields, error handling, recipes |
| [Custom queries](custom-queries.md) | Your own metadata SQL, for a driver whose `DatabaseMetaData` is wrong or missing |

For how rudbgen is built rather than used:

| Page | What it covers |
|:---|:---|
| [Architecture](architecture.md) | The design: the decisions, the crate layout, the JNI bridge, the template engine, the milestones |
| [Progress and handoff](status.md) | How far the work has come, what is left, and how work is done in this repository |

## Where things come from

rudbgen is two projects joined at the seam.

- **The template language, the metadata contracts and the type mapping** are
  [jdbgen](https://github.com/xcomart/jdbgen)'s, ported to Rust. jdbgen's engine
  tests are ported case for case and its three shipped templates must render
  byte for byte identically, so a template written for jdbgen renders here
  unchanged. Where the port deliberately differs — three places — the reference
  pages say so and say why.
- **The window, the JDBC bridge, the connection handling and the SSH tunnels**
  are [rudbman](https://github.com/xcomart/rudbman)'s, which is where the
  embedded-JVM design and the packaging come from.

What is new is everything in between: one window instead of a stack of modal
dialogs, no master password, foreign keys and indexes in the model the templates
see, a preview of what a run will write, and a run you can cancel.
