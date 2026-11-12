//! `field-value-calculator/math-expression.ts`: safe arithmetic for the
//! "custom formula" number operation. Never panics; any failure is `None`
//! (`{ ok: false }`).

use std::collections::HashMap;
use std::sync::OnceLock;

use regex::Regex;

use crate::js;

/// `MATH_EXPRESSION_LIMITS`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MathExpressionLimits {
    pub max_length: usize,
    pub max_tokens: usize,
    pub max_depth: usize,
    pub max_arguments: usize,
}

/// `MATH_EXPRESSION_LIMITS`.
pub const MATH_EXPRESSION_LIMITS: MathExpressionLimits = MathExpressionLimits {
    max_length: 1000,
    max_tokens: 256,
    max_depth: 32,
    max_arguments: 16,
};

#[derive(Debug, Clone, PartialEq)]
enum Token {
    Number(f64),
    Name(String),
    Operator(char),
}

#[derive(Debug, Clone, PartialEq)]
enum Node {
    Number(f64),
    Variable(String),
    Unary(char, Box<Node>),
    Binary(char, Box<Node>, Box<Node>),
    Call(String, Vec<Node>),
}

fn token_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r"^\s*(?:([0-9]+\.?[0-9]*(?:e[+-]?[0-9]+)?|\.[0-9]+(?:e[+-]?[0-9]+)?)|([a-z_][a-z0-9_]*)|([-+*/%(),]))")
            .expect("valid regex")
    })
}

fn tokenize(source: &str) -> Option<Vec<Token>> {
    let mut tokens = Vec::new();
    let mut pos = 0;
    while pos < source.len() {
        let rest = &source[pos..];
        let Some(caps) = token_re().captures(rest) else {
            if js::trim(rest).is_empty() {
                break;
            }
            return None;
        };
        if let Some(number) = caps.get(1) {
            let value = js::to_number(number.as_str());
            if !value.is_finite() {
                return None;
            }
            tokens.push(Token::Number(value));
        } else if let Some(name) = caps.get(2) {
            tokens.push(Token::Name(name.as_str().to_owned()));
        } else if let Some(op) = caps.get(3) {
            tokens.push(Token::Operator(op.as_str().chars().next()?));
        }
        if tokens.len() > MATH_EXPRESSION_LIMITS.max_tokens {
            return None;
        }
        pos += caps.get(0).map_or(0, |m| m.end());
    }
    Some(tokens)
}

struct Parser<'a> {
    tokens: &'a [Token],
    position: usize,
    depth: usize,
}

impl Parser<'_> {
    fn peek_operator(&self) -> Option<char> {
        match self.tokens.get(self.position) {
            Some(Token::Operator(op)) => Some(*op),
            _ => None,
        }
    }

    fn expect_operator(&mut self, op: char) -> Option<()> {
        if self.peek_operator() != Some(op) {
            return None;
        }
        self.position += 1;
        Some(())
    }

    fn enter(&mut self) -> Option<()> {
        self.depth += 1;
        (self.depth <= MATH_EXPRESSION_LIMITS.max_depth).then_some(())
    }

    fn additive(&mut self) -> Option<Node> {
        self.enter()?;
        let mut left = self.multiplicative()?;
        while let Some(op @ ('+' | '-')) = self.peek_operator() {
            self.position += 1;
            left = Node::Binary(op, Box::new(left), Box::new(self.multiplicative()?));
        }
        self.depth -= 1;
        Some(left)
    }

    fn multiplicative(&mut self) -> Option<Node> {
        let mut left = self.unary()?;
        while let Some(op @ ('*' | '/' | '%')) = self.peek_operator() {
            self.position += 1;
            left = Node::Binary(op, Box::new(left), Box::new(self.unary()?));
        }
        Some(left)
    }

    fn unary(&mut self) -> Option<Node> {
        match self.peek_operator() {
            Some(op @ ('-' | '+')) => {
                self.position += 1;
                self.enter()?;
                let operand = self.unary()?;
                self.depth -= 1;
                Some(Node::Unary(op, Box::new(operand)))
            }
            _ => self.primary(),
        }
    }

    fn arguments(&mut self) -> Option<Vec<Node>> {
        let mut args = Vec::new();
        if self.peek_operator() == Some(')') {
            return Some(args);
        }
        args.push(self.additive()?);
        while self.peek_operator() == Some(',') {
            self.position += 1;
            args.push(self.additive()?);
            if args.len() > MATH_EXPRESSION_LIMITS.max_arguments {
                return None;
            }
        }
        Some(args)
    }

    fn primary(&mut self) -> Option<Node> {
        let token = self.tokens.get(self.position)?.clone();
        self.position += 1;
        match token {
            Token::Number(value) => Some(Node::Number(value)),
            Token::Name(name) => {
                if self.peek_operator() != Some('(') {
                    return Some(Node::Variable(name));
                }
                self.position += 1;
                let args = self.arguments()?;
                self.expect_operator(')')?;
                Some(Node::Call(name, args))
            }
            Token::Operator('(') => {
                let inner = self.additive()?;
                self.expect_operator(')')?;
                Some(inner)
            }
            Token::Operator(_) => None,
        }
    }
}

