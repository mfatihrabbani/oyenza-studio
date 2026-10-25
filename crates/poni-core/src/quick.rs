//! Quick add: what's typed in the Tasks page's field, which Alt+Shift+Space
//! opens from any app. Plain words are a task, as always; a few short
//! commands do something else instead:
//!
//! - `/join`: open the call of the meeting under way or coming up (Calendar)
//! - `/focus`, `/focus 50`: start a focus session (a number sets its length)
//! - `/clip invoice`: copy the newest clipboard item with those words
//! - `= 12*4500`: work it out and copy the result
//!
//! Pure parsing and arithmetic; the app looks up meetings and the clipboard.

/// A command typed in the field (anything else is a task).
#[derive(Clone, Debug, PartialEq)]
pub enum Quick {
    Join,
    /// Minutes, when given (1 to 180).
    Focus(Option<u32>),
    /// The words to look for (empty: the newest item).
    Clip(String),
    /// The result, or `None` while the sum doesn't add up yet.
    Calc(Option<f64>),
    /// A `/` the field doesn't know: the line lists the ones it does.
    Unknown,
}

/// What the line beside an unknown command says.
pub const HELP: &str = "/join \u{b7} /focus \u{b7} /clip \u{b7} = sum";

pub fn parse(text: &str) -> Option<Quick> {
    let t = text.trim();
    if let Some(expr) = t.strip_prefix('=') {
        return Some(Quick::Calc(calc(expr)));
    }
    let rest = t.strip_prefix('/')?;
    let (word, arg) = match rest.split_once(char::is_whitespace) {
        Some((w, a)) => (w, a.trim()),
        None => (rest, ""),
    };
    Some(match word.to_ascii_lowercase().as_str() {
        "join" | "j" => Quick::Join,
        "focus" | "f" => match arg {
            "" => Quick::Focus(None),
            a => match a.trim_end_matches(['m', 'M']).trim().parse::<u32>() {
                Ok(n) if (1..=180).contains(&n) => Quick::Focus(Some(n)),
                _ => Quick::Unknown,
            },
        },
        "clip" | "c" => Quick::Clip(arg.to_string()),
        _ => Quick::Unknown,
    })
}

/// The newest of `items` (newest first) holding every word of `query`,
/// ignoring case; the newest of all for an empty query.
pub fn find_clip<'a>(items: impl IntoIterator<Item = &'a str>, query: &str) -> Option<&'a str> {
    let words: Vec<String> = query.split_whitespace().map(str::to_lowercase).collect();
    items.into_iter().find(|text| {
        let lower = text.to_lowercase();
        words.iter().all(|w| lower.contains(w.as_str()))
    })
}

/// "54000", "0.25", "1234.57": up to two decimals, no trailing zeros, no
/// grouping (it is pasted into forms and sheets).
pub fn fmt_number(v: f64) -> String {
    let s = format!("{:.2}", v);
    let s = s.trim_end_matches('0').trim_end_matches('.');
    if s == "-0" {
        "0".into()
    } else {
        s.into()
    }
}

/// `12*4500`, `(3+4) x 2.5`, `100 / 3`, `15%`: + - * / x : and parentheses,
/// `%` as hundredths. Spaces and `_` are ignored. `None` when it doesn't
/// add up (or divides by zero).
pub fn calc(expr: &str) -> Option<f64> {
    let tokens: Vec<char> = expr.chars().filter(|c| !c.is_whitespace() && *c != '_').collect();
    if tokens.is_empty() {
        return None;
    }
    let mut p = Parser { s: &tokens, i: 0 };
    let v = p.sum()?;
    (p.i == tokens.len() && v.is_finite()).then_some(v)
}

struct Parser<'a> {
    s: &'a [char],
    i: usize,
}

