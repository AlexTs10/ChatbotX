//! `expression/*`: references and the If / Switch expression grammar (only
//! the parse check; the runner evaluates in TypeScript).

use std::sync::OnceLock;

use regex::Regex;

use crate::js::trim;

/// `isNestedExpression`: a backtick-wrapped Flow JSON nested expression.
pub fn is_nested_expression(text: &str) -> bool {
    let t = trim(text);
    t.chars().count() >= 2 && t.starts_with('`') && t.ends_with('`')
}

/// `unwrapNestedExpression`.
pub fn unwrap_nested_expression(text: &str) -> &str {
    let t = trim(text);
    if is_nested_expression(t) { &t[1..t.len() - 1] } else { t }
}

/// `MiniAppReference`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reference {
    pub raw: String,
    pub screen_id: Option<String>,
    /// `form` or `data`.
    pub source: String,
    pub name: String,
}

fn screen_reference() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"^screen\.([A-Za-z_][A-Za-z0-9_]*)\.(form|data)\.([A-Za-z0-9_]+)$").expect("regex"))
}
fn local_reference() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"^(form|data)\.([A-Za-z0-9_]+)$").expect("regex"))
}
fn reference_pattern() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"\$\{([^}]*)\}").expect("regex"))
}

/// `parseReference`: the inside of a `${...}`.
pub fn parse_reference(raw: &str) -> Option<Reference> {
    let raw = trim(raw);
    if let Some(c) = screen_reference().captures(raw) {
        return Some(Reference {
            raw: raw.to_owned(),
            screen_id: Some(c[1].to_owned()),
            source: c[2].to_owned(),
            name: c[3].to_owned(),
        });
    }
    let c = local_reference().captures(raw)?;
    Some(Reference { raw: raw.to_owned(), screen_id: None, source: c[1].to_owned(), name: c[2].to_owned() })
}

/// `extractReferences`: every `${...}` (raw inside, parsed reference).
pub fn extract_references(text: &str) -> Vec<(String, Option<Reference>)> {
    reference_pattern()
        .captures_iter(text)
        .map(|c| (c[1].to_owned(), parse_reference(&c[1])))
        .collect()
}

#[derive(Debug, Clone, PartialEq)]
enum Token {
    Operand,
    Operator(&'static str),
    Open,
    Close,
}

const OPERATORS: &[&str] = &["==", "!=", ">=", "<=", "&&", "||", ">", "<", "!"];

fn number_len(rest: &str) -> Option<usize> {
    let bytes = rest.as_bytes();
    let mut i = usize::from(bytes.first() == Some(&b'-'));
    let start = i;
    while bytes.get(i).is_some_and(u8::is_ascii_digit) {
        i += 1;
    }
    if i == start {
        return None;
    }
    if bytes.get(i) == Some(&b'.') && bytes.get(i + 1).is_some_and(u8::is_ascii_digit) {
        i += 1;
        while bytes.get(i).is_some_and(u8::is_ascii_digit) {
            i += 1;
        }
    }
    Some(i)
}

fn tokenize(input: &str) -> Option<Vec<Token>> {
    let mut tokens = Vec::new();
    let mut i = 0;
    while let Some(c) = input[i..].chars().next() {
        let rest = &input[i..];
        if c.is_whitespace() {
            i += c.len_utf8();
            continue;
        }
        match c {
            '(' | ')' => {
                tokens.push(if c == '(' { Token::Open } else { Token::Close });
                i += 1;
                continue;
            }
            '\'' | '"' => {
                let mut chars = rest.char_indices().skip(1);
                let mut end = None;
                while let Some((j, ch)) = chars.next() {
                    if ch == '\\' && j + 1 < rest.len() {
                        chars.next();
                    } else if ch == c {
                        end = Some(j);
                        break;
                    }
                }
                i += end? + 1;
                tokens.push(Token::Operand);
                continue;
            }
            '$' if rest[1..].starts_with('{') => {
                let end = rest.find('}')?;
                parse_reference(&rest[2..end])?;
                tokens.push(Token::Operand);
                i += end + 1;
                continue;
            }
            _ => {}
        }
        let expects_operand = matches!(tokens.last(), None | Some(Token::Operator(_) | Token::Open));
        if let Some(len) = number_len(rest)
            && (c != '-' || expects_operand)
        {
            tokens.push(Token::Operand);
            i += len;
            continue;
        }
        if let Some(op) = OPERATORS.iter().find(|op| rest.starts_with(**op)) {
            tokens.push(Token::Operator(op));
            i += op.len();
            continue;
        }
        let word: String = rest.chars().take_while(|ch| ch.is_ascii_alphabetic() || *ch == '_').collect();
        if word == "true" || word == "false" {
            tokens.push(Token::Operand);
            i += word.len();
            continue;
        }
        return None;
    }
    Some(tokens)
}

const LEVELS: &[&[&str]] = &[&["||"], &["&&"], &["==", "!="], &[">", ">=", "<", "<="]];

struct Parser {
    tokens: Vec<Token>,
    position: usize,
}

impl Parser {
    fn primary(&mut self) -> Option<()> {
        let token = self.tokens.get(self.position)?.clone();
        self.position += 1;
        match token {
            Token::Operand => Some(()),
            Token::Open => {
                self.level(0)?;
                let closing = self.tokens.get(self.position).cloned();
                self.position += 1;
                (closing == Some(Token::Close)).then_some(())
            }
            Token::Operator("!") => self.primary(),
            _ => None,
        }
    }

    fn level(&mut self, level: usize) -> Option<()> {
        if level >= LEVELS.len() {
            return self.primary();
        }
        self.level(level + 1)?;
        while let Some(Token::Operator(op)) = self.tokens.get(self.position)
            && LEVELS[level].contains(op)
        {
            self.position += 1;
            self.level(level + 1)?;
        }
        Some(())
    }
}

/// `isValidExpression`.
pub fn is_valid_expression(expression: &str) -> bool {
    let Some(tokens) = tokenize(unwrap_nested_expression(expression)) else {
        return false;
    };
    let mut parser = Parser { tokens, position: 0 };
    parser.level(0).is_some() && parser.position == parser.tokens.len()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn expressions() {
        assert!(is_valid_expression("${form.age} >= 18 && ${form.ok}"));
        assert!(is_valid_expression("`${form.a} == 'x'`"));
        assert!(is_valid_expression("!(${form.a} || false)"));
        assert!(is_valid_expression("-1 < ${form.a}"));
        assert!(!is_valid_expression("${form.a} =="));
        assert!(!is_valid_expression("(${form.a}"));
        assert!(!is_valid_expression("${foo}"));
        assert!(!is_valid_expression("'open"));
        assert!(!is_valid_expression("maybe"));
    }
}
