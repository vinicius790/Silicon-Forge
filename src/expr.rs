//! Integer expressions for the assembler and the relocating object assembler.
//!
//! Binary operators, loosest first (ca65 order for the logical and comparison ranks):
//!
//! | rank | operators | result |
//! | --- | --- | --- |
//! | 1 | `\|\|`, `.or` | 1 if either side is non-zero, else 0 |
//! | 2 | `.xor` | 1 if exactly one side is non-zero |
//! | 3 | `&&`, `.and` | 1 if both sides are non-zero |
//! | 4 | `==` `!=` `<>` `<` `>` `<=` `>=`, `.eq` `.ne` `.lt` `.gt` `.le` `.ge` | 1 or 0, unsigned 16-bit compare |
//! | 5 | `\|`, `.bitor` | bitwise or |
//! | 6 | `^`, `.bitxor` | bitwise xor |
//! | 7 | `&`, `.bitand` | bitwise and |
//! | 8 | `<<` `>>`, `.shl` `.shr` | shift; a count of 16 or more gives 0 |
//! | 9 | `+` `-` | wrapping |
//! | 10 | `*` `/` `.mod` | wrapping product, unsigned quotient and remainder |
//!
//! Equal ranks associate left (`8-2-1` is 5). `/` and `.mod` by zero are errors.
//! Both sides of `&&`/`||` are always evaluated (no short circuit), so an error on
//! either side is reported. Dot operators are case-insensitive and must stand alone
//! (`a.ltb` is not `a .lt b`).
//!
//! A binary operator is recognised only where an operand has just ended; right after
//! another operator, `<`, `>`, `-`, `~`, `!` and `.not` are prefixes. So `a < <b`
//! compares `a` with the low byte of `b`, and `2*-1` is `$FFFE`. `!x` and `.not x`
//! give 1 when `x` is 0, else 0; `-` negates (two's complement) and `~` inverts the
//! 16 bits. These bind tighter than every binary operator.
//! A `<` or `>` at the very start of the expression (not part of `<<`/`>>`/`<=`/
//! `>=`/`<>`) takes the low or high byte of the whole rest, as before: `<a == 3` is
//! `<(a == 3)`; write `(<a) == 3` to compare the low byte. Bare numbers are hexadecimal (`10` is 16, `0d10` is ten).

use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExprError(pub String);

impl fmt::Display for ExprError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for ExprError {}

/// Symbolic operators, longest first so `<<` wins over `<` and `<=` over `<`.
const OPS: &[(&str, u8)] = &[
    ("||", 1),
    ("&&", 3),
    ("==", 4),
    ("!=", 4),
    ("<>", 4),
    ("<=", 4),
    (">=", 4),
    ("<<", 8),
    (">>", 8),
    ("<", 4),
    (">", 4),
    ("|", 5),
    ("^", 6),
    ("&", 7),
    ("+", 9),
    ("-", 9),
    ("*", 10),
    ("/", 10),
];

/// Word operators (written with a leading `.`), mapped to the symbolic spelling.
const WORD_OPS: &[(&str, &str, u8)] = &[
    (".or", "||", 1),
    (".xor", "^^", 2),
    (".and", "&&", 3),
    (".eq", "==", 4),
    (".ne", "!=", 4),
    (".lt", "<", 4),
    (".gt", ">", 4),
    (".le", "<=", 4),
    (".ge", ">=", 4),
    (".bitor", "|", 5),
    (".bitxor", "^", 6),
    (".bitand", "&", 7),
    (".shl", "<<", 8),
    (".shr", ">>", 8),
    (".mod", "%", 10),
];

/// Length of the word operator `word` at the start of `rest`, if it stands alone.
fn word_at(rest: &str, word: &str) -> bool {
    rest.len() >= word.len()
        && rest.is_char_boundary(word.len())
        && rest[..word.len()].eq_ignore_ascii_case(word)
        && !rest[word.len()..]
            .chars()
            .next()
            .is_some_and(|c| c.is_ascii_alphanumeric() || c == '_')
}

pub fn eval(input: &str, lookup: &dyn Fn(&str) -> Option<u16>) -> Result<u16, ExprError> {
    parse(input.trim(), lookup)
}

