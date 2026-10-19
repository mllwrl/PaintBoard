//! Arithmetic for handwritten expressions ("12+7=" -> 19), like Math Notes in Apple's
//! Notes.
//!
//! Input comes from handwriting recognition, so the usual handwritten forms are accepted:
//! `×`, `x` and `·` multiply, `÷` and `:` divide, `,` is a decimal comma, `√` takes a
//! square root, `²` and `³` raise to a power and `π` is pi. Letters that recognizers
//! (Latin or Cyrillic) give for digits count as those digits: `O` and `О` as `0`, `l` and
//! `I` as `1`, `З` as `3`, `б` as `6` and so on; Cyrillic `х` multiplies like `x`. Spaces
//! inside a number are ignored (`1 000`). Whatever follows `=` is ignored, so an already
//! answered line can be recalculated, and so is a trailing operator, which is often a
//! misread `=`.
//!
//! TODO: an automatic mode that solves a line as soon as `=` is written; variables
//! (`a = 5`), functions and units.

use std::fmt;

#[derive(Clone, Debug, PartialEq)]
pub enum MathError {
    Empty,
    /// A character that is not part of arithmetic, with its position.
    Unexpected(char),
    UnbalancedParentheses,
    DivisionByZero,
    /// The result is not a finite number (overflow, root of a negative number).
    NotANumber,
}

impl fmt::Display for MathError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty => write!(f, "there is no expression"),
            Self::Unexpected(c) => write!(f, "\"{c}\" is not part of an expression"),
            Self::UnbalancedParentheses => write!(f, "the parentheses do not match"),
            Self::DivisionByZero => write!(f, "division by zero"),
            Self::NotANumber => write!(f, "the result is not a number"),
        }
    }
}

impl std::error::Error for MathError {}

/// The value of an arithmetic expression: `+ - * / ^`, parentheses, unary minus, `%`,
/// `√` and implicit multiplication (`2(3+4)`).
pub fn evaluate(input: &str) -> Result<f64, MathError> {
    let mut tokens = tokenize(input)?;
    // "12 + 7 -": the "-" is most likely an "=" read wrong.
    while matches!(
        tokens.last(),
        Some(Token::Plus | Token::Minus | Token::Times | Token::Divide | Token::Power)
    ) && tokens.len() > 1
    {
        tokens.pop();
    }
    if tokens.is_empty() {
        return Err(MathError::Empty);
    }
    let mut parser = Parser { tokens, pos: 0 };
    let value = parser.sum()?;
    match parser.peek() {
        None => {}
        Some(Token::Close) => return Err(MathError::UnbalancedParentheses),
        Some(token) => return Err(MathError::Unexpected(token.symbol())),
    }
    if value.is_finite() {
        Ok(value)
    } else {
        Err(MathError::NotANumber)
    }
}

/// Whether the written line already ends with `=` (or what is likely one read wrong), so
/// the answer goes right after it.
pub fn ends_with_equals(input: &str) -> bool {
    let line = input.trim_end();
    line.ends_with('=') || line.ends_with(['-', '−', '–', '—', '_'])
}

/// The reading of handwriting to calculate. Recognizers give each written word with
/// alternatives, best first, and the best one is not always the arithmetic one: for
/// `1+1=` Windows' first guess is `/+1=`, its second `1+1=`. So the readings are tried
/// from the best guess on, by how far down the lists they go in all, and the first that
/// can be calculated wins; if none can, the best guess.
pub fn best_reading(words: &[Vec<String>]) -> String {
    /// Readings tried at most.
    const TRIES: usize = 2000;
    let words: Vec<&Vec<String>> = words.iter().filter(|w| !w.is_empty()).collect();
    let read = |ranks: &[usize]| -> String {
        words
            .iter()
            .zip(ranks)
            .map(|(w, &r)| w[r].as_str())
            .collect::<Vec<_>>()
            .join(" ")
    };
    let deepest: usize = words.iter().map(|w| w.len() - 1).sum();
    let mut tries = 0;
    for depth in 0..=deepest {
        let mut found = None;
        for_ranks(&words, depth, &mut Vec::new(), &mut |ranks| {
            tries += 1;
            let reading = read(ranks);
            if evaluate(&reading).is_ok() {
                found = Some(reading);
            }
            found.is_none() && tries < TRIES
        });
        if let Some(reading) = found {
            return reading;
        }
        if tries >= TRIES {
            break;
        }
    }
    read(&vec![0; words.len()])
}

