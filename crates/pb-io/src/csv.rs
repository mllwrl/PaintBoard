//! Comma- (or semicolon-, or tab-) separated tables, read into rows of cells.

/// Rows of cells, all the same length (short rows are padded with empty cells).
pub type Rows = Vec<Vec<String>>;

/// Reads a CSV file's text. The separator is guessed from the first line: whichever of
/// comma, semicolon and tab appears most outside quotes. Quoted cells may hold separators,
/// line breaks and doubled quotes, as in RFC 4180. Blank lines are skipped.
pub fn parse(text: &str) -> Rows {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    parse_with(text, guess_separator(text))
}

/// Reads tab-separated text, as spreadsheets put on the clipboard.
pub fn parse_tsv(text: &str) -> Rows {
    parse_with(text, '\t')
}

fn guess_separator(text: &str) -> char {
    let mut counts = [(',', 0usize), (';', 0), ('\t', 0)];
    let mut quoted = false;
    for c in text.chars() {
        match c {
            '"' => quoted = !quoted,
            '\n' if !quoted => break,
            _ if !quoted => {
                if let Some(entry) = counts.iter_mut().find(|(s, _)| *s == c) {
                    entry.1 += 1;
                }
            }
            _ => {}
        }
    }
    // The first of the most frequent: a comma when there are none.
    counts
        .iter()
        .fold(
            (',', 0),
            |best, &(s, n)| if n > best.1 { (s, n) } else { best },
        )
        .0
}

fn parse_with(text: &str, separator: char) -> Rows {
    let mut rows: Rows = Vec::new();
    let mut row = Vec::new();
    let mut cell = String::new();
    let mut quoted = false;
    let mut chars = text.chars().peekable();
    let mut end_row = |row: &mut Vec<String>, cell: &mut String| {
        row.push(std::mem::take(cell));
        let row = std::mem::take(row);
        if !(row.len() == 1 && row[0].is_empty()) {
            rows.push(row);
        }
    };
    while let Some(c) = chars.next() {
        if quoted {
            match c {
                '"' if chars.peek() == Some(&'"') => {
                    chars.next();
                    cell.push('"');
                }
                '"' => quoted = false,
                '\r' if chars.peek() == Some(&'\n') => {}
                _ => cell.push(c),
            }
            continue;
        }
        match c {
            '"' if cell.is_empty() => quoted = true,
            '\r' if chars.peek() == Some(&'\n') => {}
            '\n' | '\r' => end_row(&mut row, &mut cell),
            _ if c == separator => row.push(std::mem::take(&mut cell)),
            _ => cell.push(c),
        }
    }
    if !cell.is_empty() || !row.is_empty() {
        end_row(&mut row, &mut cell);
    }
    let width = rows.iter().map(Vec::len).max().unwrap_or(0);
    for row in &mut rows {
        row.resize(width, String::new());
    }
    rows
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rows(lines: &[&[&str]]) -> Rows {
        lines
            .iter()
            .map(|l| l.iter().map(|c| c.to_string()).collect())
            .collect()
    }

    #[test]
    fn reads_plain_cells() {
        assert_eq!(
            parse("name,age\nAnn,31\r\nBob,4\n"),
            rows(&[&["name", "age"], &["Ann", "31"], &["Bob", "4"]])
        );
    }

    #[test]
    fn reads_quoted_cells() {
        let text = "a,\"b, c\",\"say \"\"hi\"\"\"\n\"two\nlines\",x,\n";
        assert_eq!(
            parse(text),
            rows(&[&["a", "b, c", "say \"hi\""], &["two\nlines", "x", ""]])
        );
    }

    #[test]
    fn guesses_the_separator() {
        assert_eq!(
            parse("a;b;c\n1;2,5;3"),
            rows(&[&["a", "b", "c"], &["1", "2,5", "3"]])
        );
        assert_eq!(parse("a\tb\n1\t2"), rows(&[&["a", "b"], &["1", "2"]]));
        assert_eq!(parse("\u{feff}one\n\ntwo"), rows(&[&["one"], &["two"]]));
    }

    #[test]
    fn pads_short_rows() {
        assert_eq!(
            parse_tsv("a\tb\tc\n1"),
            rows(&[&["a", "b", "c"], &["1", "", ""]])
        );
        assert!(parse("").is_empty());
    }
}