fn parse(s: &str, lookup: &dyn Fn(&str) -> Option<u16>) -> Result<u16, ExprError> {
    let s = s.trim();
    if s.is_empty() {
        return Err(ExprError("expressão vazia".into()));
    }
    if let Some(rest) = byte_prefix(s, '<') {
        return Ok(parse(rest, lookup)? & 0xFF);
    }
    if let Some(rest) = byte_prefix(s, '>') {
        return Ok((parse(rest, lookup)? >> 8) & 0xFF);
    }
    if let Some((lhs, op, rhs)) = split_loose(s)? {
        if op == "+" && is_char_const(lhs) && is_char_const(rhs) {
            return Err(ExprError("constantes de caractere não se somam".into()));
        }
        return apply(op, parse(lhs, lookup)?, parse(rhs, lookup)?);
    }
    if let Some(rest) = s.strip_prefix('!') {
        return Ok(u16::from(parse(rest, lookup)? == 0));
    }
    if word_at(s, ".not") {
        return Ok(u16::from(parse(&s[4..], lookup)? == 0));
    }
    if let Some(rest) = s.strip_prefix('~') {
        return Ok(!parse(rest, lookup)?);
    }
    if let Some(rest) = s.strip_prefix('-') {
        return Ok(parse(rest, lookup)?.wrapping_neg());
    }
    primary(s, lookup)
}

fn byte_prefix(s: &str, op: char) -> Option<&str> {
    let mut chars = s.chars();
    let next = {
        if chars.next() != Some(op) {
            return None;
        }
        chars.next()
    };
    if next == Some(op) || next == Some('=') || (op == '<' && next == Some('>')) {
        return None;
    }
    Some(s[op.len_utf8()..].trim())
}

