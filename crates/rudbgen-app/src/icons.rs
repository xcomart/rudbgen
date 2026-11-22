//! rudbgen's own vector icons, embedded in the binary.
//!
//! gpui's [`svg`](gpui::svg) element resolves its `path` through the
//! [`AssetSource`](gpui::AssetSource) the application was built with — [`ICONS`]
//! here — and paints the result as a *monochrome* sprite: resvg rasterizes the
//! file, only the alpha channel survives, and the element's `text_color`
//! supplies the color. Two things follow, and both are why these files look
//! the way they do:
//!
//! * the colors written in an icon never reach the screen, only its coverage
//!   does, so a `fill-opacity` below `1` reads as a lighter shade of the tint;
//! * the tint is whatever the *element* asks for, and unlike text it is not
//!   inherited from a parent, so a hover that recolors a button has to reach
//!   the icon through [`group_hover`](gpui::InteractiveElement::group_hover).
//!
//! The bytes come from [`include_bytes!`], not from files read at run time: a
//! release then carries its icons wherever it is unpacked, and packaging has
//! nothing extra to ship. Cargo tracks the embedded files itself, so an edited
//! icon rebuilds the crate without help from `build.rs`.
//!
//! Only the marks that are *rudbgen's* are here. The four caption glyphs a
//! self-drawn title bar needs are the same four files in every application that
//! draws one, so they come from
//! [`rugpui_shell::WINDOW_CONTROL_ICONS`](rugpui_shell::WINDOW_CONTROL_ICONS) and
//! [`ICONS`] concatenates the two tables.

use std::path::{Path, PathBuf};

use gpui::App;
use rudbkit_core::dialects::{IconRef, icon_ref};
use rugpui::ImagePath;
use rugpui_shell::IconSet;

/// The button at the end of the tab strip that lists every open tab.
///
/// A plain chevron rather than a stack of lines: the strip's other end already
/// carries the application menu's `☰`, and two list-shaped glyphs facing each
/// other across one toolbar would read as the same control twice. A chevron
/// says "this opens downwards", which is the one thing the button does.
pub const TAB_LIST: &str = "icons/tab-list.svg";

/// The button at the end of the tab strip that opens a new connection.
///
/// Drawn with the stroke of [`TAB_LIST`] rather than a toolbar icon's: the two
/// sit shoulder to shoulder in the strip, and it is that pairing the glyph has
/// to match.
pub const NEW_TAB: &str = "icons/new-tab.svg";

/// The application mark, drawn at the left end of the custom title bar.
///
/// Reuse the shipped PNG generated from `assets/icon.svg` by `assets/render.py`.
/// [`img`](gpui::img) preserves its colors on every platform; the tintable
/// [`svg`](gpui::svg) element would reduce the filled tile to a solid silhouette.
/// The 128 px asset also covers scaled displays without a second icon to maintain.
pub const APP_ICON: &str = "icons/app-icon.png";

// --- the explorer's object marks ------------------------------------------
//
// One glyph per kind of thing a database holds, because the tree is read by
// shape before it is read by word: a schema with two hundred tables and four
// views is only scannable if a view does not look like a table. They share the
// set's 1.8 stroke and 24×24 box, and they are drawn as outlines rather than
// filled shapes so that the theme's tint is what colors them.

/// A base table.
pub const TABLE: &str = "icons/table.svg";

/// A view.
///
/// The table's frame, dashed — a view has the shape of a table without being
/// one — with an eye inside it. The dashes are what tells the two apart at a
/// glance down a long list, which is the only moment this distinction matters.
pub const VIEW: &str = "icons/view.svg";

/// One of the folders a schema is divided into.
pub const FOLDER: &str = "icons/folder.svg";

/// A schema, and — where a product has them — a catalog.
///
/// The database cylinder, the same solid [`APP_ICON`] draws, because that is what
/// the level *is*: everything under it is one database's contents.
pub const SCHEMA: &str = "icons/schema.svg";

/// A stored procedure.
pub const PROCEDURE: &str = "icons/procedure.svg";

/// A stored function.
///
/// The mathematician's *f*, not the procedure's arrow: a function is asked for
/// a value and a procedure is told to do something, and the glyphs say which.
pub const FUNCTION: &str = "icons/function.svg";

/// A sequence.
pub const SEQUENCE: &str = "icons/sequence.svg";

/// Reloads whatever the panel is showing.
pub const REFRESH: &str = "icons/refresh.svg";

/// Copies the panel's text to the clipboard.
pub const COPY: &str = "icons/copy.svg";

/// Shows and hides the explorer.
pub const SIDEBAR: &str = "icons/sidebar.svg";

