use itertools::Itertools;
use std::borrow::Cow;

/// Abbreviator for long strings with path structure (contains delimiters).
pub struct Abbreviator<'a> {
    delimiter: &'a str,
    max_len: usize,
    stub: &'a str,
}

impl<'a> Abbreviator<'a> {
    /// Create new abbreviator.
    ///
    /// # Arguments
    ///
    /// * `delimiter`: delimiter between atomic string parts
    /// * `stub`: string appended to start of result string if it was abbreviated
    /// * `max_len`: maximum length of abbreviated string
    pub fn new(delimiter: &'a str, stub: &'a str, max_len: usize) -> Self {
        Self {
            delimiter,
            max_len,
            stub,
        }
    }

    /// Get abbreviation from string `s` (if `s` need to be abbreviated).
    pub fn apply<'b>(&self, s: &'b str) -> Cow<'b, str> {
        if s.len() <= self.max_len {
            return Cow::Borrowed(s);
        }

        let parts = s.split(self.delimiter).collect_vec();

        let mut reminder: isize = self.max_len as isize;
        let needle_parts_cnt = parts
            .iter()
            .rev()
            .take_while(|s| {
                reminder -= (s.len() + self.delimiter.len()) as isize;
                reminder >= 0
            })
            .count();

        let start_from = parts.len() - needle_parts_cnt;
        let result = self.stub.to_string()
            + self.delimiter
            + parts
                .into_iter()
                .skip(start_from)
                .join(self.delimiter)
                .as_str();

        Cow::Owned(result)
    }
}

#[cfg(test)]
mod test {
    use crate::ui::short::Abbreviator;

    #[test]
    fn test_abbreviator() {
        struct TestCase {
            input: &'static str,
            expected: &'static str,
        }

        let cases = [
            TestCase {
                input: "aa::bb",
                expected: "aa::bb",
            },
            TestCase {
                input: "aa::bbb",
                expected: "::bbb",
            },
            TestCase {
                input: "a::b::c",
                expected: "::b::c",
            },
        ];

        for tc in cases {
            let ab = Abbreviator::new("::", "", 6);
            assert_eq!(ab.apply(tc.input), tc.expected);
        }
    }
}
