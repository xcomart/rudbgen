//! `${date}`: Java's `SimpleDateFormat` patterns on top of `chrono`.
//!
//! Only the pattern letters are reproduced, not the locale: month and weekday
//! names are written in English whatever the machine is set to, where Java
//! would follow the default locale. Every template that ships with jdbgen uses
//! numeric fields only, where the two agree exactly.

use chrono::{Datelike, NaiveDate, NaiveDateTime, Timelike};

const MONTHS: [&str; 12] = [
    "January",
    "February",
    "March",
    "April",
    "May",
    "June",
    "July",
    "August",
    "September",
    "October",
    "November",
    "December",
];

const WEEKDAYS: [&str; 7] = [
    "Sunday",
    "Monday",
    "Tuesday",
    "Wednesday",
    "Thursday",
    "Friday",
    "Saturday",
];

/// Format `when` with a `SimpleDateFormat` pattern.
///
/// The error is the message of the `IllegalArgumentException` Java would have
/// thrown, so that a template with a broken format says the same thing here.
pub(crate) fn format(pattern: &str, when: &NaiveDateTime) -> Result<String, String> {
    let chars: Vec<char> = pattern.chars().collect();
    let mut out = String::with_capacity(pattern.len() + 8);
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c == '\'' {
            // '' is a single quote, '...' is literal text
            i += 1;
            if i < chars.len() && chars[i] == '\'' {
                out.push('\'');
                i += 1;
                continue;
            }
            while i < chars.len() && chars[i] != '\'' {
                out.push(chars[i]);
                i += 1;
            }
            i += 1; // the closing quote
        } else if c.is_ascii_alphabetic() {
            let start = i;
            while i < chars.len() && chars[i] == c {
                i += 1;
            }
            out.push_str(&field(c, i - start, when)?);
        } else {
            out.push(c);
            i += 1;
        }
    }
    Ok(out)
}

/// One pattern field, `count` letters wide.
fn field(letter: char, count: usize, when: &NaiveDateTime) -> Result<String, String> {
    let date = when.date();
    Ok(match letter {
        'G' => (if date.year() > 0 { "AD" } else { "BC" }).to_string(),
        'y' => year(date.year(), count),
        'Y' => year(week_year(date), count),
        'M' | 'L' => month(date.month(), count),
        'w' => number(week_of_year(date) as i64, count),
        'W' => number(week_of_month(date) as i64, count),
        'D' => number(date.ordinal() as i64, count),
        'd' => number(date.day() as i64, count),
        'F' => number(((date.day() - 1) / 7 + 1) as i64, count),
        'E' => weekday(date, count),
        'u' => number(date.weekday().number_from_monday() as i64, count),
        'a' => (if when.hour() < 12 { "AM" } else { "PM" }).to_string(),
        'H' => number(when.hour() as i64, count),
        'k' => number(
            if when.hour() == 0 {
                24
            } else {
                when.hour() as i64
            },
            count,
        ),
        'K' => number((when.hour() % 12) as i64, count),
        'h' => {
            let hour = when.hour() % 12;
            number(if hour == 0 { 12 } else { hour as i64 }, count)
        }
        'm' => number(when.minute() as i64, count),
        's' => number(when.second() as i64, count),
        'S' => number((when.nanosecond() / 1_000_000) as i64, count),
        'z' | 'Z' | 'X' => {
            return Err(format!(
                "Unsupported pattern character '{letter}': ${{date}} carries no time zone"
            ));
        }
        other => return Err(format!("Illegal pattern character '{other}'")),
    })
}

/// A number, zero padded to `count` digits.
fn number(value: i64, count: usize) -> String {
    format!("{value:0count$}")
}

/// A year: two letters ask for the last two digits, anything else pads.
fn year(value: i32, count: usize) -> String {
    if count == 2 {
        format!("{:02}", value.rem_euclid(100))
    } else {
        number(value as i64, count)
    }
}

fn month(month: u32, count: usize) -> String {
    let name = MONTHS[(month - 1) as usize];
    match count {
        0..=2 => number(month as i64, count),
        3 => name[..3].to_string(),
        _ => name.to_string(),
    }
}

fn weekday(date: NaiveDate, count: usize) -> String {
    let name = WEEKDAYS[date.weekday().num_days_from_sunday() as usize];
    if count <= 3 {
        name[..3].to_string()
    } else {
        name.to_string()
    }
}

/// The week-year of `date` under Java's default calendar rules: a week runs
/// from Sunday to Saturday and week one is the week holding January 1st, so a
/// date in the last days of December already belongs to the next year when its
/// Saturday does.
fn week_year(date: NaiveDate) -> i32 {
    let to_saturday = 6 - date.weekday().num_days_from_sunday() as i64;
    let saturday = date + chrono::Duration::days(to_saturday);
    if saturday.year() > date.year() {
        date.year() + 1
    } else {
        date.year()
    }
}

/// Week of the year, counted from the week holding January 1st.
fn week_of_year(date: NaiveDate) -> u32 {
    if week_year(date) > date.year() {
        return 1;
    }
    let jan1 = NaiveDate::from_ymd_opt(date.year(), 1, 1).expect("january exists");
    let offset = jan1.weekday().num_days_from_sunday();
    (date.ordinal() - 1 + offset) / 7 + 1
}

/// Week of the month, counted the same way from the first of the month.
fn week_of_month(date: NaiveDate) -> u32 {
    let first = NaiveDate::from_ymd_opt(date.year(), date.month(), 1).expect("the first exists");
    let offset = first.weekday().num_days_from_sunday();
    (date.day() - 1 + offset) / 7 + 1
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(y: i32, m: u32, d: u32, hh: u32, mm: u32, ss: u32) -> NaiveDateTime {
        NaiveDate::from_ymd_opt(y, m, d)
            .unwrap()
            .and_hms_opt(hh, mm, ss)
            .unwrap()
    }

    #[test]
    fn the_numeric_fields_are_zero_padded_to_their_width() {
        let when = at(2024, 3, 7, 9, 5, 4);
        assert_eq!(
            format("yyyy-MM-dd HH:mm:ss", &when).unwrap(),
            "2024-03-07 09:05:04"
        );
        assert_eq!(format("yy/M/d", &when).unwrap(), "24/3/7");
        assert_eq!(format("h a", &when).unwrap(), "9 AM");
    }

    #[test]
    fn text_in_the_pattern_may_be_quoted() {
        let when = at(2024, 3, 7, 13, 0, 0);
        assert_eq!(format("yyyy 'at' HH", &when).unwrap(), "2024 at 13");
        assert_eq!(format("''yyyy''", &when).unwrap(), "'2024'");
    }

    #[test]
    fn an_unknown_pattern_letter_is_refused() {
        let err = format("yyyy-bb", &at(2024, 1, 1, 0, 0, 0)).unwrap_err();
        assert!(err.to_lowercase().contains("pattern"), "{err}");
        assert!(err.contains('b'), "{err}");
    }

    #[test]
    fn the_week_year_rolls_over_with_the_week_and_not_with_the_year() {
        // 2024-12-30 is a Monday; its week runs into 2025
        assert_eq!(format("YYYY", &at(2024, 12, 30, 0, 0, 0)).unwrap(), "2025");
        assert_eq!(format("yyyy", &at(2024, 12, 30, 0, 0, 0)).unwrap(), "2024");
        // 2024-12-24 is a Tuesday, its week ends on the 28th
        assert_eq!(format("YYYY", &at(2024, 12, 24, 0, 0, 0)).unwrap(), "2024");
    }
}