/// The explorer tree's disclosure mark on a closed node.
///
/// The tree used to draw `▸` and `▾` as text, and no font size rescued them:
/// those code points fill a fraction of their em square, so the mark that
/// reached the screen was a third of the space it was given and its direction
/// was a couple of antialiased pixels. A chevron drawn as geometry spans the
/// box it is handed, and the two directions differ by half the glyph.
///
/// Like the window controls, it carries a heavier stroke than the set's usual
/// `1.8` — `2.4` here. The tree renders it at 14 px (`ARROW_ICON_SIZE` in
/// [`rugpui::tree`]) out of the 24 of the viewBox, so what reaches the
/// screen is a little over half of what the file asks for: 1.4 px, which a row
/// of pixels can hold, where `1.8` would have arrived as the same hairline the
/// glyph was.
pub const CHEVRON_RIGHT: &str = "icons/chevron-right.svg";

/// The explorer tree's disclosure mark on an open node — [`CHEVRON_RIGHT`]
/// turned a quarter turn, so that opening a node reads as the mark tipping over
/// rather than as a different mark.
pub const CHEVRON_DOWN: &str = "icons/chevron-down.svg";

// --- driver marks ------------------------------------------------------------

/// The mark of a driver whose product this build has no logo for: a database
/// cylinder.
pub const GENERIC_DRIVER: &str = "icons/drivers/generic.png";

/// The products' own logos, by the stem a driver definition's `icon` names,
/// drawn beside a driver in the connection dialog's picker and the driver
/// manager's list. Unlike the glyphs above they are color images, drawn with
/// [`img`](gpui::img) as they are: a tinted `svg` would make every product's
/// mark the same gray shape. Each is a 64 px square with the logo trimmed and
/// centered; `../../../assets/drivers/README.md` says where each came from.
const DRIVER_MARKS: &[(&str, &[u8])] = &[
    (
        "icons/drivers/altibase.png",
        include_bytes!("../../../assets/drivers/altibase.png"),
    ),
    (
        "icons/drivers/cockroachdb.png",
        include_bytes!("../../../assets/drivers/cockroachdb.png"),
    ),
    (
        "icons/drivers/cubrid.png",
        include_bytes!("../../../assets/drivers/cubrid.png"),
    ),
    (
        "icons/drivers/db2.png",
        include_bytes!("../../../assets/drivers/db2.png"),
    ),
    (
        "icons/drivers/firebird.png",
        include_bytes!("../../../assets/drivers/firebird.png"),
    ),
    (
        "icons/drivers/generic.png",
        include_bytes!("../../../assets/drivers/generic.png"),
    ),
    (
        "icons/drivers/h2.png",
        include_bytes!("../../../assets/drivers/h2.png"),
    ),
    (
        "icons/drivers/mariadb.png",
        include_bytes!("../../../assets/drivers/mariadb.png"),
    ),
    (
        "icons/drivers/mongodb.png",
        include_bytes!("../../../assets/drivers/mongodb.png"),
    ),
    (
        "icons/drivers/mssql.png",
        include_bytes!("../../../assets/drivers/mssql.png"),
    ),
    (
        "icons/drivers/mysql.png",
        include_bytes!("../../../assets/drivers/mysql.png"),
    ),
    (
        "icons/drivers/oracle.png",
        include_bytes!("../../../assets/drivers/oracle.png"),
    ),
    (
        "icons/drivers/postgresql.png",
        include_bytes!("../../../assets/drivers/postgresql.png"),
    ),
    (
        "icons/drivers/sqlite.png",
        include_bytes!("../../../assets/drivers/sqlite.png"),
    ),
    (
        "icons/drivers/tidb.png",
        include_bytes!("../../../assets/drivers/tidb.png"),
    ),
];

/// The images `driver` may be drawn with, best first.
///
/// A driver on a dialect profile wears the icon the profile's `driver`
/// section names, so that a definition offered by an older version of the
/// profile follows the newer's; then the definition's own `icon`; then the
/// mark of the built-in dialect it names. An icon is the name of a mark this
/// build ships or an image file ([`rudbkit_core::dialects::icon_ref`]), a
/// relative one taken from `profiles`, the profiles' folder. A name this build
/// has no mark for gives way to the next, and so does `generic`, which names
/// no product: a Db2 definition from before Db2 had a mark gets it.
fn driver_images(driver: &rudbgen_core::DriverDef, profiles: &Path) -> Vec<ImagePath> {
    let report = rudbgen_core::dialects::current();
    let profile = report
        .entry_for(&driver.dialect)
        .filter(|entry| entry.failure().is_none())
        .and_then(|entry| entry.driver.as_ref()?.icon.clone());
    let dialect = rudbkit_core::dialects::built_in(&driver.dialect).map(|dialect| match dialect {
        "postgres" => "postgresql",
        other => other,
    });
    [profile.as_deref(), driver.icon.as_deref(), dialect]
        .into_iter()
        .flatten()
        .filter_map(|icon| match icon_ref(icon, profiles) {
            IconRef::File(path) => Some(ImagePath::from(path)),
            IconRef::Mark("generic") => None,
            IconRef::Mark(stem) => driver_mark(stem).map(ImagePath::from),
        })
        .collect()
}