fn finite(value: f64) -> Option<f64> {
    value.is_finite().then_some(value)
}

fn call(name: &str, args: &[f64]) -> Option<f64> {
    let one = |f: fn(f64) -> f64| (args.len() == 1).then(|| f(args[0]));
    let many = |f: fn(&[f64]) -> f64| (1..=16).contains(&args.len()).then(|| f(args));
    match name {
        "abs" => one(f64::abs),
        "ceil" => one(f64::ceil),
        "floor" => one(f64::floor),
        "round" => one(js::math_round),
        "sqrt" => one(f64::sqrt),
        "exp" => one(f64::exp),
        "log" => one(f64::ln),
        "logten" => one(f64::log10),
        "sin" => one(f64::sin),
        "cos" => one(f64::cos),
        "tan" => one(f64::tan),
        "asin" => one(f64::asin),
        "acos" => one(f64::acos),
        "atan" => one(f64::atan),
        "min" => many(|a| {
            a.iter().copied().fold(f64::INFINITY, |m, v| {
                if v.is_nan() || m.is_nan() {
                    f64::NAN
                } else {
                    m.min(v)
                }
            })
        }),
        "max" => many(|a| {
            a.iter().copied().fold(f64::NEG_INFINITY, |m, v| {
                if v.is_nan() || m.is_nan() {
                    f64::NAN
                } else {
                    m.max(v)
                }
            })
        }),
        "hypot" => many(|a| a.iter().fold(0.0f64, |acc, v| acc.hypot(*v))),
        "pow" => (args.len() == 2).then(|| args[0].powf(args[1])),
        "fmod" => (args.len() == 2).then(|| args[0] % args[1]),
        _ => None,
    }
}

fn evaluate(node: &Node, variables: &HashMap<String, f64>) -> Option<f64> {
    match node {
        Node::Number(value) => Some(*value),
        Node::Variable(name) => variables.get(name).copied().or(match name.as_str() {
            "pi" => Some(std::f64::consts::PI),
            "e" => Some(std::f64::consts::E),
            _ => None,
        }),
        Node::Unary(op, operand) => {
            let value = evaluate(operand, variables)?;
            Some(if *op == '-' { -value } else { value })
        }
        Node::Binary(op, left, right) => {
            let (l, r) = (evaluate(left, variables)?, evaluate(right, variables)?);
            finite(match op {
                '+' => l + r,
                '-' => l - r,
                '*' => l * r,
                '/' if r == 0.0 => f64::NAN,
                '/' => l / r,
                '%' if r == 0.0 => f64::NAN,
                '%' => l % r,
                _ => return None,
            })
        }
        Node::Call(name, args) => {
            // The arity check comes before the arguments are evaluated.
            call(name, &vec![0.0; args.len()])?;
            let values: Option<Vec<f64>> = args.iter().map(|a| evaluate(a, variables)).collect();
            finite(call(name, &values?)?)
        }
    }
}

/// `evaluateMathExpression`: the value, or `None` for any failure.
pub fn evaluate_math_expression(source: &str, variables: &HashMap<String, f64>) -> Option<f64> {
    if js::utf16_len(source) > MATH_EXPRESSION_LIMITS.max_length {
        return None;
    }
    let tokens = tokenize(&source.to_lowercase())?;
    if tokens.is_empty() {
        return None;
    }
    let mut parser = Parser {
        tokens: &tokens,
        position: 0,
        depth: 0,
    };
    let root = parser.additive()?;
    if parser.position != tokens.len() {
        return None;
    }
    finite(evaluate(&root, variables)?)
}
