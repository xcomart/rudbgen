//! The parts of jdbgen's `StrUtils` the template language is defined by.
//!
//! These are not general purpose helpers: every one of them reproduces a Java
//! method the engine's observable behaviour depends on, quirks included. The
//! quirk that matters most is [`trim`], which strips one enclosing pair of
//! quotes along with the white space - that is how a quoted attribute value
//! loses its quotes, so it is load bearing rather than cosmetic.

use unicode_width::UnicodeWidthStr;

/// Characters jdbgen counts as generic space: blank, tab, CR and LF.
const SPACE_CHARS: [char; 4] = [' ', '\t', '\r', '\n'];

/// `StrUtils.isSpace`: one of the four generic space characters.
pub(crate) fn is_space(c: char) -> bool {
    SPACE_CHARS.contains(&c)
}

/// `StrUtils.isEmpty`: nothing, or nothing but white space.
pub(crate) fn is_blank(s: &str) -> bool {
    // Java's String.trim() cuts everything up to and including the blank.
    s.chars().all(|c| c <= ' ')
}

/// Java's `String.trim()`, which cuts every character up to the blank.
pub(crate) fn java_trim(s: &str) -> &str {
    s.trim_matches(|c: char| c <= ' ')
}

/// `StrUtils.trim`: strip generic space off both ends **and** one enclosing
/// pair of quotes with it.
///
/// The unquoting is what turns `prepend='(a,b)'` into `(a,b)`; a single quote
/// character on its own is not a pair and stays.
pub(crate) fn trim(input: &str) -> &str {
    let cut = input.trim_matches(|c: char| SPACE_CHARS.contains(&c));
    let bytes = cut.as_bytes();
    if bytes.len() > 1 {
        let first = bytes[0];
        let last = bytes[bytes.len() - 1];
        if (first == b'"' || first == b'\'') && first == last {
            return &cut[1..cut.len() - 1];
        }
    }
    cut
}

/// `StrUtils.split(src, delim, true)`: split and trim every element.
///
/// jdbgen appends the delimiter before splitting, which is exactly what Rust's
/// [`str::split`] does anyway.
pub(crate) fn split_trim(src: &str, delim: char) -> Vec<&str> {
    src.split(delim).map(java_trim).collect()
}

/// `StrUtils.isUpper`: not a single lower case ASCII letter, which is how a
/// name written in one case (`USER_ID`) is told from a mixed case one.
pub(crate) fn is_upper(s: &str) -> bool {
    !s.chars().any(|c| c.is_ascii_lowercase())
}

/// `StrUtils.toCamelCase`.
pub(crate) fn to_camel_case(s: &str) -> String {
    if s.contains('_') || s.contains('-') {
        let mut out = String::with_capacity(s.len());
        let mut upper = false;
        for c in s.chars() {
            if c == '_' || c == '-' {
                upper = true;
            } else if upper {
                out.extend(c.to_uppercase());
                upper = false;
            } else {
                out.extend(c.to_lowercase());
            }
        }
        out
    } else if is_upper(s) {
        // a name in one case carries no word boundaries at all - and this is
        // also what keeps an empty name from indexing into nothing
        s.to_lowercase()
    } else {
        let mut chars = s.chars();
        // is_upper() answered false, so there is at least one character
        let first = chars.next().expect("non-empty: is_upper('') is true");
        let mut out: String = first.to_lowercase().collect();
        out.push_str(chars.as_str());
        out
    }
}

/// `StrUtils.toPascalCase`: [`to_camel_case`] with an upper case first letter.
pub(crate) fn to_pascal_case(s: &str) -> String {
    let camel = to_camel_case(s);
    let mut chars = camel.chars();
    match chars.next() {
        None => camel,
        Some(first) => {
            let mut out: String = first.to_uppercase().collect();
            out.push_str(chars.as_str());
            out
        }
    }
}

/// `StrUtils.toSnakeCase`.
pub(crate) fn to_snake_case(s: &str) -> String {
    if s.contains('_') || s.contains('-') {
        s.replace('-', "_").to_lowercase()
    } else {
        // a name in one case holds no case boundary to split at
        let owned;
        let s = if is_upper(s) {
            owned = s.to_lowercase();
            owned.as_str()
        } else {
            s
        };
        let mut out = String::with_capacity(s.len() + 4);
        for c in s.chars() {
            // no separator in front of a leading upper case character
            if c.is_ascii_uppercase() && !out.is_empty() {
                out.push('_');
            }
            out.extend(c.to_lowercase());
        }
        out
    }
}

/// `StrUtils.toScreamingSnakeCase`.
pub(crate) fn to_screaming_snake_case(s: &str) -> String {
    to_snake_case(s).to_uppercase()
}