/// The image `driver` is drawn with: the first of its images (see
/// [`driver_images`]) that can be read, or [`GENERIC_DRIVER`].
pub fn driver_icon(driver: &rudbgen_core::DriverDef, cx: &mut App) -> ImagePath {
    let profiles = rudbgen_core::dialects_dir().unwrap_or_default();
    driver_images(driver, &profiles)
        .into_iter()
        .find(|image| rugpui::image_loads(image, cx))
        .unwrap_or_else(|| ImagePath::from(GENERIC_DRIVER))
}

/// An image file `driver`'s icon names that cannot be read as an image: what
/// the driver manager says, since another mark is drawn in its place.
pub fn unreadable_icon(driver: &rudbgen_core::DriverDef, cx: &mut App) -> Option<PathBuf> {
    let profiles = rudbgen_core::dialects_dir().unwrap_or_default();
    driver_images(driver, &profiles)
        .into_iter()
        .find_map(|image| match &image {
            ImagePath::File(path) if !rugpui::image_loads(&image, cx) => Some(path.to_path_buf()),
            _ => None,
        })
}

/// The asset path of the mark whose stem is `stem`, if this build has it.
fn driver_mark(stem: &str) -> Option<&'static str> {
    DRIVER_MARKS.iter().map(|(path, _)| *path).find(|path| {
        path.strip_prefix("icons/drivers/")
            .and_then(|file| file.strip_suffix(".png"))
            == Some(stem)
    })
}

/// rudbgen's own icons, paired with the bytes [`ICONS`] hands back for them.
const APP_ICONS: &[(&str, &[u8])] = &[
    (APP_ICON, include_bytes!("../../../assets/icon-128.png")),
    (TAB_LIST, include_bytes!("../assets/icons/tab-list.svg")),
    (NEW_TAB, include_bytes!("../assets/icons/new-tab.svg")),
    (TABLE, include_bytes!("../assets/icons/table.svg")),
    (VIEW, include_bytes!("../assets/icons/view.svg")),
    (FOLDER, include_bytes!("../assets/icons/folder.svg")),
    (SCHEMA, include_bytes!("../assets/icons/schema.svg")),
    (PROCEDURE, include_bytes!("../assets/icons/procedure.svg")),
    (FUNCTION, include_bytes!("../assets/icons/function.svg")),
    (SEQUENCE, include_bytes!("../assets/icons/sequence.svg")),
    (REFRESH, include_bytes!("../assets/icons/refresh.svg")),
    (COPY, include_bytes!("../assets/icons/copy.svg")),
    (SIDEBAR, include_bytes!("../assets/icons/sidebar.svg")),
    (
        CHEVRON_RIGHT,
        include_bytes!("../assets/icons/chevron-right.svg"),
    ),
    (
        CHEVRON_DOWN,
        include_bytes!("../assets/icons/chevron-down.svg"),
    ),
];

/// The asset source backing the app's embedded images and SVG glyphs.
///
/// Three tables: the shell's caption glyphs, the disclosure carets the widget
/// kit draws for a collapsible, a tree or a dropdown that has not been given
/// an icon of its own, and rudbgen's own marks. Install it with
/// [`Application::with_assets`](gpui::Application::with_assets); without it
/// gpui's default source answers every path with `None` and the icons paint as
/// nothing at all — and leaving [`rugpui::ICONS`] out of the chain does the
/// same to the arrows alone, which is what `rugpui::init` warns about.
pub const ICONS: IconSet = IconSet::new(&[
    rugpui_shell::WINDOW_CONTROL_ICONS,
    rugpui::ICONS,
    APP_ICONS,
    DRIVER_MARKS,
]);

#[cfg(test)]
mod tests {
    use gpui::AssetSource;

    use super::*;

    #[test]
    fn every_icon_loads_and_matches_its_extension() {
        for (name, _) in ICONS.all() {
            let bytes = ICONS
                .load(name)
                .expect("loading an embedded icon cannot fail")
                .unwrap_or_else(|| panic!("{name} is missing from the asset source"));
            // The application mark is a color image; the other icons are
            // 24×24 SVG glyphs that can be tinted to match the theme.
            if name.ends_with(".png") {
                assert!(
                    bytes.starts_with(b"\x89PNG\r\n\x1a\n"),
                    "{name} is not a PNG"
                );
                continue;
            }
            let text = std::str::from_utf8(&bytes).expect("an icon must be UTF-8");
            assert!(text.contains("<svg"), "{name} is not an SVG");
            assert!(
                text.contains("viewBox=\"0 0 24 24\""),
                "{name} has the wrong viewBox"
            );
        }
    }

