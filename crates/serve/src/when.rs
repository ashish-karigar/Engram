//! Time words in a question ("10 days ago", "last week", "in February") as a range of dates, so
//! search can favour what happened then. Relative to `today`: now for agents, the question's date
//! in evaluation.

use chrono::{Datelike, Duration, NaiveDate};

/// Inclusive range of days a question points at.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Window {
    pub from: NaiveDate,
    pub to: NaiveDate,
}

impl Window {
    fn around(center: NaiveDate, slack: i64) -> Window {
        Window { from: center - Duration::days(slack), to: center + Duration::days(slack) }
    }

    pub fn contains(&self, day: NaiveDate) -> bool {
        self.from <= day && day <= self.to
    }
}

const MONTHS: [&str; 12] = ["january", "february", "march", "april", "may", "june", "july", "august", "september", "october", "november", "december"];

fn number(word: &str) -> Option<i64> {
    const WORDS: [&str; 13] = ["zero", "one", "two", "three", "four", "five", "six", "seven", "eight", "nine", "ten", "eleven", "twelve"];
    match word {
        "a" | "an" => Some(1),
        "couple" => Some(2),
        "few" => Some(3),
        _ => word.parse().ok().or_else(|| WORDS.iter().position(|w| *w == word).map(|n| n as i64)),
    }
}

/// Days in one unit, and how far either side of the exact day still counts (people round).
fn unit(word: &str) -> Option<(i64, i64)> {
    match word.trim_end_matches('s') {
        "day" => Some((1, 1)),
        "week" => Some((7, 3)),
        "month" => Some((30, 10)),
        "year" => Some((365, 45)),
        _ => None,
    }
}

fn month_window(month: u32, year: i32) -> Option<Window> {
    let from = NaiveDate::from_ymd_opt(year, month, 1)?;
    let next = if month == 12 { NaiveDate::from_ymd_opt(year + 1, 1, 1)? } else { NaiveDate::from_ymd_opt(year, month + 1, 1)? };
    Some(Window { from, to: next - Duration::days(1) })
}

/// The first time expression found, if any.
pub fn window(query: &str, today: NaiveDate) -> Option<Window> {
    let words: Vec<String> = query.to_lowercase().split(|c: char| !c.is_alphanumeric()).filter(|w| !w.is_empty()).map(String::from).collect();
    let at = |i: usize| words.get(i).map(String::as_str).unwrap_or("");
    for i in 0..words.len() {
        let w = at(i);
        // "10 days ago", "a week ago", "a couple of months ago"
        if w == "ago" && i >= 2 {
            let (n, u) = if at(i - 2) == "of" && i >= 3 { (at(i - 3), at(i - 1)) } else { (at(i - 2), at(i - 1)) };
            if let (Some(n), Some((days, slack))) = (number(n), unit(u)) {
                return Some(Window::around(today - Duration::days(n * days), slack.max(n * slack / 4)));
            }
        }
        // "the past month", "the last 3 weeks", "past two weeks"
        if (w == "past" || w == "last") && !matches!(at(i + 1), "") {
            let (n, u) = match number(at(i + 1)) {
                Some(n) if unit(at(i + 2)).is_some() => (n, at(i + 2)),
                _ => (1, at(i + 1)),
            };
            if let Some((days, _)) = unit(u) {
                // Up to today either way: "last week" is said both of the week before and of the last seven days.
                return Some(Window { from: today - Duration::days(n * days + days / 4), to: today });
            }
        }
        if w == "yesterday" {
            return Some(Window::around(today - Duration::days(1), 1));
        }
        if w == "today" || w == "tonight" {
            return Some(Window::around(today, 0));
        }
        if w == "this" && matches!(at(i + 1), "week" | "month" | "year") {
            let (days, _) = unit(at(i + 1))?;
            return Some(Window { from: today - Duration::days(days), to: today });
        }
        // "in February": the most recent February up to today.
        if let Some(m) = MONTHS.iter().position(|name| *name == w) {
            let month = m as u32 + 1;
            // "may" is also a verb; only count it after "in", "during", "of" or "early/late".
            if w == "may" && !matches!(at(i.wrapping_sub(1)), "in" | "during" | "of" | "early" | "late" | "mid") {
                continue;
            }
            let year = if month <= today.month() { today.year() } else { today.year() - 1 };
            return month_window(month, year);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(s: &str) -> NaiveDate {
        NaiveDate::parse_from_str(s, "%Y-%m-%d").unwrap()
    }

    #[test]
    fn relative_expressions() {
        let today = d("2023-04-20");
        assert!(window("How many days ago did I meet Emma?", today).is_none());
        let w = window("What kitchen appliance did I buy 10 days ago?", today).unwrap();
        assert!(w.contains(d("2023-04-10")) && !w.contains(d("2023-04-01")));
        assert!(window("the life event I went to a week ago", today).unwrap().contains(d("2023-04-13")));
        assert!(window("sports events during the past month", today).unwrap().contains(d("2023-03-25")));
        assert!(window("what did I cook yesterday", today).unwrap().contains(d("2023-04-19")));
        assert_eq!(window("which vehicle did I fix first in February", today), month_window(2, 2023));
        assert_eq!(window("trips in November", today), month_window(11, 2022));
        assert!(window("I may go hiking", today).is_none());
    }
}