/// `StrUtils.toSkewerCase`, also known as kebab-case.
pub(crate) fn to_skewer_case(s: &str) -> String {
    to_snake_case(s).replace('_', "-")
}

/// Width of `s` in display columns - see architecture.md §7.4.
///
/// This is [`UnicodeWidthStr::width`], the same wcwidth-compatible table a
/// terminal lays cells out by, not jdbgen's EUC-KR byte count: the two agree
/// on ASCII, Hangul and Hanja, which is every character the shipped templates
/// and golden fixtures contain, but part on what EUC-KR encodes as two bytes
/// and a terminal draws in one column (Cyrillic, Greek), and on what EUC-KR
/// cannot encode at all (emoji), where the column count is the one the
/// generated file is actually viewed in. Zero-width characters count zero.
pub(crate) fn display_width(s: &str) -> usize {
    UnicodeWidthStr::width(s)
}

/// Java's `String.equalsIgnoreCase`, which folds the case of every character
/// and not only of the ASCII ones.
pub(crate) fn eq_ignore_case(a: &str, b: &str) -> bool {
    a.eq_ignore_ascii_case(b) || a.to_lowercase() == b.to_lowercase()
}

/// `n` blanks.
pub(crate) fn spaces(n: usize) -> String {
    " ".repeat(n)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trim_strips_one_enclosing_quote_pair() {
        assert_eq!(trim("  abc  "), "abc");
        assert_eq!(trim(" 'a,b' "), "a,b");
        assert_eq!(trim("\"x\""), "x");
        // a single quote character is not a pair
        assert_eq!(trim("'"), "'");
        // mismatched quotes stay where they are
        assert_eq!(trim("'x\""), "'x\"");
        // parentheses are not quotes
        assert_eq!(trim("(a,b)"), "(a,b)");
    }

    #[test]
    fn the_case_conversions_follow_jdbgen() {
        assert_eq!(to_camel_case("user_name"), "userName");
        assert_eq!(to_camel_case("USER_NAME"), "userName");
        assert_eq!(to_camel_case("user-name"), "userName");
        assert_eq!(to_camel_case("UserName"), "userName");
        assert_eq!(to_camel_case("userName"), "userName");
        assert_eq!(to_camel_case("ABC"), "abc");
        assert_eq!(to_camel_case(""), "");
        // a trailing separator names no following character
        assert_eq!(to_camel_case("user_"), "user");

        assert_eq!(to_pascal_case("user_name"), "UserName");
        assert_eq!(to_pascal_case("USER_NAME"), "UserName");
        assert_eq!(to_pascal_case("userName"), "UserName");
        assert_eq!(to_pascal_case("ABC"), "Abc");
        assert_eq!(to_pascal_case(""), "");

        assert_eq!(to_snake_case("UserName"), "user_name");
        assert_eq!(to_snake_case("userName"), "user_name");
        assert_eq!(to_snake_case("user_name"), "user_name");
        assert_eq!(to_snake_case("user-name"), "user_name");
        assert_eq!(to_snake_case("ABC"), "abc");
        assert_eq!(to_snake_case(""), "");

        assert_eq!(to_screaming_snake_case("UserName"), "USER_NAME");
        assert_eq!(to_skewer_case("UserName"), "user-name");
    }

    #[test]
    fn is_upper_asks_for_a_lower_case_letter() {
        assert!(is_upper("USER_ID"));
        assert!(is_upper("ID2"));
        // no lower case letter at all, so nothing tells the two cases apart
        assert!(is_upper("123"));
        assert!(is_upper(""));
        assert!(!is_upper("UserId"));
        assert!(!is_upper("userid"));
    }

    #[test]
    fn a_hangul_syllable_is_two_columns_wide() {
        assert_eq!(display_width("abc"), 3);
        // two Hangul syllables, four display columns - the same count EUC-KR
        // bytes would have given, which is why the golden fixtures don't
        // notice the switch from one measure to the other
        assert_eq!(display_width("가나"), 4);
    }

    #[test]
    fn a_cyrillic_letter_is_one_column_wide_unlike_its_euckr_byte_count() {
        // EUC-KR would have encoded 'п' as two bytes, but a terminal draws it
        // in a single column
        assert_eq!(display_width("п"), 1);
    }

    #[test]
    fn an_emoji_is_two_columns_wide_though_euckr_cannot_encode_it_at_all() {
        assert_eq!(display_width("\u{1F600}"), 2);
    }

    #[test]
    fn a_combining_character_is_zero_columns_wide() {
        // 'e' followed by a combining acute accent (U+0301)
        assert_eq!(display_width("e\u{0301}"), 1);
    }
}