    #[test]
    fn every_driver_mark_is_a_png_and_every_built_in_has_one() {
        for (name, bytes) in DRIVER_MARKS {
            assert!(
                bytes.starts_with(b"\x89PNG\r\n\x1a\n"),
                "{name} is not a PNG"
            );
        }
        for driver in rudbgen_core::DriverDef::builtins() {
            let icon = driver
                .icon
                .as_deref()
                .expect("every built-in names an icon");
            assert!(driver_mark(icon).is_some(), "{}: no mark {icon}", driver.id);
        }
    }

    fn driver(icon: Option<&str>, dialect: &str) -> rudbgen_core::DriverDef {
        rudbgen_core::DriverDef {
            icon: icon.map(str::to_string),
            dialect: dialect.to_string(),
            ..rudbgen_core::DriverDef::default()
        }
    }

    #[test]
    fn a_driver_without_a_mark_of_its_own_takes_its_dialects() {
        let base = std::path::Path::new("/config/dialects");
        let images = |icon, dialect| driver_images(&driver(icon, dialect), base);
        let mark = |stem: &'static str| ImagePath::from(stem);
        assert_eq!(
            images(Some("oracle"), "generic"),
            [mark("icons/drivers/oracle.png")]
        );
        assert_eq!(
            images(None, "PostgreSQL"),
            [mark("icons/drivers/postgresql.png")]
        );
        assert_eq!(
            images(Some("nothing"), "mssql"),
            [mark("icons/drivers/mssql.png")]
        );
        assert_eq!(images(None, "generic"), []);
        assert_eq!(images(None, "nosuch"), []);
        assert_eq!(
            images(Some("generic"), "db2"),
            [mark("icons/drivers/db2.png")]
        );
        assert_eq!(
            images(Some("acme.svg"), "mysql"),
            [
                ImagePath::from(base.join("acme.svg")),
                mark("icons/drivers/mysql.png")
            ],
            "a file, from the profiles' folder, before the dialect's mark"
        );
    }

    /// An icon file that is an image is drawn; one that is not gives way, and
    /// is what the driver manager reports.
    #[gpui::test]
    fn an_icon_file_is_drawn_or_reported(cx: &mut gpui::TestAppContext) {
        let dir = tempfile::tempdir().expect("tempdir");
        let good = dir.path().join("good.svg");
        std::fs::write(
            &good,
            "<svg xmlns='http://www.w3.org/2000/svg' width='8' height='8'><rect width='8' height='8'/></svg>",
        )
        .expect("write");
        let bad = dir.path().join("bad.png");
        std::fs::write(&bad, "not an image").expect("write");
        let good_icon = good.display().to_string();
        let bad_icon = bad.display().to_string();
        cx.update(|cx| {
            let drawn = driver_icon(&driver(Some(&good_icon), "nosuch"), cx);
            assert_eq!(drawn, ImagePath::from(good.clone()));
            assert_eq!(
                unreadable_icon(&driver(Some(&good_icon), "nosuch"), cx),
                None
            );

            let fallback = driver_icon(&driver(Some(&bad_icon), "nosuch"), cx);
            assert_eq!(fallback, ImagePath::from(GENERIC_DRIVER));
            assert_eq!(
                unreadable_icon(&driver(Some(&bad_icon), "nosuch"), cx),
                Some(bad.clone())
            );
        });
    }

    #[test]
    fn an_unknown_path_is_not_an_error() {
        assert!(
            ICONS
                .load("icons/nothing.svg")
                .expect("a missing asset is not a failure")
                .is_none()
        );
    }

    #[test]
    fn listing_returns_both_tables() {
        assert_eq!(ICONS.list("icons/").unwrap().len(), ICONS.len());
        // rudbgen's own marks, the products' logos, the four caption glyphs the
        // shell owns, and the widget kit's two disclosure carets.
        assert_eq!(
            ICONS.len(),
            APP_ICONS.len() + DRIVER_MARKS.len() + 4 + rugpui::ICONS.len()
        );
    }

    /// The widget kit resolves its default arrows through the host's asset
    /// source, so dropping [`rugpui::ICONS`] from the chain would leave every
    /// collapsible and tree with an invisible disclosure mark.
    #[test]
    fn the_widget_kit_carets_are_in_the_chain() {
        for path in [rugpui::CARET_RIGHT, rugpui::CARET_DOWN] {
            assert!(
                ICONS
                    .load(path)
                    .expect("loading an embedded icon cannot fail")
                    .is_some(),
                "{path} is missing from the asset source"
            );
        }
    }
}
