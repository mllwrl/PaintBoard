//! Arithmetic for handwritten expressions ("12+7=" -> 19), like Math Notes in Apple's
//! Notes.
//!
//! Input comes from handwriting recognition, so the usual handwritten forms are accepted:
//! `×`, `x` and `·` multiply, `÷` and `:` divide, `,` is a decimal comma, and letters that
//! look like digits (`O`, `l`, `I`, `|`) count as `0` and `1`. Whatever follows `=` is
//! ignored, so an already answered line can be recalculated.
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

/// The value of an arithmetic expression: `+ - * / ^`, parentheses, unary minus, `%` and
/// implicit multiplication (`2(3+4)`).
pub fn evaluate(input: &str) -> Result<f64, MathError> {
    let tokens = tokenize(input)?;
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

/// Whether the written line already ends with `=`, so the answer goes right after it.
pub fn ends_with_equals(input: &str) -> bool {
    input.trim_end().ends_with('=')
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
            'O' | 'o' => Some('0'),
            'l' | 'I' | '|' => Some('1'),
            '.' | ',' => Some('.'),
            _ => None,
        };
        if let Some(d) = digit {
            number.push(d);
            continue;
        }
        flush(&mut number, &mut tokens)?;
        let token = match c {
            '+' => Token::Plus,
            '-' | '−' | '–' => Token::Minus,
            '*' | '×' | 'x' | 'X' | '·' => Token::Times,
            '/' | '÷' | ':' => Token::Divide,
            '^' => Token::Power,
            '%' => Token::Percent,
            '(' | '[' => Token::Open,
            ')' | ']' => Token::Close,
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
                // 2(3 + 4) and (1 + 2)(3 + 4).
                Some(Token::Open) => value *= self.power()?,
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

    /// unary = ("-" | "+") unary | atom "%"*
    fn unary(&mut self) -> Result<f64, MathError> {
        match self.peek() {
            Some(Token::Minus) => {
                self.pos += 1;
                Ok(-self.unary()?)
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
    fn errors() {
        assert_eq!(evaluate(""), Err(MathError::Empty));
        assert_eq!(evaluate("="), Err(MathError::Empty));
        assert_eq!(evaluate("1 / 0"), Err(MathError::DivisionByZero));
        assert_eq!(evaluate("(1 + 2"), Err(MathError::UnbalancedParentheses));
        assert_eq!(evaluate("1 + 2)"), Err(MathError::UnbalancedParentheses));
        assert_eq!(evaluate("hello"), Err(MathError::Unexpected('h')));
        assert_eq!(evaluate("1 +"), Err(MathError::Empty));
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
