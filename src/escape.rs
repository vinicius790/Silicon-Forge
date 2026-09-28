//! String and character-literal escapes shared by the assembler, its preprocessor
//! and the expression evaluator: `\"` `\'` `\\` `\n` `\r` `\t` `\0` and `\xHH`
//! (exactly two hex digits). Any other character after `\` is an error.

/// Decode one escape; `s` starts just after the backslash. Returns the byte and how
/// many bytes of `s` it used.
pub fn decode_escape(s: &str) -> Result<(u8, usize), String> {
    let mut chars = s.chars();
    let c = chars.next().ok_or("escape incompleto no fim")?;
    Ok(match c {
        '"' => (b'"', 1),
        '\'' => (b'\'', 1),
        '\\' => (b'\\', 1),
        'n' => (b'\n', 1),
        'r' => (b'\r', 1),
        't' => (b'\t', 1),
        '0' => (0, 1),
        'x' | 'X' => {
            let hex = s
                .get(1..3)
                .filter(|h| h.bytes().all(|b| b.is_ascii_hexdigit()));
            let hex = hex.ok_or_else(|| {
                format!(
                    "\\x exige dois dígitos hex: \\{}",
                    s.chars().take(3).collect::<String>()
                )
            })?;
            (u8::from_str_radix(hex, 16).expect("checked hex"), 3)
        }
        other => return Err(format!("escape desconhecido: \\{other}")),
    })
}

/// Bytes of a string body (between the quotes), escapes decoded. An unescaped `"`
/// inside is an error.
pub fn decode_string(body: &str) -> Result<Vec<u8>, String> {
    let mut out = Vec::with_capacity(body.len());
    let mut i = 0;
    while i < body.len() {
        let rest = &body[i..];
        if let Some(after) = rest.strip_prefix('\\') {
            let (b, used) = decode_escape(after)?;
            out.push(b);
            i += 1 + used;
        } else if rest.starts_with('"') {
            return Err("aspas internas sem escape".into());
        } else {
            let c = rest.chars().next().expect("non-empty");
            let mut buf = [0u8; 4];
            out.extend_from_slice(c.encode_utf8(&mut buf).as_bytes());
            i += c.len_utf8();
        }
    }
    Ok(out)
}

/// `s` starts with `"`: byte index of the closing quote, honouring `\` escapes.
pub fn string_end(s: &str) -> Option<usize> {
    let bytes = s.as_bytes();
    let mut i = 1;
    while i < bytes.len() {
        match bytes[i] {
            b'\\' => i += 2,
            b'"' => return Some(i),
            _ => i += 1,
        }
    }
    None
}

/// `s` starts with `'`: total length of a well-formed character literal (`'a'`,
/// `'\n'`, `'\x41'`), or `None` when it is not one.
pub fn char_literal_len(s: &str) -> Option<usize> {
    let rest = s.strip_prefix('\'')?;
    let inner = if let Some(after) = rest.strip_prefix('\\') {
        1 + decode_escape(after).ok()?.1
    } else {
        let c = rest.chars().next()?;
        if c == '\'' {
            return None;
        }
        c.len_utf8()
    };
    (rest.as_bytes().get(inner) == Some(&b'\'')).then_some(inner + 2)
}

/// Value of a whole character literal: one ASCII character or one escape.
pub fn char_value(s: &str) -> Result<u8, String> {
    let inner = s
        .strip_prefix('\'')
        .and_then(|r| r.strip_suffix('\''))
        .ok_or_else(|| format!("aspas simples sem fechar: {s}"))?;
    if inner.is_empty() {
        return Err("caractere vazio".into());
    }
    if let Some(after) = inner.strip_prefix('\\') {
        let (b, used) = decode_escape(after)?;
        if used != after.len() {
            return Err("caractere maior que um".into());
        }
        return Ok(b);
    }
    let mut chars = inner.chars();
    match (chars.next(), chars.next()) {
        (Some(c), None) if c.is_ascii() => Ok(c as u8),
        (Some(c), None) => Err(format!("caractere fora de ASCII: {c}")),
        _ => Err("caractere maior que um".into()),
    }
}