fn split_loose(s: &str) -> Result<Option<(&str, &str, &str)>, ExprError> {
    let mut depth = 0i32;
    let mut skip_until = 0;
    // True at the start and right after a binary operator: an operator character
    // here is a prefix, not a binary operator.
    let mut expect_operand = true;
    let mut best: Option<(usize, usize, &'static str, u8)> = None;
    for (idx, ch) in s.char_indices() {
        if idx < skip_until {
            continue;
        }
        match ch {
            '\'' => {
                if let Some(n) = crate::escape::char_literal_len(&s[idx..]) {
                    skip_until = idx + n;
                }
                expect_operand = false;
            }
            '(' => {
                depth += 1;
                expect_operand = true;
            }
            ')' => {
                depth -= 1;
                if depth < 0 {
                    return Err(ExprError("parênteses desbalanceados".into()));
                }
                expect_operand = false;
            }
            c if c.is_whitespace() => {}
            _ if depth == 0 && !expect_operand => {
                let rest = &s[idx..];
                let found = OPS
                    .iter()
                    .find(|(op, _)| rest.starts_with(op))
                    .map(|(op, rank)| (op.len(), *op, *rank))
                    .or_else(|| {
                        WORD_OPS
                            .iter()
                            .find(|(word, _, _)| word_at(rest, word))
                            .map(|(word, op, rank)| (word.len(), *op, *rank))
                    });
                match found {
                    Some((len, op, rank)) => {
                        if s[idx + len..].trim().is_empty() {
                            return Err(ExprError(format!(
                                "falta o operando depois de {}",
                                &rest[..len]
                            )));
                        }
                        if best.is_none_or(|(_, _, _, have)| rank <= have) {
                            best = Some((idx, len, op, rank));
                        }
                        skip_until = idx + len;
                        expect_operand = true;
                    }
                    None => expect_operand = false,
                }
            }
            _ => {
                if depth == 0 && expect_operand && word_at(&s[idx..], ".not") {
                    skip_until = idx + 4;
                } else if !(depth == 0
                    && expect_operand
                    && matches!(ch, '<' | '>' | '-' | '~' | '!'))
                {
                    expect_operand = false;
                }
            }
        }
    }
    if depth != 0 {
        return Err(ExprError("parênteses desbalanceados".into()));
    }
    Ok(best.map(|(idx, len, op, _)| (&s[..idx], op, &s[idx + len..])))
}

fn apply(op: &str, left: u16, right: u16) -> Result<u16, ExprError> {
    Ok(match op {
        "/" => left
            .checked_div(right)
            .ok_or_else(|| ExprError("divisão por zero".into()))?,
        "+" => left.wrapping_add(right),
        "-" => left.wrapping_sub(right),
        "*" => left.wrapping_mul(right),
        "&" => left & right,
        "|" => left | right,
        "^" => left ^ right,
        "%" => left
            .checked_rem(right)
            .ok_or_else(|| ExprError("resto por zero".into()))?,
        "||" => u16::from(left != 0 || right != 0),
        "^^" => u16::from((left != 0) != (right != 0)),
        "&&" => u16::from(left != 0 && right != 0),
        "==" => u16::from(left == right),
        "!=" | "<>" => u16::from(left != right),
        "<" => u16::from(left < right),
        ">" => u16::from(left > right),
        "<=" => u16::from(left <= right),
        ">=" => u16::from(left >= right),
        "<<" => {
            if right >= 16 {
                0
            } else {
                left << right
            }
        }
        ">>" => {
            if right >= 16 {
                0
            } else {
                left >> right
            }
        }
        _ => unreachable!("operator comes from OPS or WORD_OPS"),
    })
}

fn primary(s: &str, lookup: &dyn Fn(&str) -> Option<u16>) -> Result<u16, ExprError> {
    let s = s.trim();
    if let Some((name, inner)) = call(s) {
        let name = name.to_ascii_lowercase();
        if matches!(name.as_str(), "hi" | "lo") && is_char_const(inner) {
            return Err(ExprError("caractere não é rótulo de 16 bits".into()));
        }
        let value = parse(inner, lookup)?;
        return match name.as_str() {
            "hi" => Ok((value >> 8) & 0xFF),
            "lo" => Ok(value & 0xFF),
            _ => Err(ExprError(format!("símbolo ou número inválido: {s}"))),
        };
    }
    if s.starts_with('(') {
        if let Some(inner) = wrapped(s) {
            return parse(inner, lookup);
        }
        return Err(ExprError("parênteses desbalanceados".into()));
    }
    if is_ident(s) {
        if let Some(value) = lookup(&s.to_ascii_lowercase()) {
            return Ok(value);
        }
    }
    literal(s)
}

fn call(s: &str) -> Option<(&str, &str)> {
    let mut end = 0usize;
    for (idx, ch) in s.char_indices() {
        if idx == 0 {
            if !ch.is_ascii_alphabetic() {
                return None;
            }
            end = idx + ch.len_utf8();
            continue;
        }
        if ch.is_ascii_alphanumeric() || ch == '_' {
            end = idx + ch.len_utf8();
        } else {
            break;
        }
    }
    if end == 0 {
        return None;
    }
    let rest = s[end..].trim_start();
    let inner = wrapped(rest)?;
    Some((&s[..end], inner))
}

fn wrapped(s: &str) -> Option<&str> {
    if !s.starts_with('(') || !s.ends_with(')') {
        return None;
    }
    let mut depth = 0i32;
    for (idx, ch) in s.char_indices() {
        match ch {
            '(' => depth += 1,
            ')' => {
                depth -= 1;
                if depth == 0 {
                    return (idx == s.len() - 1).then_some(&s[1..idx]);
                }
            }
            _ => {}
        }
    }
    None
}

fn is_ident(s: &str) -> bool {
    let mut chars = s.chars();
    match chars.next() {
        Some(ch) if ch.is_ascii_alphabetic() || ch == '_' => {}
        _ => return false,
    }
    chars.all(|ch| ch.is_ascii_alphanumeric() || ch == '_')
}

fn is_char_const(s: &str) -> bool {
    let s = s.trim();
    crate::escape::char_literal_len(s) == Some(s.len())
}

fn literal(s: &str) -> Result<u16, ExprError> {
    let s = s.trim();
    if s.starts_with('\'') {
        return crate::escape::char_value(s)
            .map(u16::from)
            .map_err(ExprError);
    }
    let (digits, radix) = if let Some(v) = s
        .strip_prefix('$')
        .or_else(|| s.strip_prefix("0x"))
        .or_else(|| s.strip_prefix("0X"))
    {
        (v, 16)
    } else if let Some(v) = s.strip_prefix('%') {
        (v, 2)
    } else if let Some(v) = s.strip_prefix("0d").or_else(|| s.strip_prefix("0D")) {
        (v, 10)
    } else {
        (s, 16)
    };
    u16::from_str_radix(digits, radix)
        .map_err(|_| ExprError(format!("símbolo ou número inválido: {s}")))
}