impl Parser<'_> {
    fn peek(&self) -> Option<char> {
        self.s.get(self.i).copied()
    }

    fn sum(&mut self) -> Option<f64> {
        let mut v = self.product()?;
        while let Some(c @ ('+' | '-')) = self.peek() {
            self.i += 1;
            let r = self.product()?;
            v = if c == '+' { v + r } else { v - r };
        }
        Some(v)
    }

    fn product(&mut self) -> Option<f64> {
        let mut v = self.unary()?;
        while let Some(c @ ('*' | 'x' | 'X' | '\u{d7}' | '/' | ':' | '\u{f7}')) = self.peek() {
            self.i += 1;
            let r = self.unary()?;
            v = if matches!(c, '/' | ':' | '\u{f7}') {
                if r == 0.0 {
                    return None;
                }
                v / r
            } else {
                v * r
            };
        }
        Some(v)
    }

    fn unary(&mut self) -> Option<f64> {
        if self.peek() == Some('-') {
            self.i += 1;
            return self.unary().map(|v| -v);
        }
        if self.peek() == Some('+') {
            self.i += 1;
        }
        let mut v = self.atom()?;
        while self.peek() == Some('%') {
            self.i += 1;
            v /= 100.0;
        }
        Some(v)
    }

    fn atom(&mut self) -> Option<f64> {
        if self.peek() == Some('(') {
            self.i += 1;
            let v = self.sum()?;
            if self.peek() != Some(')') {
                return None;
            }
            self.i += 1;
            return Some(v);
        }
        let start = self.i;
        while self.peek().is_some_and(|c| c.is_ascii_digit() || c == '.' || c == ',') {
            self.i += 1;
        }
        // A comma is a decimal point too ("2,5"), as people type it here.
        let n: String = self.s[start..self.i].iter().map(|c| if *c == ',' { '.' } else { *c }).collect();
        n.parse().ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn words_are_tasks_and_commands_are_not() {
        assert_eq!(parse("Balas email klien"), None);
        assert_eq!(parse("  "), None);
        assert_eq!(parse("kirim invoice / laporan"), None, "a slash inside is just a word");
        assert_eq!(parse("/join"), Some(Quick::Join));
        assert_eq!(parse(" /J "), Some(Quick::Join));
        assert_eq!(parse("/focus"), Some(Quick::Focus(None)));
        assert_eq!(parse("/focus 50"), Some(Quick::Focus(Some(50))));
        assert_eq!(parse("/f 45m"), Some(Quick::Focus(Some(45))));
        assert_eq!(parse("/focus 999"), Some(Quick::Unknown));
        assert_eq!(parse("/clip  invoice bca "), Some(Quick::Clip("invoice bca".into())));
        assert_eq!(parse("/clip"), Some(Quick::Clip(String::new())));
        assert_eq!(parse("/what"), Some(Quick::Unknown));
        assert_eq!(parse("/"), Some(Quick::Unknown));
        assert_eq!(parse("= 12*4500"), Some(Quick::Calc(Some(54000.0))));
        assert_eq!(parse("="), Some(Quick::Calc(None)));
    }

    #[test]
    fn sums() {
        assert_eq!(calc("12*4500"), Some(54000.0));
        assert_eq!(calc("12 x 4_500"), Some(54000.0));
        assert_eq!(calc("(3+4) * 2.5"), Some(17.5));
        assert_eq!(calc("2,5 + 1"), Some(3.5));
        assert_eq!(calc("1+2*3"), Some(7.0));
        assert_eq!(calc("10 - -2"), Some(12.0));
        assert_eq!(calc("200000 * 11%"), Some(22000.0));
        assert_eq!(calc("100 : 4"), Some(25.0));
        assert_eq!(calc("1/0"), None);
        assert_eq!(calc("2*"), None);
        assert_eq!(calc("(1+2"), None);
        assert_eq!(calc("abc"), None);
        assert_eq!(fmt_number(54000.0), "54000");
        assert_eq!(fmt_number(100.0 / 3.0), "33.33");
        assert_eq!(fmt_number(0.25), "0.25");
        assert_eq!(fmt_number(-0.001), "0");
    }

    #[test]
    fn clips_are_found_newest_first_by_all_their_words() {
        let items = ["Transfer BCA 1234 a.n. Rani", "invoice #88 PT Maju", "Invoice #87 PT Maju"];
        assert_eq!(find_clip(items, "invoice"), Some("invoice #88 PT Maju"));
        assert_eq!(find_clip(items, "INVOICE 87"), Some("Invoice #87 PT Maju"));
        assert_eq!(find_clip(items, "bca rani"), Some("Transfer BCA 1234 a.n. Rani"));
        assert_eq!(find_clip(items, ""), Some("Transfer BCA 1234 a.n. Rani"));
        assert_eq!(find_clip(items, "mandiri"), None);
    }
}