/// Calls `visit` with each way to pick an alternative of each word whose ranks add up to
/// `depth`, while it returns true.
fn for_ranks(
    words: &[&Vec<String>],
    depth: usize,
    ranks: &mut Vec<usize>,
    visit: &mut impl FnMut(&[usize]) -> bool,
) -> bool {
    let Some((word, rest)) = words.split_first() else {
        return depth > 0 || visit(ranks);
    };
    for rank in 0..word.len().min(depth + 1) {
        ranks.push(rank);
        let go_on = for_ranks(rest, depth - rank, ranks, visit);
        ranks.pop();
        if !go_on {
            return false;
        }
    }
    true
}

/// A result for display: up to 10 significant digits, without trailing zeros, so that
/// `0.1 + 0.2` shows as `0.3`.
pub fn format_number(value: f64) -> String {
    if value == 0.0 {
        return "0".into();
    }
    let digits = 10 - (value.abs().log10().floor() as i32 + 1);
    let mut text = if value.abs() >= 1e15 || value.abs() < 1e-6 {
        format!("{value:.6e}")
    } else {
        format!("{:.*}", digits.clamp(0, 15) as usize, value)
    };
    if text.contains('.') && !text.contains('e') {
        text = text.trim_end_matches('0').trim_end_matches('.').to_owned();
    }
    if text == "-0" { "0".into() } else { text }
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Token {
    Number(f64),
    Plus,
    Minus,
    Times,
    Divide,
    Power,
    Percent,
    Open,
    Close,
    Root,
}

impl Token {
    fn symbol(self) -> char {
        match self {
            Self::Number(_) => '0',
            Self::Plus => '+',
            Self::Minus => '-',
            Self::Times => '*',
            Self::Divide => '/',
            Self::Power => '^',
            Self::Percent => '%',
            Self::Open => '(',
            Self::Close => ')',
            Self::Root => '√',
        }
    }
}

fn tokenize(input: &str) -> Result<Vec<Token>, MathError> {
    let line = input.split('=').next().unwrap_or_default();
    let mut tokens = Vec::new();
    let mut number = String::new();
    let flush = |number: &mut String, tokens: &mut Vec<Token>| -> Result<(), MathError> {
        if !number.is_empty() {
            let value = number.parse().map_err(|_| MathError::Unexpected('.'))?;
            tokens.push(Token::Number(value));
            number.clear();
        }
        Ok(())
    };
    for c in line.chars() {
        let digit = match c {
            '0'..='9' => Some(c),
            // Latin and Cyrillic letters recognizers give for digits.
            'O' | 'o' | 'О' | 'о' | 'D' => Some('0'),
            'l' | 'I' | '|' | 'i' | '!' => Some('1'),
            'Z' | 'z' => Some('2'),
            'З' | 'з' | 'Э' => Some('3'),
            'Ч' | 'ч' => Some('4'),
            'S' | 's' => Some('5'),
            'б' | 'b' | 'G' => Some('6'),
            'T' | 'Т' => Some('7'),
            'B' | 'В' => Some('8'),
            'g' | 'q' | 'Я' => Some('9'),
            '.' | ',' => Some('.'),
            _ => None,
        };
        if let Some(d) = digit {
            number.push(d);
            continue;
        }
        // A space inside a number ("1 000") is skipped.
        if c.is_whitespace() && !number.is_empty() {
            continue;
        }
        flush(&mut number, &mut tokens)?;
        let token = match c {
            '+' => Token::Plus,
            '-' | '−' | '–' | '—' | '_' => Token::Minus,
            '*' | '×' | 'x' | 'X' | '·' | '∙' | '•' | '⋅' | 'х' | 'Х' => Token::Times,
            '/' | '÷' | ':' => Token::Divide,
            '^' => Token::Power,
            '%' => Token::Percent,
            '(' | '[' | '{' => Token::Open,
            ')' | ']' | '}' => Token::Close,
            '√' | 'V' | 'v' => Token::Root,
            'π' | 'п' | 'П' => Token::Number(std::f64::consts::PI),
            '²' | '³' => {
                tokens.push(Token::Power);
                Token::Number(if c == '²' { 2.0 } else { 3.0 })
            }
            c if c.is_whitespace() => continue,
            c => return Err(MathError::Unexpected(c)),
        };
        tokens.push(token);
    }
    flush(&mut number, &mut tokens)?;
    Ok(tokens)
}

struct Parser {
    tokens: Vec<Token>,
    pos: usize,
}

impl Parser {
    fn peek(&self) -> Option<Token> {
        self.tokens.get(self.pos).copied()
    }

    /// The next token is pi, written right after something (2π).
    fn after_constant(&self) -> bool {
        matches!(self.peek(), Some(Token::Number(v)) if v == std::f64::consts::PI)
    }

    fn next(&mut self) -> Option<Token> {
        let token = self.peek();
        self.pos += 1;
        token
    }

    /// sum = product (("+" | "-") product)*
    fn sum(&mut self) -> Result<f64, MathError> {
        let mut value = self.product()?;
        while let Some(op @ (Token::Plus | Token::Minus)) = self.peek() {
            self.pos += 1;
            let rhs = self.product()?;
            value = if op == Token::Plus {
                value + rhs
            } else {
                value - rhs
            };
        }
        Ok(value)
    }

    /// product = power (("*" | "/" | implicit) power)*
    fn product(&mut self) -> Result<f64, MathError> {
        let mut value = self.power()?;
        loop {
            match self.peek() {
                Some(Token::Times) => {
                    self.pos += 1;
                    value *= self.power()?;
                }
                Some(Token::Divide) => {
                    self.pos += 1;
                    let rhs = self.power()?;
                    if rhs == 0.0 {
                        return Err(MathError::DivisionByZero);
                    }
                    value /= rhs;
                }
                // 2(3 + 4), (1 + 2)(3 + 4), 2√9 and 2π.
                Some(Token::Open | Token::Root) => value *= self.power()?,
                Some(Token::Number(_)) if self.after_constant() => value *= self.power()?,
                _ => return Ok(value),
            }
        }
    }

    /// power = unary ("^" power)?   (right-associative)
    fn power(&mut self) -> Result<f64, MathError> {
        let base = self.unary()?;
        if self.peek() == Some(Token::Power) {
            self.pos += 1;
            return Ok(base.powf(self.power()?));
        }
        Ok(base)
    }

    /// unary = ("-" | "+" | "√") unary | atom "%"*
    fn unary(&mut self) -> Result<f64, MathError> {
        match self.peek() {
            Some(Token::Minus) => {
                self.pos += 1;
                Ok(-self.unary()?)
            }
            Some(Token::Root) => {
                self.pos += 1;
                Ok(self.unary()?.sqrt())
            }
            Some(Token::Plus) => {
                self.pos += 1;
                self.unary()
            }
            _ => {
                let mut value = self.atom()?;
                while self.peek() == Some(Token::Percent) {
                    self.pos += 1;
                    value /= 100.0;
                }
                Ok(value)
            }
        }
    }

    /// atom = number | "(" sum ")"
    fn atom(&mut self) -> Result<f64, MathError> {
        match self.next() {
            Some(Token::Number(v)) => Ok(v),
            Some(Token::Open) => {
                let value = self.sum()?;
                match self.next() {
                    Some(Token::Close) => Ok(value),
                    _ => Err(MathError::UnbalancedParentheses),
                }
            }
            Some(Token::Close) => Err(MathError::UnbalancedParentheses),
            Some(token) => Err(MathError::Unexpected(token.symbol())),
            None => Err(MathError::Empty),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn eval(s: &str) -> f64 {
        evaluate(s).unwrap_or_else(|e| panic!("{s}: {e}"))
    }

    #[test]
    fn basic_arithmetic_with_precedence() {
        assert_eq!(eval("12+7="), 19.0);
        assert_eq!(eval("2 + 3 * 4"), 14.0);
        assert_eq!(eval("(2 + 3) * 4"), 20.0);
        assert_eq!(eval("2^3^2"), 512.0);
        assert_eq!(eval("-3 + 5"), 2.0);
        assert_eq!(eval("2(3+4)"), 14.0);
        assert_eq!(eval("50%"), 0.5);
    }

    #[test]
    fn handwritten_forms() {
        assert_eq!(eval("6 × 7"), 42.0);
        assert_eq!(eval("6x7"), 42.0);
        assert_eq!(eval("8 ÷ 2"), 4.0);
        assert_eq!(eval("9 : 3"), 3.0);
        assert_eq!(eval("1,5 + 1,5"), 3.0);
        assert_eq!(eval("1O − l"), 9.0);
        // Already answered: the old answer is ignored.
        assert_eq!(eval("2 + 2 = 5"), 4.0);
    }

    #[test]
    fn what_recognizers_give_for_digits_and_signs() {
        // Cyrillic look-alikes from a Russian recognizer.
        assert_eq!(eval("З х 4"), 12.0);
        assert_eq!(eval("1О + б"), 16.0);
        assert_eq!(eval("2 • 3"), 6.0);
        assert_eq!(eval("1 000 + 1"), 1001.0);
        // "=" read as a minus or underscore.
        assert_eq!(eval("12 + 7 -"), 19.0);
        assert!(ends_with_equals("12 + 7 -"));
        assert!(ends_with_equals("12 + 7 _"));
        assert_eq!(eval("√9 + 3²"), 12.0);
        assert_eq!(eval("2√16"), 8.0);
        assert!((eval("2π") - std::f64::consts::TAU).abs() < 1e-12);
        assert_eq!(eval("{2+3}*2"), 10.0);
    }

    #[test]
    fn the_arithmetic_reading_is_picked() {
        let words = |w: &[&[&str]]| -> Vec<Vec<String>> {
            w.iter()
                .map(|a| a.iter().map(|s| s.to_string()).collect())
                .collect()
        };
        // The best guess for the second word is a word; an alternative is "+".
        let reading = best_reading(&words(&[&["12"], &["Плюс", "+", "t"], &["7="]]));
        assert_eq!(reading, "12 + 7=");
        assert_eq!(eval(&reading), 19.0);
        // What Windows gave for a handwritten 1+1=.
        let reading = best_reading(&words(&[&["/+1=", "1+1=", "/+/=", "1+/=", "1+1-"]]));
        assert_eq!(reading, "1+1=");
        assert_eq!(best_reading(&words(&[&["hello", "world"]])), "hello");
        assert_eq!(best_reading(&[]), "");
    }

    #[test]
    fn errors() {
        assert_eq!(evaluate(""), Err(MathError::Empty));
        assert_eq!(evaluate("="), Err(MathError::Empty));
        assert_eq!(evaluate("1 / 0"), Err(MathError::DivisionByZero));
        assert_eq!(evaluate("(1 + 2"), Err(MathError::UnbalancedParentheses));
        assert_eq!(evaluate("1 + 2)"), Err(MathError::UnbalancedParentheses));
        assert_eq!(evaluate("hello"), Err(MathError::Unexpected('h')));
        assert_eq!(evaluate("+"), Err(MathError::Empty));
        assert_eq!(evaluate("(-1)^0.5"), Err(MathError::NotANumber));
    }

    #[test]
    fn numbers_format_cleanly() {
        assert_eq!(format_number(19.0), "19");
        assert_eq!(format_number(0.1 + 0.2), "0.3");
        assert_eq!(format_number(1.0 / 3.0), "0.3333333333");
        assert_eq!(format_number(-2.5), "-2.5");
        assert_eq!(format_number(123456789.0), "123456789");
        assert_eq!(format_number(1e20), "1.000000e20");
        assert_eq!(format_number(-0.0), "0");
    }

    #[test]
    fn answer_placement() {
        assert!(ends_with_equals("12 + 7 ="));
        assert!(!ends_with_equals("12 + 7"));
    }
}
