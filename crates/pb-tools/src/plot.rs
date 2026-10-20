//! Graphs of functions `y = f(x)`: formulas like `sin(x)`, `x^2 - 3x + 1` or `1/x`,
//! sampled into curves that fit a box, with axis ticks.
//!
//! Formulas take numbers (`2.5` or `2,5`), `x`, `+ - * / ^` (also `× · ÷ −`, `²`, `³`),
//! parentheses, `|x|`, implicit multiplication (`2x`, `3sin(x)`, `x(x+1)`), the constants
//! `pi` (`π`) and `e`, and the functions `sin cos tan (tg) cot (ctg) asin acos atan sinh
//! cosh tanh sqrt (√) abs exp ln log (lg, base 10)`. A function's argument may go
//! without parentheses: `sin x`.

use std::fmt;

/// Samples taken across the x range.
const SAMPLES: usize = 800;

#[derive(Clone, Debug, PartialEq)]
pub enum PlotError {
    Empty,
    /// A character or name that is not part of a formula.
    Unexpected(String),
    UnbalancedParentheses,
    /// The x range is empty or not a number.
    BadRange,
    /// The function has no value anywhere in the range.
    NoValues,
}

impl fmt::Display for PlotError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty => write!(f, "there is no formula"),
            Self::Unexpected(s) => write!(f, "\"{s}\" is not part of a formula"),
            Self::UnbalancedParentheses => write!(f, "the parentheses do not match"),
            Self::BadRange => write!(f, "the range of x is empty"),
            Self::NoValues => write!(f, "the function has no values in this range"),
        }
    }
}

impl std::error::Error for PlotError {}

