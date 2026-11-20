//! Pure built-in functions: dates, math helpers and locale-aware number formatting.
//!
//! Dates are Int milliseconds of *local wall-clock time* (`now()` already includes the
//! device's time zone offset), so every calendar operation here is plain UTC arithmetic and
//! the same on every platform and in headless tests.

pub const DAY_MS: i64 = 86_400_000;

/// Days since 1970-01-01 -> (year, month 1..12, day 1..31). Howard Hinnant's algorithm.
pub fn civil_from_days(z: i64) -> (i64, i64, i64) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (if m <= 2 { y + 1 } else { y }, m, d)
}

/// (year, month 1..12, day) -> days since 1970-01-01. Months outside 1..12 roll the year.
pub fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let (y, m) = (y + (m - 1).div_euclid(12), (m - 1).rem_euclid(12) + 1);
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y.rem_euclid(400);
    let mp = if m > 2 { m - 3 } else { m + 9 };
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

pub fn days_in_month(y: i64, m: i64) -> i64 {
    days_from_civil(y, m + 1, 1) - days_from_civil(y, m, 1)
}

/// The parts of a timestamp: year, month, day, hour, minute, second, weekday (1 = Monday … 7 = Sunday).
pub struct Parts {
    pub year: i64,
    pub month: i64,
    pub day: i64,
    pub hour: i64,
    pub minute: i64,
    pub second: i64,
    pub weekday: i64,
}

pub fn parts(ms: i64) -> Parts {
    let days = ms.div_euclid(DAY_MS);
    let rest = ms.rem_euclid(DAY_MS) / 1000;
    let (year, month, day) = civil_from_days(days);
    // 1970-01-01 was a Thursday (4)
    let weekday = (days + 3).rem_euclid(7) + 1;
    Parts { year, month, day, hour: rest / 3600, minute: rest / 60 % 60, second: rest % 60, weekday }
}

pub fn date_of(year: i64, month: i64, day: i64, hour: i64, minute: i64) -> i64 {
    days_from_civil(year, month, 1) * DAY_MS + (day - 1) * DAY_MS + hour * 3_600_000 + minute * 60_000
}

/// Add calendar months; the day is clamped to the target month (Jan 31 + 1 month = Feb 28/29).
pub fn add_months(ms: i64, n: i64) -> i64 {
    let p = parts(ms);
    let total = p.year * 12 + (p.month - 1) + n;
    let (y, m) = (total.div_euclid(12), total.rem_euclid(12) + 1);
    let d = p.day.min(days_in_month(y, m));
    days_from_civil(y, m, d) * DAY_MS + ms.rem_euclid(DAY_MS)
}

pub fn start_of_day(ms: i64) -> i64 {
    ms.div_euclid(DAY_MS) * DAY_MS
}

/// Whole calendar days from `a` to `b` (negative when `b` is earlier).
pub fn days_between(a: i64, b: i64) -> i64 {
    b.div_euclid(DAY_MS) - a.div_euclid(DAY_MS)
}