/// A parsed formula of `x`.
#[derive(Clone, Debug, PartialEq)]
pub enum Expr {
    Number(f64),
    X,
    Neg(Box<Expr>),
    Binary(Op, Box<Expr>, Box<Expr>),
    Call(Func, Box<Expr>),
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Op {
    Add,
    Sub,
    Mul,
    Div,
    Pow,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Func {
    Sin,
    Cos,
    Tan,
    Cot,
    Asin,
    Acos,
    Atan,
    Sinh,
    Cosh,
    Tanh,
    Sqrt,
    Abs,
    Exp,
    Ln,
    Log,
}

/// Names known in formulas, longest first so `sinh` is not read as `sin h`.
const NAMES: [(&str, Name); 25] = [
    ("asin", Name::Func(Func::Asin)),
    ("acos", Name::Func(Func::Acos)),
    ("atan", Name::Func(Func::Atan)),
    ("sinh", Name::Func(Func::Sinh)),
    ("cosh", Name::Func(Func::Cosh)),
    ("tanh", Name::Func(Func::Tanh)),
    ("sqrt", Name::Func(Func::Sqrt)),
    ("sin", Name::Func(Func::Sin)),
    ("cos", Name::Func(Func::Cos)),
    ("tan", Name::Func(Func::Tan)),
    ("cot", Name::Func(Func::Cot)),
    ("ctg", Name::Func(Func::Cot)),
    ("abs", Name::Func(Func::Abs)),
    ("exp", Name::Func(Func::Exp)),
    ("log", Name::Func(Func::Log)),
    ("tg", Name::Func(Func::Tan)),
    ("ln", Name::Func(Func::Ln)),
    ("lg", Name::Func(Func::Log)),
    ("pi", Name::Const(std::f64::consts::PI)),
    ("π", Name::Const(std::f64::consts::PI)),
    ("√", Name::Func(Func::Sqrt)),
    ("e", Name::Const(std::f64::consts::E)),
    ("x", Name::X),
    ("х", Name::X),
    ("X", Name::X),
];

#[derive(Clone, Copy, Debug, PartialEq)]
enum Name {
    Func(Func),
    Const(f64),
    X,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Token {
    Number(f64),
    Name(Name),
    Op(Op),
    Open,
    Close,
    Bar,
}

impl Expr {
    /// The value at `x`; NaN or infinite where the function has none.
    pub fn eval(&self, x: f64) -> f64 {
        match self {
            Self::Number(v) => *v,
            Self::X => x,
            Self::Neg(e) => -e.eval(x),
            Self::Binary(op, a, b) => {
                let (a, b) = (a.eval(x), b.eval(x));
                match op {
                    Op::Add => a + b,
                    Op::Sub => a - b,
                    Op::Mul => a * b,
                    Op::Div => a / b,
                    Op::Pow => a.powf(b),
                }
            }
            Self::Call(f, e) => {
                let v = e.eval(x);
                match f {
                    Func::Sin => v.sin(),
                    Func::Cos => v.cos(),
                    Func::Tan => v.tan(),
                    Func::Cot => 1.0 / v.tan(),
                    Func::Asin => v.asin(),
                    Func::Acos => v.acos(),
                    Func::Atan => v.atan(),
                    Func::Sinh => v.sinh(),
                    Func::Cosh => v.cosh(),
                    Func::Tanh => v.tanh(),
                    Func::Sqrt => v.sqrt(),
                    Func::Abs => v.abs(),
                    Func::Exp => v.exp(),
                    Func::Ln => v.ln(),
                    Func::Log => v.log10(),
                }
            }
        }
    }
}

/// Reads a formula of `x`. A leading `y =` or `f(x) =` is skipped.
pub fn parse(src: &str) -> Result<Expr, PlotError> {
    let body = match src.split_once('=') {
        Some((_, right)) => right,
        None => src,
    };
    let tokens = tokenize(body)?;
    if tokens.is_empty() {
        return Err(PlotError::Empty);
    }
    let mut parser = Parser { tokens, pos: 0 };
    let expr = parser.sum()?;
    match parser.peek() {
        None => Ok(expr),
        Some(Token::Close) => Err(PlotError::UnbalancedParentheses),
        Some(token) => Err(PlotError::Unexpected(symbol(token))),
    }
}

fn symbol(token: Token) -> String {
    match token {
        Token::Number(v) => v.to_string(),
        Token::Name(_) => "name".into(),
        Token::Op(op) => match op {
            Op::Add => "+",
            Op::Sub => "-",
            Op::Mul => "*",
            Op::Div => "/",
            Op::Pow => "^",
        }
        .into(),
        Token::Open => "(".into(),
        Token::Close => ")".into(),
        Token::Bar => "|".into(),
    }
}

fn tokenize(src: &str) -> Result<Vec<Token>, PlotError> {
    let chars: Vec<char> = src.chars().collect();
    let mut tokens = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c.is_ascii_digit()
            || ((c == '.' || c == ',') && chars.get(i + 1).is_some_and(char::is_ascii_digit))
        {
            let start = i;
            while i < chars.len()
                && (chars[i].is_ascii_digit() || chars[i] == '.' || chars[i] == ',')
            {
                i += 1;
            }
            let text: String = chars[start..i]
                .iter()
                .map(|&c| if c == ',' { '.' } else { c })
                .collect();
            let value = text
                .parse()
                .map_err(|_| PlotError::Unexpected(text.clone()))?;
            tokens.push(Token::Number(value));
            continue;
        }
        let token = match c {
            c if c.is_whitespace() => None,
            '+' => Some(Token::Op(Op::Add)),
            '-' | '−' | '–' => Some(Token::Op(Op::Sub)),
            '*' | '×' | '·' | '∙' => Some(Token::Op(Op::Mul)),
            '/' | '÷' | ':' => Some(Token::Op(Op::Div)),
            '^' => Some(Token::Op(Op::Pow)),
            '(' | '[' => Some(Token::Open),
            ')' | ']' => Some(Token::Close),
            '|' => Some(Token::Bar),
            '²' | '³' => {
                tokens.push(Token::Op(Op::Pow));
                Some(Token::Number(if c == '²' { 2.0 } else { 3.0 }))
            }
            _ => {
                let rest: String = chars[i..].iter().collect();
                let lower = rest.to_lowercase();
                let Some((name, value)) = NAMES.iter().find(|(n, _)| {
                    // `X` is the only name that counts in capitals; others match lowercased.
                    if *n == "X" {
                        rest.starts_with('X')
                    } else {
                        lower.starts_with(n)
                    }
                }) else {
                    let word: String = chars[i..]
                        .iter()
                        .take_while(|c| c.is_alphanumeric())
                        .collect();
                    let word = if word.is_empty() { c.to_string() } else { word };
                    return Err(PlotError::Unexpected(word));
                };
                tokens.push(Token::Name(*value));
                i += name.chars().count();
                continue;
            }
        };
        tokens.extend(token);
        i += 1;
    }
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

    /// sum = product (("+" | "-") product)*
    fn sum(&mut self) -> Result<Expr, PlotError> {
        let mut expr = self.product()?;
        while let Some(Token::Op(op @ (Op::Add | Op::Sub))) = self.peek() {
            self.pos += 1;
            expr = Expr::Binary(op, Box::new(expr), Box::new(self.product()?));
        }
        Ok(expr)
    }

    /// product = unary (("*" | "/") unary | implicit power)*
    fn product(&mut self) -> Result<Expr, PlotError> {
        let mut expr = self.unary()?;
        loop {
            match self.peek() {
                Some(Token::Op(op @ (Op::Mul | Op::Div))) => {
                    self.pos += 1;
                    expr = Expr::Binary(op, Box::new(expr), Box::new(self.unary()?));
                }
                // 2x, 3sin(x), x(x + 1).
                Some(Token::Number(_) | Token::Name(_) | Token::Open) => {
                    expr = Expr::Binary(Op::Mul, Box::new(expr), Box::new(self.power()?));
                }
                _ => return Ok(expr),
            }
        }
    }

    /// unary = ("-" | "+") unary | power
    fn unary(&mut self) -> Result<Expr, PlotError> {
        match self.peek() {
            Some(Token::Op(Op::Sub)) => {
                self.pos += 1;
                Ok(Expr::Neg(Box::new(self.unary()?)))
            }
            Some(Token::Op(Op::Add)) => {
                self.pos += 1;
                self.unary()
            }
            _ => self.power(),
        }
    }

    /// power = atom ("^" unary)?   (right-associative: 2^3^2 = 2^9)
    fn power(&mut self) -> Result<Expr, PlotError> {
        let base = self.atom()?;
        if self.peek() == Some(Token::Op(Op::Pow)) {
            self.pos += 1;
            return Ok(Expr::Binary(
                Op::Pow,
                Box::new(base),
                Box::new(self.unary()?),
            ));
        }
        Ok(base)
    }

    /// atom = number | x | constant | function power | "(" sum ")" | "|" sum "|"
    fn atom(&mut self) -> Result<Expr, PlotError> {
        let token = self.peek().ok_or(PlotError::Empty)?;
        self.pos += 1;
        match token {
            Token::Number(v) => Ok(Expr::Number(v)),
            Token::Name(Name::X) => Ok(Expr::X),
            Token::Name(Name::Const(v)) => Ok(Expr::Number(v)),
            Token::Name(Name::Func(f)) => Ok(Expr::Call(f, Box::new(self.power()?))),
            Token::Open => {
                let inner = self.sum()?;
                match self.peek() {
                    Some(Token::Close) => {
                        self.pos += 1;
                        Ok(inner)
                    }
                    _ => Err(PlotError::UnbalancedParentheses),
                }
            }
            Token::Bar => {
                let inner = self.sum()?;
                match self.peek() {
                    Some(Token::Bar) => {
                        self.pos += 1;
                        Ok(Expr::Call(Func::Abs, Box::new(inner)))
                    }
                    _ => Err(PlotError::UnbalancedParentheses),
                }
            }
            Token::Close => Err(PlotError::UnbalancedParentheses),
            Token::Op(_) => Err(PlotError::Unexpected(symbol(token))),
        }
    }
}

/// Functions sampled for drawing.
#[derive(Clone, Debug, PartialEq)]
pub struct Graph {
    /// For each function, its pieces as `(x, y)` points within the ranges; a piece ends
    /// where the function has no value or leaves the y range.
    pub curves: Vec<Vec<Vec<(f64, f64)>>>,
    pub x: (f64, f64),
    pub y: (f64, f64),
}

/// Samples `functions` over `x0..x1`. The y range takes in their values, leaving out
/// spikes (like `tan` near its poles) so the rest stays readable.
pub fn graph(functions: &[Expr], x0: f64, x1: f64) -> Result<Graph, PlotError> {
    if !(x0.is_finite() && x1.is_finite() && x1 > x0) {
        return Err(PlotError::BadRange);
    }
    let xs: Vec<f64> = (0..=SAMPLES)
        .map(|i| x0 + (x1 - x0) * i as f64 / SAMPLES as f64)
        .collect();
    let values: Vec<Vec<f64>> = functions
        .iter()
        .map(|f| xs.iter().map(|&x| f.eval(x)).collect())
        .collect();
    let mut finite: Vec<f64> = values
        .iter()
        .flatten()
        .copied()
        .filter(|v| v.is_finite())
        .collect();
    if finite.is_empty() {
        return Err(PlotError::NoValues);
    }
    finite.sort_by(f64::total_cmp);
    let at = |q: f64| finite[((finite.len() - 1) as f64 * q).round() as usize];
    let (lo, hi) = (at(0.0), at(1.0));
    // Spikes: the middle 90% of values, widened, unless everything fits in a bit more.
    let (p5, p95) = (at(0.05), at(0.95));
    let wide = (p95 - p5).max(1e-9) * 3.0;
    let (mut y0, mut y1) = if hi - lo <= wide {
        (lo, hi)
    } else {
        (lo.max(p5 - wide / 3.0), hi.min(p95 + wide / 3.0))
    };
    if y1 - y0 < 1e-9 {
        let pad = if y0.abs() > 1e-9 { y0.abs() * 0.5 } else { 1.0 };
        (y0, y1) = (y0 - pad, y1 + pad);
    } else {
        let pad = (y1 - y0) * 0.08;
        (y0, y1) = (y0 - pad, y1 + pad);
    }
    let curves = values.iter().map(|ys| pieces(&xs, ys, y0, y1)).collect();
    Ok(Graph {
        curves,
        x: (x0, x1),
        y: (y0, y1),
    })
}

/// The parts of the sampled curve inside `y0..=y1`, cut where it leaves (at the edge) or
/// has no value.
fn pieces(xs: &[f64], ys: &[f64], y0: f64, y1: f64) -> Vec<Vec<(f64, f64)>> {
    let inside = |y: f64| y.is_finite() && (y0..=y1).contains(&y);
    let mut out = Vec::new();
    let mut piece: Vec<(f64, f64)> = Vec::new();
    for i in 0..xs.len() {
        let (x, y) = (xs[i], ys[i]);
        let prev = i.checked_sub(1).map(|j| (xs[j], ys[j]));
        if inside(y) {
            // Coming back in: start at the edge it crossed.
            if piece.is_empty()
                && let Some((px, py)) = prev
                && py.is_finite()
                && !inside(py)
            {
                let edge = if py > y1 { y1 } else { y0 };
                piece.push((px + (x - px) * (edge - py) / (y - py), edge));
            }
            piece.push((x, y));
        } else {
            // Going out: end at the edge.
            if let Some(&(px, py)) = piece.last()
                && y.is_finite()
            {
                let edge = if y > y1 { y1 } else { y0 };
                piece.push((px + (x - px) * (edge - py) / (y - py), edge));
            }
            if piece.len() > 1 {
                out.push(std::mem::take(&mut piece));
            }
            piece.clear();
        }
    }
    if piece.len() > 1 {
        out.push(piece);
    }
    out
}

/// Tick positions across `lo..=hi`: round steps of 1, 2 or 5 times a power of ten, about
/// `count` of them.
pub fn ticks(lo: f64, hi: f64, count: usize) -> Vec<f64> {
    let span = hi - lo;
    if !(span.is_finite() && span > 0.0) || count == 0 {
        return Vec::new();
    }
    let rough = span / count as f64;
    let power = 10f64.powf(rough.log10().floor());
    let step = [1.0, 2.0, 5.0, 10.0]
        .into_iter()
        .map(|m| m * power)
        .find(|s| *s >= rough)
        .unwrap_or(10.0 * power);
    let first = (lo / step).ceil() as i64;
    let last = (hi / step).floor() as i64;
    (first..=last)
        .map(|i| {
            let v = i as f64 * step;
            // No "-0" or 0.30000000000000004.
            if v.abs() < step * 1e-9 {
                0.0
            } else {
                (v / step).round() * step
            }
        })
        .collect()
}

/// A tick label: as short as its step allows.
pub fn tick_label(value: f64) -> String {
    let text = format!("{value:.6}");
    let text = text.trim_end_matches('0').trim_end_matches('.');
    if text == "-0" {
        "0".into()
    } else {
        text.into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(src: &str, x: f64) -> f64 {
        parse(src).unwrap_or_else(|e| panic!("{src}: {e}")).eval(x)
    }

    #[test]
    fn formulas_read_as_written_by_hand() {
        assert_eq!(at("x^2 - 3x + 1", 2.0), -1.0);
        assert_eq!(at("y = 2x", 3.0), 6.0);
        assert_eq!(at("f(x) = x(x+1)", 2.0), 6.0);
        assert!((at("sin(x)", std::f64::consts::FRAC_PI_2) - 1.0).abs() < 1e-12);
        assert!((at("3sin x", std::f64::consts::FRAC_PI_2) - 3.0).abs() < 1e-12);
        assert!((at("sinh(0)+cosh(0)", 1.0) - 1.0).abs() < 1e-12);
        assert_eq!(at("|x - 5|", 2.0), 3.0);
        assert_eq!(at("√x", 9.0), 3.0);
        assert_eq!(at("x²", -3.0), 9.0);
        assert_eq!(at("-x^2", 3.0), -9.0);
        assert_eq!(at("2^3^2", 0.0), 512.0);
        assert_eq!(at("2,5x", 2.0), 5.0);
        assert!((at("2πx", 1.0) - std::f64::consts::TAU).abs() < 1e-12);
        assert!((at("lg(100) + ln(e)", 0.0) - 3.0).abs() < 1e-12);
        assert!((at("tg(x)", 1.0) - 1f64.tan()).abs() < 1e-12);
        assert_eq!(at("1/x", 0.0), f64::INFINITY);
    }

    #[test]
    fn mistakes_are_reported() {
        assert_eq!(parse(""), Err(PlotError::Empty));
        assert_eq!(parse("y ="), Err(PlotError::Empty));
        assert_eq!(parse("(x+1"), Err(PlotError::UnbalancedParentheses));
        assert_eq!(parse("x)"), Err(PlotError::UnbalancedParentheses));
        assert_eq!(parse("foo(x)"), Err(PlotError::Unexpected("foo".into())));
        assert_eq!(parse("x +"), Err(PlotError::Empty));
    }

    #[test]
    fn graphs_fit_their_values_and_break_at_poles() {
        let g = graph(&[parse("x^2").unwrap()], -2.0, 2.0).unwrap();
        assert!(
            g.y.0 < 0.0 && g.y.0 > -1.0 && g.y.1 > 4.0 && g.y.1 < 5.0,
            "{:?}",
            g.y
        );
        assert_eq!(g.curves[0].len(), 1);
        // 1/x: two branches, spikes near 0 cut off.
        let g = graph(&[parse("1/x").unwrap()], -5.0, 5.0).unwrap();
        assert_eq!(g.curves[0].len(), 2, "{:?}", g.y);
        assert!(g.y.1 < 1000.0);
        for piece in &g.curves[0] {
            assert!(
                piece
                    .iter()
                    .all(|&(_, y)| y >= g.y.0 - 1e-9 && y <= g.y.1 + 1e-9)
            );
        }
        // A constant still gets a range.
        let g = graph(&[parse("3").unwrap()], 0.0, 1.0).unwrap();
        assert!(g.y.0 < 3.0 && g.y.1 > 3.0);
        assert_eq!(
            graph(&[parse("sqrt(x)").unwrap()], -2.0, -1.0),
            Err(PlotError::NoValues)
        );
        assert_eq!(
            graph(&[parse("x").unwrap()], 1.0, 1.0),
            Err(PlotError::BadRange)
        );
    }

    #[test]
    fn ticks_are_round() {
        assert_eq!(ticks(-2.0, 2.0, 4), [-2.0, -1.0, 0.0, 1.0, 2.0]);
        assert_eq!(ticks(0.0, 10.0, 5), [0.0, 2.0, 4.0, 6.0, 8.0, 10.0]);
        let labels: Vec<_> = ticks(0.05, 0.31, 3).into_iter().map(tick_label).collect();
        assert_eq!(labels, ["0.1", "0.2", "0.3"]);
        assert_eq!(tick_label(0.30000000000000004), "0.3");
        assert_eq!(tick_label(-2.0), "-2");
    }
}