struct Names {
    months: [&'static str; 12],
    /// month names after a day number ("15 января"); same as `months` where grammar has no case
    months_of: [&'static str; 12],
    short_months: [&'static str; 12],
    weekdays: [&'static str; 7],
    short_weekdays: [&'static str; 7],
}

const EN: Names = Names {
    months: ["January", "February", "March", "April", "May", "June", "July", "August", "September", "October", "November", "December"],
    months_of: ["January", "February", "March", "April", "May", "June", "July", "August", "September", "October", "November", "December"],
    short_months: ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"],
    weekdays: ["Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday", "Sunday"],
    short_weekdays: ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"],
};

const RU: Names = Names {
    months: ["январь", "февраль", "март", "апрель", "май", "июнь", "июль", "август", "сентябрь", "октябрь", "ноябрь", "декабрь"],
    months_of: ["января", "февраля", "марта", "апреля", "мая", "июня", "июля", "августа", "сентября", "октября", "ноября", "декабря"],
    short_months: ["янв", "фев", "мар", "апр", "мая", "июн", "июл", "авг", "сен", "окт", "ноя", "дек"],
    weekdays: ["понедельник", "вторник", "среда", "четверг", "пятница", "суббота", "воскресенье"],
    short_weekdays: ["пн", "вт", "ср", "чт", "пт", "сб", "вс"],
};

fn names(locale: &str) -> &'static Names {
    match locale.split(['-', '_']).next().unwrap_or("") {
        "ru" => &RU,
        _ => &EN,
    }
}

/// Format a timestamp with a pattern: `yyyy yy MMMM MMM MM M dd d EEEE EEE HH H mm ss`;
/// text in single quotes is literal ('at'), any other character is copied.
pub fn format_date(ms: i64, pattern: &str, locale: &str) -> String {
    let p = parts(ms);
    let n = names(locale);
    let has_day = pattern.contains('d');
    let chars: Vec<char> = pattern.chars().collect();
    let mut out = String::new();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c == '\'' {
            i += 1;
            while i < chars.len() && chars[i] != '\'' {
                out.push(chars[i]);
                i += 1;
            }
            i += 1;
            continue;
        }
        if !matches!(c, 'y' | 'M' | 'd' | 'E' | 'H' | 'm' | 's') {
            out.push(c);
            i += 1;
            continue;
        }
        let mut run = 1;
        while i + run < chars.len() && chars[i + run] == c {
            run += 1;
        }
        i += run;
        let mi = (p.month - 1) as usize;
        let wi = (p.weekday - 1) as usize;
        match (c, run) {
            ('y', 2) => out.push_str(&format!("{:02}", p.year.rem_euclid(100))),
            ('y', _) => out.push_str(&p.year.to_string()),
            ('M', 1) => out.push_str(&p.month.to_string()),
            ('M', 2) => out.push_str(&format!("{:02}", p.month)),
            ('M', 3) => out.push_str(n.short_months[mi]),
            ('M', _) => out.push_str(if has_day { n.months_of[mi] } else { n.months[mi] }),
            ('d', 1) => out.push_str(&p.day.to_string()),
            ('d', _) => out.push_str(&format!("{:02}", p.day)),
            ('E', 1..=3) => out.push_str(n.short_weekdays[wi]),
            ('E', _) => out.push_str(n.weekdays[wi]),
            ('H', 1) => out.push_str(&p.hour.to_string()),
            ('H', _) => out.push_str(&format!("{:02}", p.hour)),
            ('m', _) => out.push_str(&format!("{:02}", p.minute)),
            (_, _) => out.push_str(&format!("{:02}", p.second)),
        }
    }
    out
}

/// Thousands and decimal separators of a locale: "1,234.5" (en), "1 234,5" (ru, fr, …),
/// "1.234,5" (de, es, it, …).
fn separators(locale: &str) -> (&'static str, char) {
    match locale.split(['-', '_']).next().unwrap_or("") {
        "ru" | "uk" | "be" | "kk" | "fr" | "pl" | "cs" | "sk" | "sv" | "fi" | "nb" | "no" | "hu" | "bg" | "lt" | "lv" | "et" => ("\u{a0}", ','),
        "de" | "es" | "it" | "pt" | "nl" | "tr" | "id" | "da" | "el" | "ro" | "hr" | "sl" | "sr" | "vi" => (".", ','),
        _ => (",", '.'),
    }
}

/// `x.format(digits)`: fixed decimals with locale grouping (1234567.5 -> "1,234,567.50").
pub fn format_number(x: f64, digits: usize, locale: &str) -> String {
    if !x.is_finite() {
        return if x.is_nan() { "NaN".into() } else if x > 0.0 { "∞".into() } else { "-∞".into() };
    }
    let (group, point) = separators(locale);
    let s = format!("{:.*}", digits, x.abs());
    let (int, frac) = match s.split_once('.') {
        Some((a, b)) => (a.to_string(), Some(b.to_string())),
        None => (s.clone(), None),
    };
    let mut grouped = String::new();
    for (i, ch) in int.chars().enumerate() {
        if i > 0 && (int.len() - i) % 3 == 0 {
            grouped.push_str(group);
        }
        grouped.push(ch);
    }
    let negative = x < 0.0 && s.chars().any(|c| c.is_ascii_digit() && c != '0');
    let mut out = String::new();
    if negative {
        out.push('-');
    }
    out.push_str(&grouped);
    if let Some(f) = frac {
        out.push(point);
        out.push_str(&f);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn civil_round_trip() {
        for d in [-800_000, -1, 0, 1, 19_000, 20_468, 2_932_896] {
            let (y, m, dd) = civil_from_days(d);
            assert_eq!(days_from_civil(y, m, dd), d);
        }
        assert_eq!(civil_from_days(0), (1970, 1, 1));
        assert_eq!(days_from_civil(2026, 13, 1), days_from_civil(2027, 1, 1));
    }

    #[test]
    fn parts_and_months() {
        let t = date_of(2026, 1, 31, 9, 30);
        let p = parts(t);
        assert_eq!((p.year, p.month, p.day, p.hour, p.minute, p.weekday), (2026, 1, 31, 9, 30, 6));
        let f = parts(add_months(t, 1));
        assert_eq!((f.year, f.month, f.day, f.hour), (2026, 2, 28, 9));
        let back = parts(add_months(t, -2));
        assert_eq!((back.year, back.month, back.day), (2025, 11, 30));
        assert_eq!(days_between(t, date_of(2026, 3, 1, 0, 0)), 29);
    }

    #[test]
    fn formats() {
        let t = date_of(2026, 3, 5, 14, 7);
        assert_eq!(format_date(t, "d MMM yyyy, HH:mm", "en"), "5 Mar 2026, 14:07");
        assert_eq!(format_date(t, "EEEE, d MMMM", "ru"), "четверг, 5 марта");
        assert_eq!(format_date(t, "MMMM yyyy", "ru"), "март 2026");
        assert_eq!(format_date(t, "dd.MM.yy 'at' H", "en"), "05.03.26 at 14");
        assert_eq!(format_number(1234567.456, 2, "en"), "1,234,567.46");
        assert_eq!(format_number(-1234.6, 0, "ru"), "-1\u{a0}235");
        assert_eq!(format_number(-0.001, 2, "en"), "0.00");
        assert_eq!(format_number(999.0, 1, "de"), "999,0");
        assert_eq!(format_number(1000.0, 0, "de"), "1.000");
    }
}
