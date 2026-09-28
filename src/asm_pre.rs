// Macro expansion, in-memory includes, and local labels.
//
// Included into `asm.rs`. A macro may call macros to a depth of four
// (the call from the program plus three nested calls). The fifth call is an
// error, so a macro that calls itself stops. `.macro` nested inside `.macro`
// is still an error. Local labels are `@name` (scoped to the
// previous global label) and classic numeric labels `1:` referenced as `1b` / `1f`.

/// Every source line carries `\u{1}<n>` (or `\u{1}<file>:<n>` from an include) through the
/// preprocessor so its errors can name the line; the tags are removed before assembly.
const TAG: &str = "\u{1}";

fn tag_lines(src: &str, file: Option<&str>) -> String {
    src.lines()
        .enumerate()
        .map(|(i, raw)| match file {
            Some(f) => format!("{raw}{TAG}{f}:{}\n", i + 1),
            None => format!("{raw}{TAG}{}\n", i + 1),
        })
        .collect()
}

/// `linha <n>: message` when `raw` carries a tag.
/// A line from a macro body carries the definition's tag, then the call's (one per
/// nesting level): `linha <call>: (macro, linha <def>) message`.
fn here(raw: &str, message: impl Into<String>) -> AsmError {
    let message = message.into();
    let tags: Vec<&str> = raw.split(TAG).skip(1).collect();
    match tags.as_slice() {
        [] => AsmError(message),
        [only] => AsmError(format!("linha {only}: {message}")),
        [def, .., call] => AsmError(format!("linha {call}: (macro, linha {def}) {message}")),
    }
}

fn preprocess(
    src: &str,
    files: &HashMap<String, String>,
    blobs: &HashMap<String, Vec<u8>>,
) -> Result<(String, Vec<String>), AsmError> {
    let mut stack = Vec::new();
    let included = expand_includes(&tag_lines(src, None), files, &mut stack)?;
    let packed = expand_macpacks(&included)?;
    let scoped = rewrite_scope_access(&packed);
    let (without_defs, macros) = extract_macros(&scoped)?;
    let expanded = expand_macros(&without_defs, &macros)?;
    let with_strings = expand_string_funcs(&expanded)?;
    let (controlled, warnings) = expand_control(&with_strings, files, blobs)?;
    let structured = expand_anonymous(&expand_procs(&expand_structs(&controlled)?)?)?;
    let mapped = apply_charmap(&structured)?;
    // Tags stay on: the assembler strips them per line and keeps the last one
    // (the line of the original source) for the listing.
    Ok((rewrite_locals(&mapped)?, warnings))
}

/// `.charmap <de>, <para>` translates byte `de` to `para` in every later string of
/// `.text`, `.ascii`, `.asciiz`, `.pstring` and `.byte`/`.db` (the target machine's
/// character set, e.g. screen codes). `.charmap` with no operands restores the
/// identity map. Both operands are expressions in `0..=255`. With the identity map
/// the source is unchanged; otherwise each string becomes a `.byte` list.
fn apply_charmap(src: &str) -> Result<String, AsmError> {
    let mut map: [u8; 256] = std::array::from_fn(|i| i as u8);
    let mut out = String::with_capacity(src.len());
    for raw in src.split_inclusive('\n') {
        let body_end = raw.find(TAG).unwrap_or(raw.trim_end_matches('\n').len());
        let (body, rest) = raw.split_at(body_end);
        let trimmed = body.trim();
        let (label, stmt) = match trimmed.split_once(char::is_whitespace) {
            Some((l, r)) if l.ends_with(':') => (format!("{l} "), r.trim()),
            _ if trimmed.ends_with(':') => (trimmed.to_string(), ""),
            _ => (String::new(), trimmed),
        };
        let (head, args) = stmt.split_once(char::is_whitespace).unwrap_or((stmt, ""));
        let upper = head.to_ascii_uppercase();
        if upper == ".CHARMAP" {
            let args = crate::asm::strip_comment(args);
            if args.is_empty() {
                map = std::array::from_fn(|i| i as u8);
            } else {
                let (a, b) = args
                    .split_once(',')
                    .ok_or_else(|| here(raw.trim_end_matches('\n'), ".charmap exige de, para"))?;
                let byte = |t: &str| -> Result<u8, AsmError> {
                    let v = crate::expr::eval(t.trim(), &|_| None).map_err(|e| here(raw.trim_end_matches('\n'), e.0))?;
                    u8::try_from(v).map_err(|_| here(raw.trim_end_matches('\n'), format!(".charmap fora da faixa: {}", t.trim())))
                };
                let (from, to) = (byte(a)?, byte(b)?);
                map[usize::from(from)] = to;
            }
            out.push_str(&label);
            out.push_str(rest);
            if !rest.ends_with('\n') && raw.ends_with('\n') {
                out.push('\n');
            }
            continue;
        }
        let identity = map.iter().enumerate().all(|(i, &v)| v == i as u8);
        let text_like = matches!(upper.as_str(), ".TEXT" | ".ASCII" | ".ASCIIZ" | ".PSTRING");
        let byte_like = matches!(upper.as_str(), ".BYTE" | ".DB" | "DB");
        if identity || !(text_like || byte_like) || !args.contains('"') {
            out.push_str(raw);
            continue;
        }
        let list = |bytes: &[u8]| -> String {
            bytes.iter().map(|&b| format!("${:02X}", map[usize::from(b)])).collect::<Vec<_>>().join(",")
        };
        let stmt = if text_like {
            let a = args.trim();
            let text = match crate::escape::string_end(a) {
                Some(end) if a.starts_with('"') && end == a.len() - 1 => crate::escape::decode_string(&a[1..end])
                    .map_err(|e| here(raw.trim_end_matches('\n'), e))?,
                _ => return Err(here(raw.trim_end_matches('\n'), format!("string sem aspas correspondentes: {args}"))),
            };
            let mut items = Vec::new();
            if upper == ".PSTRING" {
                let n = u8::try_from(text.len())
                    .map_err(|_| here(raw.trim_end_matches('\n'), format!(".pstring com mais de 255 bytes: {}", text.len())))?;
                items.push(format!("${n:02X}"));
            }
            if !text.is_empty() {
                items.push(list(&text));
            }
            if upper == ".ASCIIZ" {
                items.push("$00".into());
            }
            if items.is_empty() { String::new() } else { format!(".byte {}", items.join(",")) }
        } else {
            let line = raw.trim_end_matches('\n');
            let mut acc = String::new();
            let mut i = 0;
            while i < args.len() {
                let r = &args[i..];
                if r.starts_with('"') {
                    let end = crate::escape::string_end(r)
                        .ok_or_else(|| here(line, format!("string sem aspas correspondentes: {args}")))?;
                    let body = crate::escape::decode_string(&r[1..end]).map_err(|e| here(line, e))?;
                    if body.is_empty() {
                        return Err(here(line, "string vazia em .byte"));
                    }
                    acc += &list(&body);
                    i += end + 1;
                } else {
                    let n = if r.starts_with('\'') {
                        crate::escape::char_literal_len(r).unwrap_or(1)
                    } else {
                        r.chars().next().map_or(1, char::len_utf8)
                    };
                    acc += &r[..n];
                    i += n;
                }
            }
            format!(".byte {acc}")
        };
        out.push_str(&label);
        out.push_str(&stmt);
        out.push_str(rest);
        if !rest.ends_with('\n') && raw.ends_with('\n') {
            out.push('\n');
        }
    }
    Ok(out)
}

fn expand_includes(
    src: &str,
    files: &HashMap<String, String>,
    stack: &mut Vec<String>,
) -> Result<String, AsmError> {
    if stack.len() > 16 {
        return Err(AsmError("inclusão aninhada demais".into()));
    }
    let mut out = String::new();
    for raw in src.lines() {
        let line = strip_comment(raw);
        if let Some(name) = parse_include(line) {
            if stack.iter().any(|seen| seen == name) {
                return Err(here(raw.trim_end_matches('\n'), format!("inclusão cíclica: {name}")));
            }
            let body = files.get(name).ok_or_else(|| {
                here(raw.trim_end_matches('\n'), format!(".include não encontrado no mapa em memória: {name}"))
            })?;
            stack.push(name.to_string());
            let expanded = expand_includes(&tag_lines(body, Some(name)), files, stack)?;
            stack.pop();
            out.push_str(&expanded);
            if !expanded.ends_with('\n') {
                out.push('\n');
            }
        } else {
            out.push_str(raw);
            out.push('\n');
        }
    }
    Ok(out)
}

fn parse_include(line: &str) -> Option<&str> {
    let rest = line
        .strip_prefix(".include")
        .or_else(|| line.strip_prefix(".INCLUDE"))?;
    if !rest.is_empty() && !rest.starts_with(|c: char| c.is_whitespace()) {
        return None;
    }
    let rest = rest.trim();
    if rest.is_empty() {
        return Some("");
    }
    if let Some(inner) = rest.strip_prefix('"') {
        let end = inner.find('"')?;
        return Some(&inner[..end]);
    }
    Some(rest.split_whitespace().next().unwrap_or(""))
}

struct MacroDef {
    params: Vec<String>,
    /// The last parameter was written `name...`: it takes every argument from its
    /// position on, joined with `, ` (possibly none, then it is blank).
    variadic: bool,
    body: Vec<String>,
}

fn extract_macros(src: &str) -> Result<(String, HashMap<String, MacroDef>), AsmError> {
    let mut macros = HashMap::new();
    let mut out = String::new();
    let mut current: Option<(String, MacroDef)> = None;
    let mut opened = String::new();
    for raw in src.lines() {
        let line = strip_comment(raw);
        if let Some((name, params, variadic)) = parse_macro_header(line).map_err(|e| here(raw.trim_end_matches('\n'), e.0))? {
            opened = raw.to_string();
            if current.is_some() {
                return Err(here(raw.trim_end_matches('\n'), 
                    "macro aninhada rejeitada: .macro dentro de .macro",
                ));
            }
            if macros.contains_key(&name) {
                return Err(here(raw.trim_end_matches('\n'), format!("macro duplicada: {name}")));
            }
            current = Some((
                name,
                MacroDef {
                    params,
                    variadic,
                    body: Vec::new(),
                },
            ));
            continue;
        }
        if is_endmacro(line) {
            let Some((name, def)) = current.take() else {
                return Err(here(raw.trim_end_matches('\n'), ".endmacro sem .macro"));
            };
            macros.insert(name, def);
            continue;
        }
        if let Some((_, def)) = current.as_mut() {
            if !line.is_empty() {
                let tag = raw.find(TAG).map_or("", |i| &raw[i..]);
                def.body.push(format!("{line}{tag}"));
            }
        } else {
            out.push_str(raw);
            out.push('\n');
        }
    }
    if current.is_some() {
        return Err(here(&opened, ".macro sem .endmacro"));
    }
    Ok((out, macros))
}

type MacroHeader = (String, Vec<String>, bool);

fn parse_macro_header(line: &str) -> Result<Option<MacroHeader>, AsmError> {
    let Some(rest) = line
        .strip_prefix(".macro")
        .or_else(|| line.strip_prefix(".MACRO"))
    else {
        return Ok(None);
    };
    if !rest.is_empty() && !rest.starts_with(char::is_whitespace) {
        return Ok(None);
    }
    let rest = rest.trim();
    if rest.is_empty() {
        return Err(AsmError(".macro exige um nome".into()));
    }
    let mut tokens = rest
        .split(|c: char| c == ',' || c.is_whitespace())
        .filter(|token| !token.is_empty());
    let name = tokens.next().unwrap().to_ascii_lowercase();
    validate_ident(&name)?;
    let mut params = Vec::new();
    let mut variadic = false;
    for param in tokens {
        if variadic {
            return Err(AsmError(format!("parâmetro depois do variádico em {name}")));
        }
        let mut param = param.to_ascii_lowercase();
        if let Some(base) = param.strip_suffix("...") {
            variadic = true;
            param = base.to_string();
        }
        if params.contains(&param) {
            return Err(AsmError(format!("parâmetro de macro inválido em {name}")));
        }
        validate_ident(&param)?;
        params.push(param);
    }
    Ok(Some((name, params, variadic)))
}

fn is_endmacro(line: &str) -> bool {
    let token = line.split_whitespace().next().unwrap_or("");
    token.eq_ignore_ascii_case(".endmacro") || token.eq_ignore_ascii_case(".endm")
}

fn validate_ident(name: &str) -> Result<(), AsmError> {
    let mut chars = name.chars();
    if !chars
        .next()
        .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        || !chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
    {
        return Err(AsmError(format!("nome inválido: {name}")));
    }
    Ok(())
}

fn expand_macros(src: &str, macros: &HashMap<String, MacroDef>) -> Result<String, AsmError> {
    let mut invocation = 0u32;
    expand_macro_text(src, macros, 0, &mut invocation)
}

/// `depth` 0 is the call from the program. Depths 1, 2 and 3 are nested calls.
/// Depth 4 is the fifth invocation and is rejected, which also bounds recursion.
fn expand_macro_text(
    src: &str,
    macros: &HashMap<String, MacroDef>,
    depth: u32,
    invocation: &mut u32,
) -> Result<String, AsmError> {
    let mut out = String::new();
    for raw in src.lines() {
        let line = strip_comment(raw);
        let tag = raw.find(TAG).map_or("", |i| &raw[i..]);
        if line.is_empty() {
            out.push_str(tag);
            out.push('\n');
            continue;
        }
        let (label, rest) = split_label(line);
        if rest.is_empty() {
            out.push_str(raw.trim_end());
            out.push('\n');
            continue;
        }
        let (name, arg_src) = split_word(rest);
        let call = macros
            .get(&name.to_ascii_lowercase())
            .map(|def| (def.params.clone(), def.variadic, def.body.clone()));
        if let Some((params, variadic, body_lines)) = call {
            if depth >= 4 {
                return Err(here(raw.trim_end_matches('\n'), 
                    "macro aninhada rejeitada: quinto nível",
                ));
            }
            if let Some(label) = label {
                out.push_str(label);
                out.push(':');
                out.push_str(tag);
                out.push('\n');
            }
            let mut args = split_args(arg_src).map_err(|e| here(raw.trim_end_matches('\n'), e.0))?;
            let passed = args.len();
            if variadic {
                let fixed = params.len() - 1;
                if passed < fixed {
                    return Err(here(raw.trim_end_matches('\n'), format!(
                        "macro {name} espera pelo menos {fixed} argumento(s), recebeu {passed}"
                    )));
                }
                let rest = args.split_off(fixed).join(", ");
                args.push(rest);
            } else if passed != params.len() {
                return Err(here(raw.trim_end_matches('\n'), format!(
                    "macro {name} espera {} argumento(s), recebeu {}",
                    params.len(),
                    passed
                )));
            }
            *invocation += 1;
            let id = *invocation;
            let mut body = String::new();
            for body_line in &body_lines {
                let substituted = substitute_params(body_line, &params, &args);
                let substituted = replace_paramcount(&substituted, passed);
                let substituted = rename_macro_locals(&substituted, id);
                body.push_str(&substituted);
                body.push_str(tag);
                body.push('\n');
            }
            out.push_str(&expand_macro_text(&body, macros, depth + 1, invocation)?);
        } else {
            out.push_str(line);
            out.push_str(tag);
            out.push('\n');
        }
    }
    Ok(out)
}

fn split_word(s: &str) -> (&str, &str) {
    let s = s.trim();
    match s.find(char::is_whitespace) {
        Some(i) => (s[..i].trim(), s[i..].trim()),
        None => (s, ""),
    }
}

fn split_args(src: &str) -> Result<Vec<String>, AsmError> {
    if src.trim().is_empty() {
        return Ok(Vec::new());
    }
    let mut args = Vec::new();
    for part in src.split(',') {
        let part = part.trim();
        if part.is_empty() {
            return Err(AsmError("argumento de macro vazio".into()));
        }
        args.push(part.to_string());
    }
    Ok(args)
}

/// `.paramcount` in a body is the number of arguments the call passed (ca65).
fn replace_paramcount(line: &str, passed: usize) -> String {
    let mut out = String::with_capacity(line.len());
    let mut rest = line;
    while let Some(at) = find_ci_unquoted(rest, ".paramcount") {
        let after = &rest[at + ".paramcount".len()..];
        if after.starts_with(|c: char| c.is_ascii_alphanumeric() || c == '_') {
            out.push_str(&rest[..at + 1]);
            rest = &rest[at + 1..];
            continue;
        }
        out.push_str(&rest[..at]);
        out.push_str(&format!("0d{passed}"));
        rest = after;
    }
    out.push_str(rest);
    out
}

fn substitute_params(line: &str, params: &[String], args: &[String]) -> String {
    let mut order: Vec<usize> = (0..params.len()).collect();
    order.sort_by_key(|&i| std::cmp::Reverse(params[i].len()));
    let mut out = line.to_string();
    for i in order {
        let needle = format!("\\{}", params[i]);
        out = out.replace(&needle, &args[i]);
    }
    out
}

/// Give each macro expansion its own `@` local-label prefix so two calls do not clash.
fn rename_macro_locals(line: &str, invocation: u32) -> String {
    let (label, rest) = split_label(line);
    let mut out = String::new();
    if let Some(label) = label {
        if let Some(name) = label.strip_prefix('@') {
            out.push_str(&format!("__m{invocation}_{name}"));
        } else {
            out.push_str(label);
        }
        out.push_str(": ");
    }
    out.push_str(&rename_at_refs(rest, invocation));
    out
}

fn rename_at_refs(rest: &str, invocation: u32) -> String {
    let mut out = String::new();
    let bytes = rest.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'@' {
            let start = i + 1;
            let mut j = start;
            while j < bytes.len() && (bytes[j].is_ascii_alphanumeric() || bytes[j] == b'_') {
                j += 1;
            }
            if j > start {
                out.push_str(&format!("__m{invocation}_{}", &rest[start..j]));
                i = j;
                continue;
            }
        }
        out.push(bytes[i] as char);
        i += 1;
    }
    out
}

fn active_scope(implicit: u32, explicit: &[u32]) -> u32 {
    explicit.last().copied().unwrap_or(implicit)
}

fn rewrite_locals(src: &str) -> Result<String, AsmError> {
    let raw_lines: Vec<&str> = src.lines().collect();
    let mut implicit = 0u32;
    let mut explicit: Vec<u32> = Vec::new();
    let mut next_explicit = 0x1_0000u32;
    let mut line_scope = vec![0u32; raw_lines.len()];
    let mut at_defs: HashMap<(u32, String), String> = HashMap::new();
    let mut num_defs: Vec<(usize, String, String)> = Vec::new();
    let mut num_seen: HashMap<String, u32> = HashMap::new();
    let mut scope_openers: Vec<String> = Vec::new();

    for (index, raw) in raw_lines.iter().enumerate() {
        let line = strip_comment(raw);
        if is_directive(line, ".scope") {
            next_explicit += 1;
            explicit.push(next_explicit);
            scope_openers.push(raw.to_string());
            line_scope[index] = active_scope(implicit, &explicit);
            continue;
        }
        if is_directive(line, ".endscope") {
            scope_openers.pop();
            if explicit.pop().is_none() {
                return Err(here(raw.trim_end_matches('\n'), ".endscope sem .scope"));
            }
            line_scope[index] = active_scope(implicit, &explicit);
            continue;
        }
        if is_directive(line, ".local") {
            let scope = active_scope(implicit, &explicit);
            let list = line[".local".len()..].trim();
            if list.is_empty() {
                return Err(here(raw.trim_end_matches('\n'), ".local exige um nome"));
            }
            for name in list.split(',').map(str::trim) {
                if !is_plain_ident(name) {
                    return Err(here(raw.trim_end_matches('\n'), format!(".local com nome inválido: {name}")));
                }
                let key_name = name.to_ascii_lowercase();
                let unique = format!("__s{scope}_{key_name}");
                if at_defs.insert((scope, key_name), unique).is_some() {
                    return Err(here(raw.trim_end_matches('\n'), format!(".local repetido: {name}")));
                }
            }
            line_scope[index] = scope;
            continue;
        }
        let (label, _) = split_label(line);
        if let Some(label) = label {
            if is_numeric_label(label) {
                let n = num_seen.entry(label.to_string()).or_insert(0);
                *n += 1;
                let unique = format!("__n{label}_{n}");
                num_defs.push((index, label.to_string(), unique));
            } else if let Some(name) = label.strip_prefix('@') {
                let scope = active_scope(implicit, &explicit);
                let key_name = name.to_ascii_lowercase();
                let unique = format!("__s{scope}_{key_name}");
                if at_defs.insert((scope, key_name.clone()), unique).is_some() {
                    return Err(here(raw.trim_end_matches('\n'), format!("label local duplicado: @{name}")));
                }
            } else if is_global_label(label) && explicit.is_empty() {
                implicit += 1;
            }
        }
        line_scope[index] = active_scope(implicit, &explicit);
    }
    if !explicit.is_empty() {
        return Err(here(scope_openers.last().map_or("", |r| r.as_str()), ".scope sem .endscope"));
    }

    // `@` labels are scoped to the global label that precedes them. A global label
    // increments the scope, so an `@` on a later line sees the new scope. Recompute
    // `@` defs with that rule: the loop above incremented scope only when the global
    // label was seen, and `@` on the same line is not possible with one colon.
    // Numeric labels are program-wide (`1b` / `1f`).
    let _ = line_scope;

    let mut out = String::new();
    for (index, raw) in raw_lines.iter().enumerate() {
        let line = strip_comment(raw);
        let tag = raw.find(TAG).map_or("", |i| &raw[i..]);
        if line.is_empty() || is_directive(line, ".scope") || is_directive(line, ".endscope") || is_directive(line, ".local") {
            out.push_str(tag);
                out.push('\n');
            continue;
        }
        let (label, rest) = split_label(line);
        if let Some(label) = label {
            if is_numeric_label(label) {
                let unique = num_defs
                    .iter()
                    .find(|(i, name, _)| *i == index && name == label)
                    .map(|(_, _, unique)| unique.clone())
                    .unwrap();
                out.push_str(&unique);
                out.push(':');
                if !rest.is_empty() {
                    out.push(' ');
                    out.push_str(&resolve_refs(rest, index, line_scope[index], &at_defs, &num_defs)?);
                }
                out.push_str(tag);
                out.push('\n');
                continue;
            }
            if let Some(name) = label.strip_prefix('@') {
                let unique = at_defs
                    .get(&(line_scope[index], name.to_ascii_lowercase()))
                    .cloned()
                    .ok_or_else(|| AsmError(format!("label local indefinido: @{name}")))?;
                out.push_str(&unique);
                out.push(':');
                if !rest.is_empty() {
                    out.push(' ');
                    out.push_str(&resolve_refs(rest, index, line_scope[index], &at_defs, &num_defs)?);
                }
                out.push_str(tag);
                out.push('\n');
                continue;
            }
        }
        if let (Some(label), rest) = (label.filter(|l| {
            let mut c = l.chars();
            c.next().is_some_and(|ch| ch.is_ascii_alphabetic() || ch == '_')
                && c.all(|ch| ch.is_ascii_alphanumeric() || ch == '_')
        }), rest)
        {
            if let Some(unique) = at_defs.get(&(line_scope[index], label.to_ascii_lowercase())) {
                out.push_str(unique);
                out.push(':');
                if !rest.is_empty() {
                    out.push(' ');
                    out.push_str(&resolve_refs(rest, index, line_scope[index], &at_defs, &num_defs)?);
                }
                out.push_str(tag);
                out.push('\n');
                continue;
            }
        }
        let rewritten = resolve_refs(line, index, line_scope[index], &at_defs, &num_defs)?;
        out.push_str(&rewritten);
        out.push_str(tag);
        out.push('\n');
    }
    Ok(out)
}

fn is_numeric_label(label: &str) -> bool {
    !label.is_empty() && label.bytes().all(|b| b.is_ascii_digit())
}

fn is_global_label(label: &str) -> bool {
    // Anonymous labels do not open a new `@local` scope.
    if label.starts_with("__anon_") {
        return false;
    }
    let mut chars = label.chars();
    chars
        .next()
        .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

fn resolve_refs(
    text: &str,
    index: usize,
    scope: u32,
    at_defs: &HashMap<(u32, String), String>,
    num_defs: &[(usize, String, String)],
) -> Result<String, AsmError> {
    let mut out = String::new();
    let bytes = text.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        // Strings and 'c' literals are copied verbatim.
        let verbatim = match bytes[i] {
            b'"' => crate::escape::string_end(&text[i..]).map_or(bytes.len() - i, |e| e + 1),
            b'\'' => crate::escape::char_literal_len(&text[i..]).unwrap_or(0),
            _ => 0,
        };
        if verbatim > 0 {
            out.push_str(&text[i..i + verbatim]);
            i += verbatim;
            continue;
        }
        if bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_' || bytes[i] == b'@' {
            let start = i;
            i += 1;
            while i < bytes.len() && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_') {
                i += 1;
            }
            let tok = &text[start..i];
            let prev = if start == 0 { None } else { Some(bytes[start - 1]) };
            if matches!(prev, Some(b'$' | b'#')) {
                out.push_str(tok);
            } else {
                out.push_str(&resolve_token(tok, index, scope, at_defs, num_defs)?);
            }
        } else {
            let len = text[i..].chars().next().map_or(1, char::len_utf8);
            out.push_str(&text[i..i + len]);
            i += len;
        }
    }
    Ok(out)
}

fn resolve_token(
    tok: &str,
    index: usize,
    scope: u32,
    at_defs: &HashMap<(u32, String), String>,
    num_defs: &[(usize, String, String)],
) -> Result<String, AsmError> {
    if let Some(name) = tok.strip_prefix('@') {
        if !name.is_empty() && is_global_label(name) {
            return at_defs
                .get(&(scope, name.to_ascii_lowercase()))
                .cloned()
                .ok_or_else(|| AsmError(format!("label local indefinido: @{name}")));
        }
    }
    if let Some((num, dir)) = numeric_ref(tok) {
        let unique = match dir {
            'b' => num_defs
                .iter()
                .rev()
                .find(|(i, name, _)| *name == num && *i <= index)
                .map(|(_, _, unique)| unique.clone()),
            'f' => num_defs
                .iter()
                .find(|(i, name, _)| *name == num && *i > index)
                .map(|(_, _, unique)| unique.clone()),
            _ => None,
        };
        return unique.ok_or_else(|| AsmError(format!("label local indefinido: {tok}")));
    }
    if let Some(unique) = at_defs.get(&(scope, tok.to_ascii_lowercase())) {
        if tok.chars().next().is_some_and(|c| c.is_ascii_alphabetic() || c == '_') {
            return Ok(unique.clone());
        }
    }
    Ok(tok.to_string())
}

fn numeric_ref(tok: &str) -> Option<(&str, char)> {
    let bytes = tok.as_bytes();
    if bytes.len() < 2 {
        return None;
    }
    let dir = bytes[bytes.len() - 1] as char;
    if dir != 'b' && dir != 'f' && dir != 'B' && dir != 'F' {
        return None;
    }
    let num = &tok[..tok.len() - 1];
    if num.bytes().all(|b| b.is_ascii_digit()) {
        Some((num, dir.to_ascii_lowercase()))
    } else {
        None
    }
}


// `.rept` / `.endr`, `.if` / `.else` / `.endif`, and `.incbin`.
// Conditions and repeat counts are constants (the same expression language as
// the assembler, with no labels yet). `.incbin` copies bytes from the in-memory
// blob map, or from the UTF-8 bytes of a text file in the include map.

enum CtrlStop {
    None,
    Endr,
    If,
}

enum CtrlNode {
    Line(String),
    Rept(u16, Option<String>, Vec<CtrlNode>),
    If(bool, Vec<CtrlNode>, Vec<CtrlNode>),
    Bytes(Vec<u8>),
    Error(String),
    Warn(String),
}

/// How [`parse_ctrl`] stopped: end of input, or the terminator of its block.
/// `.else`, `.elseif` and `.endif` are left for the caller to consume.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CtrlEnd {
    Eof,
    Endr,
    Else,
    ElseIf,
    Endif,
}

/// Text macro made by `.define`: optional parameters and the replacement text.
#[derive(Debug, Clone)]
struct TextMacro {
    params: Option<Vec<String>>,
    body: String,
}

/// State carried through the conditional pass, in source order.
struct CtrlCtx<'a> {
    files: &'a HashMap<String, String>,
    blobs: &'a HashMap<String, Vec<u8>>,
    /// Names visible to `.ifdef`: labels, constants and `.define`s on earlier
    /// active lines (lowercased for labels/constants, verbatim for `.define`).
    defined: HashSet<String>,
    /// Constant values (`name = expr`) known so far, usable in `.if`/`.elseif`.
    values: HashMap<String, u16>,
    /// Active `.define` text macros (case-sensitive names).
    defines: HashMap<String, TextMacro>,
    /// Identifiers seen in active instruction operands (for `.ifref`).
    referenced: HashSet<String>,
}

/// Deepest nesting of `.define` replacements before the expansion is refused.
const DEFINE_DEPTH: u32 = 16;

fn expand_control(
    src: &str,
    files: &HashMap<String, String>,
    blobs: &HashMap<String, Vec<u8>>,
) -> Result<(String, Vec<String>), AsmError> {
    let lines: Vec<&str> = src.lines().collect();
    let mut ctx = CtrlCtx {
        files,
        blobs,
        defined: HashSet::new(),
        values: HashMap::new(),
        defines: HashMap::new(),
        referenced: HashSet::new(),
    };
    let (nodes, next, end) = parse_ctrl(&lines, 0, CtrlStop::None, true, &mut ctx)?;
    if end != CtrlEnd::Eof || next != lines.len() {
        return Err(AsmError("diretiva de controle fora de lugar".into()));
    }
    render_ctrl(&nodes)
}

/// Lines whose names must not be replaced by `.define` before they are read.
fn names_a_define(line: &str) -> bool {
    [".define", ".undefine", ".undef", ".ifdef", ".ifndef"]
        .iter()
        .any(|d| is_directive(line, d))
}

fn parse_ctrl(
    lines: &[&str],
    mut index: usize,
    stop: CtrlStop,
    active: bool,
    ctx: &mut CtrlCtx<'_>,
) -> Result<(Vec<CtrlNode>, usize, CtrlEnd), AsmError> {
    let mut nodes = Vec::new();
    while index < lines.len() {
        let original = lines[index];
        let tagged = original.trim_end_matches('\n');
        if strip_comment(original).is_empty() {
            index += 1;
            continue;
        }
        let expanded;
        let raw: &str = if active && !names_a_define(strip_comment(original)) {
            expanded = expand_text_line(original, ctx).map_err(|e| here(tagged, e.0))?;
            &expanded
        } else {
            original
        };
        let line = strip_comment(raw);
        if is_directive(line, ".endr")
            || is_directive(line, ".endrep")
            || is_directive(line, ".endrepeat")
        {
            if matches!(stop, CtrlStop::Endr) {
                return Ok((nodes, index + 1, CtrlEnd::Endr));
            }
            return Err(here(tagged, ".endr sem .rept"));
        }
        for (name, end) in [
            (".else", CtrlEnd::Else),
            (".elseif", CtrlEnd::ElseIf),
            (".endif", CtrlEnd::Endif),
        ] {
            if is_directive(line, name) {
                if matches!(stop, CtrlStop::If) {
                    return Ok((nodes, index, end));
                }
                return Err(here(tagged, format!("{name} sem .if")));
            }
        }
        if is_directive(line, ".rept") || is_directive(line, ".repeat") {
            let word = if is_directive(line, ".repeat") { ".repeat" } else { ".rept" };
            let arg = directive_arg(line, word).ok_or_else(|| here(tagged, format!("{word} sem contagem")))?;
            let (count_src, var) = match arg.split_once(',') {
                Some((c, v)) => (c.trim(), Some(v.trim().to_ascii_lowercase())),
                None => (arg, None),
            };
            if let Some(v) = &var {
                if !is_plain_ident(v) {
                    return Err(here(tagged, format!("{word} com contador inválido: {v}")));
                }
            }
            let count = if active {
                parse_expr(count_src, &ctx.values).map_err(|e| here(tagged, e.0))?
            } else {
                0
            };
            if count > 4096 {
                return Err(here(tagged, ".rept maior que 4096"));
            }
            let (body, next, _) = parse_ctrl(lines, index + 1, CtrlStop::Endr, active, ctx)?;
            nodes.push(CtrlNode::Rept(count, var, body));
            index = next;
            continue;
        }
        if let Some(cond) = if_header(line, active, ctx).map_err(|e| here(tagged, e.0))? {
            let (node, next) = parse_if_chain(lines, index + 1, cond, active, ctx)?;
            nodes.push(node);
            index = next;
            continue;
        }
        index += 1;
        if !active {
            continue;
        }
        if is_directive(line, ".define") {
            define_text_macro(line, ctx).map_err(|e| here(tagged, e.0))?;
            continue;
        }
        if is_directive(line, ".undefine") || is_directive(line, ".undef") {
            let name = line.split_whitespace().nth(1).unwrap_or("");
            if ctx.defines.remove(name).is_none() {
                return Err(here(tagged, format!(".undefine de nome não definido: {name}")));
            }
            ctx.defined.remove(name);
            continue;
        }
        if let Some((name, extra)) = incbin_args(line) {
            let bytes = lookup_blob(name, ctx.files, ctx.blobs).map_err(|e| here(tagged, e.0))?;
            let slice = incbin_slice(&bytes, name, extra, &ctx.values).map_err(|e| here(tagged, e.0))?;
            nodes.push(CtrlNode::Bytes(slice));
            continue;
        }
        if let Some(message) = control_message(line, ".error") {
            nodes.push(CtrlNode::Error(message));
            continue;
        }
        if let Some(message) = control_message(line, ".warn") {
            nodes.push(CtrlNode::Warn(message));
            continue;
        }
        note_definition(line, ctx);
        note_references(line, ctx);
        nodes.push(CtrlNode::Line(raw.to_string()));
    }
    match stop {
        CtrlStop::None => Ok((nodes, index, CtrlEnd::Eof)),
        CtrlStop::Endr => Err(AsmError(".rept sem .endr".into())),
        CtrlStop::If => Err(AsmError(".if sem .endif".into())),
    }
}

/// `Some(condition)` for `.if expr`, `.ifdef name` and `.ifndef name`. In an
/// inactive block the condition is not evaluated (it is `false`).
fn if_header(line: &str, active: bool, ctx: &CtrlCtx<'_>) -> Result<Option<bool>, AsmError> {
    if is_directive(line, ".if") {
        let arg = directive_arg(line, ".if").ok_or_else(|| AsmError(".if sem expressão".into()))?;
        return Ok(Some(active && parse_expr(arg, &ctx.values)? != 0));
    }
    if is_directive(line, ".ifconst") {
        let arg = directive_arg(line, ".ifconst").ok_or_else(|| AsmError(".ifconst sem expressão".into()))?;
        // True when every name in the expression is already a known constant.
        let ok = active && expr_is_const(arg, &ctx.values);
        return Ok(Some(ok));
    }
    if is_directive(line, ".ifref") {
        let mut words = line.split_whitespace().skip(1);
        let symbol = words
            .next()
            .ok_or_else(|| AsmError(".ifref sem nome".into()))?;
        if words.next().is_some() || !is_plain_ident(symbol) {
            return Err(AsmError(format!(".ifref exige um único nome: {symbol}")));
        }
        let key = symbol.to_ascii_lowercase();
        return Ok(Some(active && ctx.referenced.contains(&key)));
    }
    for (name, wanted) in [(".ifblank", true), (".ifnblank", false)] {
        if is_directive(line, name) {
            // ca65: true when no tokens follow (typically a macro parameter that
            // was not passed). The text is not evaluated.
            let blank = line.trim()[name.len()..].trim().is_empty();
            return Ok(Some(active && blank == wanted));
        }
    }
    for (name, wanted) in [(".ifdef", true), (".ifndef", false)] {
        if is_directive(line, name) {
            let mut words = line.split_whitespace().skip(1);
            let symbol = words
                .next()
                .ok_or_else(|| AsmError(format!("{name} sem nome")))?;
            if words.next().is_some() || !is_plain_ident(symbol) {
                return Err(AsmError(format!("{name} exige um único nome: {symbol}")));
            }
            let known = ctx.defined.contains(symbol)
                || ctx.defined.contains(&symbol.to_ascii_lowercase());
            return Ok(Some(active && known == wanted));
        }
    }
    Ok(None)
}

/// The body after a `.if`-family header, then any `.elseif` / `.else` arms, up to
/// and including the closing `.endif`. Exactly one arm is rendered.
fn parse_if_chain(
    lines: &[&str],
    start: usize,
    cond: bool,
    active: bool,
    ctx: &mut CtrlCtx<'_>,
) -> Result<(CtrlNode, usize), AsmError> {
    let (then_body, at, end) = parse_ctrl(lines, start, CtrlStop::If, active && cond, ctx)?;
    let rest_active = active && !cond;
    match end {
        CtrlEnd::Endif => Ok((CtrlNode::If(cond, then_body, Vec::new()), at + 1)),
        CtrlEnd::Else => {
            let (else_body, after, end) = parse_ctrl(lines, at + 1, CtrlStop::If, rest_active, ctx)?;
            if end != CtrlEnd::Endif {
                let tagged = lines[after].trim_end_matches('\n');
                return Err(here(tagged, ".else ou .elseif depois de .else"));
            }
            Ok((CtrlNode::If(cond, then_body, else_body), after + 1))
        }
        CtrlEnd::ElseIf => {
            let tagged = lines[at].trim_end_matches('\n');
            let header = if rest_active {
                expand_text_line(lines[at], ctx).map_err(|e| here(tagged, e.0))?
            } else {
                lines[at].to_string()
            };
            let arg = directive_arg(strip_comment(&header), ".elseif")
                .ok_or_else(|| here(tagged, ".elseif sem expressão"))?
                .to_string();
            let next_cond = rest_active
                && parse_expr(&arg, &ctx.values).map_err(|e| here(tagged, e.0))? != 0;
            let (node, after) = parse_if_chain(lines, at + 1, next_cond, rest_active, ctx)?;
            Ok((CtrlNode::If(cond, then_body, vec![node]), after))
        }
        CtrlEnd::Eof | CtrlEnd::Endr => Err(AsmError(".if sem .endif".into())),
    }
}

/// Record what an active line defines, for `.ifdef` and constant `.if`s.
fn note_definition(line: &str, ctx: &mut CtrlCtx<'_>) {
    if let Some((name, expr)) = parse_equ(line) {
        if is_plain_ident(name) {
            let key = name.to_ascii_lowercase();
            if let Ok(value) = parse_expr(&normalize_operand(expr), &ctx.values) {
                ctx.values.insert(key.clone(), value);
            }
            ctx.defined.insert(key);
        }
        return;
    }
    let (label, rest) = split_label(line);
    if let Some(label) = label {
        if is_plain_ident(label) && !rest.starts_with(['-', '+']) {
            ctx.defined.insert(label.to_ascii_lowercase());
        }
    }
}

fn is_plain_ident(s: &str) -> bool {
    let mut chars = s.chars();
    chars
        .next()
        .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// `.define NAME text` or `.define NAME(a, b) text`. Redefining is an error.
fn define_text_macro(line: &str, ctx: &mut CtrlCtx<'_>) -> Result<(), AsmError> {
    let rest = line.trim_start()[".define".len()..].trim_start();
    let name_len = rest
        .find(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
        .unwrap_or(rest.len());
    let name = &rest[..name_len];
    if !is_plain_ident(name) {
        return Err(AsmError(format!(".define com nome inválido: {name}")));
    }
    let after = &rest[name_len..];
    let (params, body) = if let Some(list) = after.strip_prefix('(') {
        let close = list
            .find(')')
            .ok_or_else(|| AsmError(format!(".define {name}: falta ')'")))?;
        let mut params = Vec::new();
        if !list[..close].trim().is_empty() {
            for p in list[..close].split(',') {
                let p = p.trim();
                if !is_plain_ident(p) {
                    return Err(AsmError(format!(".define {name}: parâmetro inválido: {p}")));
                }
                if params.iter().any(|q: &String| q == p) {
                    return Err(AsmError(format!(".define {name}: parâmetro repetido: {p}")));
                }
                params.push(p.to_string());
            }
        }
        (Some(params), list[close + 1..].trim())
    } else {
        if !after.is_empty() && !after.starts_with(char::is_whitespace) {
            return Err(AsmError(format!(".define com nome inválido: {name}{after}")));
        }
        (None, after.trim())
    };
    if ctx.defines.contains_key(name) {
        return Err(AsmError(format!(".define repetido: {name}")));
    }
    ctx.defined.insert(name.to_string());
    ctx.defines.insert(
        name.to_string(),
        TextMacro {
            params,
            body: body.to_string(),
        },
    );
    Ok(())
}

/// Apply `.define` replacements and `.strlen("...")` to the code part of a tagged
/// line; the line tag is kept.
fn expand_text_line(raw: &str, ctx: &CtrlCtx<'_>) -> Result<String, AsmError> {
    let (code, tag) = match raw.find(TAG) {
        Some(i) => raw.split_at(i),
        None => (raw, ""),
    };
    let mut text = if ctx.defines.is_empty() {
        code.to_string()
    } else {
        expand_defines(code, &ctx.defines, 0)?
    };
    if find_ci_unquoted(&text, ".strlen").is_some() {
        text = expand_strlen(&text)?;
    }
    text.push_str(tag);
    Ok(text)
}

/// Byte length of every unquoted `.strlen(<token>)` (after its escapes are decoded)
/// as a decimal number. `<token>` must be one string literal.
fn expand_strlen(text: &str) -> Result<String, AsmError> {
    let mut out = String::new();
    let mut rest = text;
    while let Some(at) = find_ci_unquoted(rest, ".strlen") {
        out.push_str(&rest[..at]);
        let after = rest[at + ".strlen".len()..].trim_start();
        let inner = after
            .strip_prefix('(')
            .ok_or_else(|| AsmError(".strlen exige (\"texto\")".into()))?
            .trim_start();
        let end = if inner.starts_with('"') {
            crate::escape::string_end(inner)
        } else {
            None
        }
        .ok_or_else(|| AsmError(".strlen exige uma string entre aspas".into()))?;
        let bytes = crate::escape::decode_string(&inner[1..end]).map_err(AsmError)?;
        let tail = inner[end + 1..].trim_start();
        let tail = tail
            .strip_prefix(')')
            .ok_or_else(|| AsmError(".strlen sem ')'".into()))?;
        out.push_str(&format!("${:X}", bytes.len()));
        rest = tail;
    }
    out.push_str(rest);
    Ok(out)
}

/// First case-insensitive `needle` outside strings and character literals.
fn find_ci_unquoted(s: &str, needle: &str) -> Option<usize> {
    let mut i = 0;
    while let Some(c) = s[i..].chars().next() {
        match c {
            '"' => i += crate::escape::string_end(&s[i..]).map_or(s.len() - i, |e| e + 1),
            '\'' => i += crate::escape::char_literal_len(&s[i..]).unwrap_or(1),
            _ => {
                if s[i..]
                    .get(..needle.len())
                    .is_some_and(|w| w.eq_ignore_ascii_case(needle))
                {
                    return Some(i);
                }
                i += c.len_utf8();
            }
        }
    }
    None
}

/// Replace `.define` names (whole identifiers outside strings, not after `.`, `$`,
/// `%` or a digit) by their text, re-expanding the result up to [`DEFINE_DEPTH`].
fn expand_defines(
    text: &str,
    defines: &HashMap<String, TextMacro>,
    depth: u32,
) -> Result<String, AsmError> {
    if depth > DEFINE_DEPTH {
        return Err(AsmError(format!(
            ".define aninhado além de {DEFINE_DEPTH} níveis"
        )));
    }
    let mut out = String::with_capacity(text.len());
    let mut i = 0;
    let mut prev: Option<char> = None;
    while let Some(c) = text[i..].chars().next() {
        let quoted = match c {
            '"' => crate::escape::string_end(&text[i..]).map_or(text.len() - i, |e| e + 1),
            '\'' => crate::escape::char_literal_len(&text[i..]).unwrap_or(1),
            _ => 0,
        };
        if quoted > 0 {
            out.push_str(&text[i..i + quoted]);
            i += quoted;
            prev = Some('"');
            continue;
        }
        let starts_word = (c.is_ascii_alphabetic() || c == '_')
            && !prev.is_some_and(|p| p.is_ascii_alphanumeric() || matches!(p, '_' | '.' | '$' | '%' | '@'));
        if !starts_word {
            out.push(c);
            prev = Some(c);
            i += c.len_utf8();
            continue;
        }
        let len = text[i..]
            .find(|ch: char| !(ch.is_ascii_alphanumeric() || ch == '_'))
            .unwrap_or(text.len() - i);
        let word = &text[i..i + len];
        i += len;
        prev = word.chars().last();
        let Some(mac) = defines.get(word) else {
            out.push_str(word);
            continue;
        };
        let replacement = match &mac.params {
            None => mac.body.clone(),
            Some(params) => {
                let after = text[i..].trim_start();
                let args_text = after.strip_prefix('(').ok_or_else(|| {
                    AsmError(format!(".define {word} exige argumentos entre parênteses"))
                })?;
                let (args, used) = split_define_args(args_text)
                    .ok_or_else(|| AsmError(format!(".define {word}: falta ')'")))?;
                i = text.len() - after.len() + 1 + used;
                let args = if args.len() == 1 && args[0].is_empty() {
                    Vec::new()
                } else {
                    args
                };
                if args.len() != params.len() {
                    return Err(AsmError(format!(
                        ".define {word} espera {} argumento(s), recebeu {}",
                        params.len(),
                        args.len()
                    )));
                }
                let arg_map: HashMap<String, TextMacro> = params
                    .iter()
                    .zip(&args)
                    .map(|(p, a)| {
                        (
                            p.clone(),
                            TextMacro {
                                params: None,
                                body: a.clone(),
                            },
                        )
                    })
                    .collect();
                replace_words_once(&mac.body, &arg_map)
            }
        };
        out.push_str(&expand_defines(&replacement, defines, depth + 1)?);
        prev = Some(')');
    }
    Ok(out)
}

/// Single, non-recursive replacement of whole words (used for parameters).
fn replace_words_once(text: &str, map: &HashMap<String, TextMacro>) -> String {
    let mut out = String::new();
    let mut i = 0;
    let mut prev: Option<char> = None;
    while let Some(c) = text[i..].chars().next() {
        let quoted = match c {
            '"' => crate::escape::string_end(&text[i..]).map_or(text.len() - i, |e| e + 1),
            '\'' => crate::escape::char_literal_len(&text[i..]).unwrap_or(1),
            _ => 0,
        };
        if quoted > 0 {
            out.push_str(&text[i..i + quoted]);
            i += quoted;
            prev = Some('"');
            continue;
        }
        if (c.is_ascii_alphabetic() || c == '_')
            && !prev.is_some_and(|p| p.is_ascii_alphanumeric() || matches!(p, '_' | '.' | '$' | '%' | '@'))
        {
            let len = text[i..]
                .find(|ch: char| !(ch.is_ascii_alphanumeric() || ch == '_'))
                .unwrap_or(text.len() - i);
            let word = &text[i..i + len];
            out.push_str(map.get(word).map_or(word, |m| m.body.as_str()));
            prev = word.chars().last();
            i += len;
            continue;
        }
        out.push(c);
        prev = Some(c);
        i += c.len_utf8();
    }
    out
}

/// Arguments up to the `)` matching an already-consumed `(`: commas at depth zero
/// outside quotes split them. Returns the trimmed arguments and the bytes used,
/// including the `)`.
fn split_define_args(s: &str) -> Option<(Vec<String>, usize)> {
    let mut depth = 0u32;
    let mut args = Vec::new();
    let mut start = 0;
    let mut i = 0;
    while let Some(c) = s[i..].chars().next() {
        match c {
            '"' => {
                i += crate::escape::string_end(&s[i..])? + 1;
                continue;
            }
            '\'' => {
                i += crate::escape::char_literal_len(&s[i..]).unwrap_or(1);
                continue;
            }
            '(' => depth += 1,
            ')' if depth == 0 => {
                args.push(s[start..i].trim().to_string());
                return Some((args, i + 1));
            }
            ')' => depth -= 1,
            ',' if depth == 0 => {
                args.push(s[start..i].trim().to_string());
                start = i + 1;
            }
            _ => {}
        }
        i += c.len_utf8();
    }
    None
}

fn render_ctrl(nodes: &[CtrlNode]) -> Result<(String, Vec<String>), AsmError> {
    let mut out = String::new();
    let mut warnings = Vec::new();
    for node in nodes {
        match node {
            CtrlNode::Line(line) => {
                out.push_str(line);
                out.push('\n');
            }
            CtrlNode::Rept(count, var, body) => {
                for i in 0..*count {
                    let (text, more) = render_ctrl(body)?;
                    if let Some(name) = var {
                        // Bind the counter as a constant line before each iteration.
                        out.push_str(&format!("{name} = 0d{i}
"));
                    }
                    out.push_str(&text);
                    warnings.extend(more);
                }
            }
            CtrlNode::If(cond, then_body, else_body) => {
                let (text, more) = render_ctrl(if *cond { then_body } else { else_body })?;
                out.push_str(&text);
                warnings.extend(more);
            }
            CtrlNode::Bytes(bytes) => {
                if bytes.is_empty() {
                    continue;
                }
                for chunk in bytes.chunks(16) {
                    out.push_str(".byte ");
                    for (i, byte) in chunk.iter().enumerate() {
                        if i > 0 {
                            out.push(',');
                        }
                        out.push_str(&format!("${byte:02X}"));
                    }
                    out.push('\n');
                }
            }
            CtrlNode::Error(message) => {
                let message = if message.is_empty() {
                    ".error".to_string()
                } else {
                    message.clone()
                };
                return Err(AsmError(message));
            }
            CtrlNode::Warn(message) => {
                warnings.push(if message.is_empty() {
                    ".warn".to_string()
                } else {
                    message.clone()
                });
            }
        }
    }
    Ok((out, warnings))
}

fn control_message(line: &str, name: &str) -> Option<String> {
    let rest = if line.len() >= name.len() && line[..name.len()].eq_ignore_ascii_case(name) {
        &line[name.len()..]
    } else {
        return None;
    };
    if !rest.is_empty() && !rest.starts_with(|c: char| c.is_whitespace()) {
        return None;
    }
    Some(unquote_message(rest.trim()))
}

fn unquote_message(text: &str) -> String {
    let Some(inner) = text.strip_prefix('"') else {
        return text.to_string();
    };
    let Some(end) = inner.find('"') else {
        return text.to_string();
    };
    inner[..end].to_string()
}

fn is_directive(line: &str, name: &str) -> bool {
    let token = line.split_whitespace().next().unwrap_or("");
    token.eq_ignore_ascii_case(name)
}

fn directive_arg<'a>(line: &'a str, name: &str) -> Option<&'a str> {
    let rest = if line.len() >= name.len() && line[..name.len()].eq_ignore_ascii_case(name) {
        &line[name.len()..]
    } else {
        return None;
    };
    if !rest.is_empty() && !rest.starts_with(|c: char| c.is_whitespace()) {
        return None;
    }
    let rest = rest.trim();
    if rest.is_empty() {
        return None;
    }
    Some(rest)
}

/// `.incbin "name"[, start[, size]]` (ca65 form): the file name and the text
/// after it (empty, or starting with `,`).
fn incbin_args(line: &str) -> Option<(&str, &str)> {
    let rest = directive_arg(line, ".incbin")?;
    if let Some(inner) = rest.strip_prefix('"') {
        let end = inner.find('"')?;
        return Some((&inner[..end], inner[end + 1..].trim()));
    }
    let end = rest.find(',').unwrap_or(rest.len());
    Some((rest[..end].trim(), rest[end..].trim()))
}

/// Bytes `start..start + size` of an `.incbin` blob. `start` defaults to 0 and
/// `size` to the rest of the file; both are constant expressions. A start past
/// the end, a range past the end or extra operands are errors (ca65 rules); a
/// start equal to the length with no size gives no bytes.
fn incbin_slice(
    bytes: &[u8],
    name: &str,
    extra: &str,
    values: &HashMap<String, u16>,
) -> Result<Vec<u8>, AsmError> {
    if extra.is_empty() {
        return Ok(bytes.to_vec());
    }
    let list = extra
        .strip_prefix(',')
        .ok_or_else(|| AsmError(format!(".incbin com texto depois do nome: {extra}")))?;
    let parts: Vec<&str> = list.split(',').map(str::trim).collect();
    if parts.len() > 2 || parts.iter().any(|p| p.is_empty()) {
        return Err(AsmError(".incbin aceita só início e tamanho".into()));
    }
    let start = usize::from(parse_expr(&normalize_operand(parts[0]), values)?);
    if start > bytes.len() {
        return Err(AsmError(format!(
            ".incbin {name}: início {start} passa do tamanho {}",
            bytes.len()
        )));
    }
    let size = match parts.get(1) {
        Some(p) => usize::from(parse_expr(&normalize_operand(p), values)?),
        None => bytes.len() - start,
    };
    if start + size > bytes.len() {
        return Err(AsmError(format!(
            ".incbin {name}: {start}+{size} passa do tamanho {}",
            bytes.len()
        )));
    }
    Ok(bytes[start..start + size].to_vec())
}

fn lookup_blob(
    name: &str,
    files: &HashMap<String, String>,
    blobs: &HashMap<String, Vec<u8>>,
) -> Result<Vec<u8>, AsmError> {
    if name.is_empty() {
        return Err(AsmError(".incbin exige um nome".into()));
    }
    if let Some(bytes) = blobs.get(name) {
        return Ok(bytes.clone());
    }
    if let Some(text) = files.get(name) {
        return Ok(text.as_bytes().to_vec());
    }
    Err(AsmError(format!(
        ".incbin não encontrado no mapa em memória: {name}"
    )))
}

// `::` scope access, `.struct`/`.enum`, `.sizeof` and anonymous labels.
//
// `A::b` (outside strings) is spelled `A__b` internally, so struct fields and enum
// members are ordinary constants: `Point::x` and `Point__x` are the same symbol.

/// Rewrite every unquoted `::` to `__`.
fn rewrite_scope_access(src: &str) -> String {
    let mut out = String::with_capacity(src.len());
    let mut i = 0;
    while let Some(c) = src[i..].chars().next() {
        let quoted = match c {
            '"' => crate::escape::string_end(&src[i..]).map_or(src.len() - i, |e| e + 1),
            '\'' => crate::escape::char_literal_len(&src[i..]).unwrap_or(1),
            _ => 0,
        };
        if quoted > 0 {
            out.push_str(&src[i..i + quoted]);
            i += quoted;
        } else if src[i..].starts_with("::") {
            out.push_str("__");
            i += 2;
        } else {
            out.push(c);
            i += c.len_utf8();
        }
    }
    out
}

/// One `.struct`: its total size and every field (nested `.tag` fields included,
/// as `outer__inner`) with offset and size.
#[derive(Debug, Clone, Default)]
struct StructDef {
    size: u16,
    fields: Vec<(String, u16, u16)>,
}

/// Expand `.struct`/`.endstruct` and `.enum`/`.endenum` into constants, then
/// replace `.sizeof(Name)` and `.sizeof(Name__field)` everywhere.
///
/// Struct fields: `[name] .byte|.word|.addr|.faraddr|.dword|.res|.tag T [count]`.
/// `.byte`/`.word`/`.addr`/`.faraddr`/`.dword` take an optional element count
/// (default 1); `.res n` reserves `n` bytes; `.tag T` embeds struct `T` (count
/// optional). An optional `.struct Name, base` starts offsets at `base`.
fn expand_structs(src: &str) -> Result<String, AsmError> {
    let mut structs: HashMap<String, StructDef> = HashMap::new();
    let mut out = String::with_capacity(src.len());
    let mut lines = src.lines();
    while let Some(raw) = lines.next() {
        let line = strip_comment(raw);
        let tagged = raw.trim_end_matches('\n');
        let tag = raw.find(TAG).map_or("", |i| &raw[i..]);
        let opens = if is_directive(line, ".struct") {
            Some(false)
        } else if is_directive(line, ".union") {
            Some(true)
        } else {
            None
        };
        if let Some(union) = opens {
            let word = if union { ".union" } else { ".struct" };
            let header = line[word.len()..].trim();
            let (name, base) = match header.split_once(',') {
                Some((n, b)) => (
                    n.trim(),
                    parse_expr(&normalize_operand(b), &HashMap::new()).map_err(|e| here(tagged, e.0))?,
                ),
                None => (header, 0),
            };
            if !is_plain_ident(name) {
                return Err(here(tagged, format!("{word} com nome inválido: {name}")));
            }
            let key = name.to_ascii_lowercase();
            if structs.contains_key(&key) {
                return Err(here(tagged, format!("{word} repetida: {name}")));
            }
            let def = aggregate(&mut lines, union, base, &structs, tagged, 1)?;
            for (field, at, _) in &def.fields {
                out.push_str(&format!("{key}__{field} = ${at:X}{tag}\n"));
            }
            structs.insert(key, def);
            continue;
        }
        if is_directive(line, ".endstruct") {
            return Err(here(tagged, ".endstruct sem .struct"));
        }
        if is_directive(line, ".endunion") {
            return Err(here(tagged, ".endunion sem .union"));
        }
        if is_directive(line, ".enum") {
            let name = line[".enum".len()..].trim();
            if !name.is_empty() && !is_plain_ident(name) {
                return Err(here(tagged, format!(".enum com nome inválido: {name}")));
            }
            let prefix = if name.is_empty() {
                String::new()
            } else {
                format!("{}__", name.to_ascii_lowercase())
            };
            let mut values: HashMap<String, u16> = HashMap::new();
            let mut next: u32 = 0;
            let mut closed = false;
            for raw in lines.by_ref() {
                let line = strip_comment(raw);
                let tagged = raw.trim_end_matches('\n');
                let tag = raw.find(TAG).map_or("", |i| &raw[i..]);
                if line.is_empty() {
                    continue;
                }
                if is_directive(line, ".endenum") {
                    closed = true;
                    break;
                }
                let (member, value) = match line.split_once('=') {
                    Some((m, e)) => (
                        m.trim(),
                        u32::from(
                            parse_expr(&normalize_operand(e), &values).map_err(|e| here(tagged, e.0))?,
                        ),
                    ),
                    None => (line, next),
                };
                if !is_plain_ident(member) {
                    return Err(here(tagged, format!("membro de .enum inválido: {member}")));
                }
                let value = u16::try_from(value)
                    .map_err(|_| here(tagged, format!("membro de .enum passa de $FFFF: {member}")))?;
                let mkey = member.to_ascii_lowercase();
                if values.insert(mkey.clone(), value).is_some() {
                    return Err(here(tagged, format!("membro repetido: {member}")));
                }
                out.push_str(&format!("{prefix}{mkey} = ${value:X}{tag}\n"));
                next = u32::from(value) + 1;
            }
            if !closed {
                return Err(here(tagged, ".enum sem .endenum"));
            }
            continue;
        }
        if is_directive(line, ".endenum") {
            return Err(here(tagged, ".endenum sem .enum"));
        }
        out.push_str(raw);
        out.push('\n');
    }
    if find_ci_unquoted(&out, ".sizeof").is_none() {
        return Ok(out);
    }
    out.lines()
        .map(|raw| {
            if find_ci_unquoted(raw, ".sizeof").is_none() {
                return Ok(format!("{raw}\n"));
            }
            expand_sizeof(raw, &structs)
                .map(|l| format!("{l}\n"))
                .map_err(|e| here(raw, e.0))
        })
        .collect()
}

/// Deepest `.struct`/`.union` nesting accepted inside one definition.
const MAX_AGGREGATE_DEPTH: usize = 16;

/// Body of one `.struct` (`union == false`) or `.union` up to its matching end.
///
/// Field offsets are absolute (they start at `base`). In a struct each member
/// follows the previous one and the size is the sum; in a union every member starts
/// at `base` and the size is the largest member (ca65 rules). Inside a body:
/// - an anonymous `.struct`/`.union` ... `.endstruct`/`.endunion` adds its fields to
///   the enclosing aggregate at the current offset;
/// - `name .struct`/`name .union` ... end is a named member whose fields are also
///   reachable as `name__field`, like `.tag`.
///
/// Nested bodies are parsed with base 0 and shifted by the caller.
fn aggregate(
    lines: &mut std::str::Lines<'_>,
    union: bool,
    base: u16,
    structs: &HashMap<String, StructDef>,
    opener: &str,
    depth: usize,
) -> Result<StructDef, AsmError> {
    let (word, end) = if union {
        (".union", ".endunion")
    } else {
        (".struct", ".endstruct")
    };
    if depth > MAX_AGGREGATE_DEPTH {
        return Err(here(opener, format!("{word} aninhada além de {MAX_AGGREGATE_DEPTH} níveis")));
    }
    let mut def = StructDef::default();
    let mut offset = u32::from(base);
    let mut largest = 0u32;
    let push = |def: &mut StructDef, name: String, at: u32, size: u16, tagged: &str| {
        if def.fields.iter().any(|(f, _, _)| *f == name) {
            return Err(here(tagged, format!("campo repetido: {name}")));
        }
        let at = u16::try_from(at).map_err(|_| here(tagged, format!("{word} maior que 64 KiB")))?;
        def.fields.push((name, at, size));
        Ok(())
    };
    while let Some(raw) = lines.next() {
        let line = strip_comment(raw);
        let tagged = raw.trim_end_matches('\n');
        if line.is_empty() {
            continue;
        }
        if is_directive(line, end) {
            let size = if union { largest } else { offset - u32::from(base) };
            def.size = u16::try_from(size).map_err(|_| here(tagged, format!("{word} maior que 64 KiB")))?;
            return Ok(def);
        }
        let other_end = if union { ".endstruct" } else { ".endunion" };
        if is_directive(line, other_end) {
            return Err(here(tagged, format!("{other_end} fecha um {word}")));
        }
        // Nested aggregate, anonymous or named.
        let (label, rest) = match line.split_once(char::is_whitespace) {
            Some((first, rest)) if !first.starts_with('.') => (Some(first), rest.trim()),
            _ => (None, line),
        };
        let inner = if is_directive(rest, ".struct") {
            Some(false)
        } else if is_directive(rest, ".union") {
            Some(true)
        } else {
            None
        };
        let at = if union { u32::from(base) } else { offset };
        let size = if let Some(inner_union) = inner {
            let inner_word = if inner_union { ".union" } else { ".struct" };
            if !rest[inner_word.len()..].trim().is_empty() {
                return Err(here(tagged, format!("{inner_word} aninhada não leva nome nem base")));
            }
            let nested = aggregate(lines, inner_union, 0, structs, tagged, depth + 1)?;
            match label {
                None => {
                    for (f, off, sz) in nested.fields {
                        push(&mut def, f, at + u32::from(off), sz, tagged)?;
                    }
                }
                Some(name) => {
                    if !is_plain_ident(name) {
                        return Err(here(tagged, format!("campo inválido: {name}")));
                    }
                    let key = name.to_ascii_lowercase();
                    push(&mut def, key.clone(), at, nested.size, tagged)?;
                    for (f, off, sz) in nested.fields {
                        push(&mut def, format!("{key}__{f}"), at + u32::from(off), sz, tagged)?;
                    }
                }
            }
            nested.size
        } else {
            let (field, size, nested) = struct_field(line, structs).map_err(|e| here(tagged, e.0))?;
            if let Some(field) = field {
                let fkey = field.to_ascii_lowercase();
                push(&mut def, fkey.clone(), at, size, tagged)?;
                for (inner, off, sz) in nested {
                    push(&mut def, format!("{fkey}__{inner}"), at + u32::from(off), sz, tagged)?;
                }
            }
            size
        };
        if union {
            largest = largest.max(u32::from(size));
            if u32::from(base) + largest > 0x10000 {
                return Err(here(tagged, format!("{word} maior que 64 KiB")));
            }
        } else {
            offset += u32::from(size);
            if offset > 0x10000 {
                return Err(here(tagged, format!("{word} maior que 64 KiB")));
            }
        }
    }
    Err(here(opener, format!("{word} sem {end}")))
}

/// Name, size and nested fields of one struct member line.
#[allow(clippy::type_complexity)]
fn struct_field(
    line: &str,
    structs: &HashMap<String, StructDef>,
) -> Result<(Option<String>, u16, Vec<(String, u16, u16)>), AsmError> {
    let mut words = line.splitn(2, char::is_whitespace);
    let first = words.next().unwrap_or("");
    let (name, rest) = if first.starts_with('.') {
        (None, line)
    } else {
        if !is_plain_ident(first) {
            return Err(AsmError(format!("campo inválido: {first}")));
        }
        let rest = words.next().unwrap_or("").trim();
        if rest.is_empty() {
            return Err(AsmError(format!("campo sem tipo: {first}")));
        }
        (Some(first.to_string()), rest)
    };
    let mut parts = rest.splitn(2, char::is_whitespace);
    let kind = parts.next().unwrap_or("").to_ascii_lowercase();
    let arg = parts.next().unwrap_or("").trim();
    let count = |text: &str| -> Result<u32, AsmError> {
        if text.is_empty() {
            Ok(1)
        } else {
            parse_expr(&normalize_operand(text), &HashMap::new()).map(u32::from)
        }
    };
    let (unit, nested, times) = match kind.as_str() {
        ".byte" => (1u32, Vec::new(), count(arg)?),
        ".word" | ".addr" => (2, Vec::new(), count(arg)?),
        ".faraddr" => (3, Vec::new(), count(arg)?),
        ".dword" => (4, Vec::new(), count(arg)?),
        ".res" => {
            if arg.is_empty() {
                return Err(AsmError(".res exige um tamanho".into()));
            }
            (1, Vec::new(), count(arg)?)
        }
        ".tag" => {
            let mut a = arg.splitn(2, char::is_whitespace);
            let tname = a.next().unwrap_or("").to_ascii_lowercase();
            let def = structs
                .get(&tname)
                .ok_or_else(|| AsmError(format!(".tag de struct desconhecida: {tname}")))?;
            let times = count(a.next().unwrap_or("").trim())?;
            let nested = if times == 1 { def.fields.clone() } else { Vec::new() };
            (u32::from(def.size), nested, times)
        }
        _ => return Err(AsmError(format!("tipo de campo desconhecido: {kind}"))),
    };
    let size = u16::try_from(unit * times).map_err(|_| AsmError("campo maior que 64 KiB".into()))?;
    Ok((name, size, nested))
}

fn expand_sizeof(raw: &str, structs: &HashMap<String, StructDef>) -> Result<String, AsmError> {
    let (code, tag) = match raw.find(TAG) {
        Some(i) => raw.split_at(i),
        None => (raw, ""),
    };
    let mut out = String::new();
    let mut rest = code;
    while let Some(at) = find_ci_unquoted(rest, ".sizeof") {
        out.push_str(&rest[..at]);
        let after = rest[at + ".sizeof".len()..].trim_start();
        let inner = after
            .strip_prefix('(')
            .ok_or_else(|| AsmError(".sizeof exige (nome)".into()))?;
        let close = inner
            .find(')')
            .ok_or_else(|| AsmError(".sizeof sem ')'".into()))?;
        let name = inner[..close].trim().to_ascii_lowercase();
        let size = if let Some(def) = structs.get(&name) {
            def.size
        } else {
            structs
                .iter()
                .find_map(|(s, def)| {
                    let field = name.strip_prefix(s.as_str())?.strip_prefix("__")?;
                    def.fields
                        .iter()
                        .find(|(f, _, _)| f == field)
                        .map(|(_, _, size)| *size)
                })
                .ok_or_else(|| AsmError(format!(".sizeof de nome desconhecido: {name}")))?
        };
        out.push_str(&format!("${size:X}"));
        rest = &inner[close + 1..];
    }
    out.push_str(rest);
    out.push_str(tag);
    Ok(out)
}

/// Anonymous labels: a line whose label is empty (`: inx`, or `:` alone) defines
/// one; `:-` is the nearest at or before the current line, `:--` the one before
/// that, `:+` the nearest after it, `:++` the second after, and so on.
fn expand_anonymous(src: &str) -> Result<String, AsmError> {
    let lines: Vec<&str> = src.lines().collect();
    let defs: Vec<usize> = lines
        .iter()
        .enumerate()
        .filter(|(_, raw)| is_anon_def(strip_comment(raw)))
        .map(|(i, _)| i)
        .collect();
    if defs.is_empty() && !lines.iter().any(|l| anon_ref_at(l).is_some()) {
        return Ok(src.to_string());
    }
    let name = |n: usize| format!("__anon_{n}");
    let mut out = String::with_capacity(src.len());
    for (index, raw) in lines.iter().enumerate() {
        let mut line = raw.to_string();
        if let Ok(k) = defs.binary_search(&index) {
            let at = line.find(':').expect("anonymous label has ':'");
            line.replace_range(at..at + 1, &format!("{}:", name(k)));
        }
        let mut done = String::new();
        let mut rest = line.as_str();
        while let Some((at, len, back, count)) = anon_ref_at(rest) {
            let target = if back {
                let before = defs.partition_point(|&d| d <= index);
                before.checked_sub(count)
            } else {
                let after = defs.partition_point(|&d| d <= index);
                Some(after + count - 1).filter(|&k| k < defs.len())
            };
            let k = target.ok_or_else(|| {
                here(raw, format!("rótulo anônimo inexistente: {}", &rest[at..at + len]))
            })?;
            done.push_str(&rest[..at]);
            done.push_str(&name(k));
            rest = &rest[at + len..];
        }
        done.push_str(rest);
        out.push_str(&done);
        out.push('\n');
    }
    Ok(out)
}

fn is_anon_def(line: &str) -> bool {
    line.strip_prefix(':')
        .is_some_and(|rest| rest.is_empty() || rest.starts_with(char::is_whitespace))
}

/// Next unquoted `:-…` / `:+…` reference in the code part of a line:
/// (byte index, length, backwards, count).
fn anon_ref_at(s: &str) -> Option<(usize, usize, bool, usize)> {
    let code = s.split(TAG).next().unwrap_or("");
    let code = match find_unquoted(code, ';') {
        Some(i) => &code[..i],
        None => code,
    };
    let mut from = 0;
    while let Some(i) = find_unquoted(&code[from..], ':') {
        let at = from + i;
        let tail = &code[at + 1..];
        let sign = tail.chars().next();
        if let Some(sign @ ('-' | '+')) = sign {
            let count = tail.chars().take_while(|&c| c == sign).count();
            let next = tail[count..].chars().next();
            if !next.is_some_and(|c| c.is_ascii_alphanumeric() || c == '_' || c == '$') {
                return Some((at, count + 1, sign == '-', count));
            }
        }
        from = at + 1;
    }
    None
}


// `.proc name` / `.endproc` (ca65): `name` is a label at the current address and
// opens a named scope. Labels and `=` constants defined inside are spelled
// `name__label` internally, so the rest of the program reaches them as
// `name::label`; inside the procedure (and in procedures nested in it) the short
// name works, the innermost definition winning. `.export a, b` inside a procedure
// also defines the short names `a`, `b` globally (aliases of the scoped symbols),
// after the procedure. At top level `.export` is accepted and changes nothing,
// because every top-level symbol is already global. Each procedure body is also an
// explicit `.scope`, so `@` locals do not leak in or out.

enum ProcItem {
    Line(String),
    Proc {
        name: String,
        opener: String,
        body: Vec<ProcItem>,
    },
}

/// Deepest `.proc` nesting accepted.
const MAX_PROC_DEPTH: usize = 16;

fn expand_procs(src: &str) -> Result<String, AsmError> {
    let has_proc = src.lines().any(|raw| {
        let line = strip_comment(raw);
        is_directive(line, ".proc") || is_directive(line, ".endproc") || is_directive(line, ".export")
    });
    if !has_proc {
        return Ok(src.to_string());
    }
    let lines: Vec<&str> = src.lines().collect();
    let mut index = 0;
    let items = parse_procs(&lines, &mut index, None, 0)?;
    let mut out = String::with_capacity(src.len());
    emit_procs(&items, "", &HashMap::new(), &mut out)?;
    Ok(out)
}

fn parse_procs(
    lines: &[&str],
    index: &mut usize,
    opener: Option<&str>,
    depth: usize,
) -> Result<Vec<ProcItem>, AsmError> {
    let mut items = Vec::new();
    while *index < lines.len() {
        let raw = lines[*index];
        *index += 1;
        let line = strip_comment(raw);
        if is_directive(line, ".proc") {
            let tagged = raw.trim_end_matches('\n');
            let mut words = line.split_whitespace().skip(1);
            let name = words
                .next()
                .ok_or_else(|| here(tagged, ".proc exige um nome"))?;
            if words.next().is_some() || !is_plain_ident(name) {
                return Err(here(tagged, format!(".proc com nome inválido: {name}")));
            }
            if depth + 1 > MAX_PROC_DEPTH {
                return Err(here(tagged, format!(".proc aninhada além de {MAX_PROC_DEPTH} níveis")));
            }
            let body = parse_procs(lines, index, Some(tagged), depth + 1)?;
            items.push(ProcItem::Proc {
                name: name.to_ascii_lowercase(),
                opener: tagged.to_string(),
                body,
            });
            continue;
        }
        if is_directive(line, ".endproc") {
            if opener.is_none() {
                return Err(here(raw.trim_end_matches('\n'), ".endproc sem .proc"));
            }
            return Ok(items);
        }
        items.push(ProcItem::Line(raw.to_string()));
    }
    match opener {
        Some(o) => Err(here(o, ".proc sem .endproc")),
        None => Ok(items),
    }
}

/// Names defined directly in `items` (not inside nested procedures, except the
/// nested procedure names themselves), lowercase.
fn proc_definitions(items: &[ProcItem]) -> Result<Vec<String>, AsmError> {
    let mut names: Vec<String> = Vec::new();
    let mut add = |name: &str, raw: &str| -> Result<(), AsmError> {
        let key = name.to_ascii_lowercase();
        if names.contains(&key) {
            return Err(here(raw, format!("nome repetido no escopo: {name}")));
        }
        names.push(key);
        Ok(())
    };
    for item in items {
        match item {
            ProcItem::Line(raw) => {
                let line = strip_comment(raw);
                let (label, rest) = split_label(line);
                if let Some(label) = label.filter(|l| is_plain_ident(l)) {
                    add(label, raw)?;
                }
                if let Some((name, _)) = parse_equ(rest) {
                    if is_plain_ident(name) {
                        add(name, raw)?;
                    }
                }
            }
            ProcItem::Proc { name, opener, .. } => add(name, opener)?,
        }
    }
    Ok(names)
}

fn emit_procs(
    items: &[ProcItem],
    prefix: &str,
    visible: &HashMap<String, String>,
    out: &mut String,
) -> Result<(), AsmError> {
    let mut map = visible.clone();
    for name in proc_definitions(items)? {
        let qualified = format!("{prefix}{name}");
        map.insert(name, qualified);
    }
    let mut exports: Vec<(String, String)> = Vec::new();
    for item in items {
        match item {
            ProcItem::Line(raw) => {
                let line = strip_comment(raw);
                let tag = raw.find(TAG).map_or("", |i| &raw[i..]);
                if is_directive(line, ".export") {
                    let list = line[".export".len()..].trim();
                    if list.is_empty() {
                        return Err(here(raw, ".export exige um nome"));
                    }
                    for name in list.split(',').map(str::trim) {
                        if !is_plain_ident(name) {
                            return Err(here(raw, format!(".export com nome inválido: {name}")));
                        }
                        let key = name.to_ascii_lowercase();
                        let Some(q) = map.get(&key).filter(|q| q.starts_with(prefix)) else {
                            return Err(here(raw, format!(".export de nome não definido aqui: {name}")));
                        };
                        if !prefix.is_empty() {
                            exports.push((key, format!("{q}{tag}")));
                        }
                    }
                    out.push_str(tag);
                    out.push('\n');
                    continue;
                }
                if prefix.is_empty() {
                    out.push_str(raw);
                } else {
                    out.push_str(&rename_idents(line, &map));
                    out.push_str(tag);
                }
                out.push('\n');
            }
            ProcItem::Proc { name, opener, body } => {
                let qualified = map[name].clone();
                let tag = opener.find(TAG).map_or("", |i| &opener[i..]);
                out.push_str(&format!("{qualified}:{tag}\n.scope{tag}\n"));
                emit_procs(body, &format!("{qualified}__"), &map, out)?;
                out.push_str(&format!(".endscope{tag}\n"));
            }
        }
    }
    for (short, target) in exports {
        out.push_str(&format!("{short} = {target}\n"));
    }
    Ok(())
}

/// Replace every identifier of `text` found in `map` (outside strings and
/// character literals, not after `$`, `%`, `.` or `@`). `a__b` with `a` in the
/// map becomes `map[a]__b`, so `inner::x` works from the enclosing procedure.
fn rename_idents(text: &str, map: &HashMap<String, String>) -> String {
    let mut out = String::with_capacity(text.len());
    let bytes = text.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        let verbatim = match bytes[i] {
            b'"' => crate::escape::string_end(&text[i..]).map_or(bytes.len() - i, |e| e + 1),
            b'\'' => crate::escape::char_literal_len(&text[i..]).unwrap_or(0),
            _ => 0,
        };
        if verbatim > 0 {
            out.push_str(&text[i..i + verbatim]);
            i += verbatim;
            continue;
        }
        if bytes[i].is_ascii_alphabetic() || bytes[i] == b'_' {
            let start = i;
            while i < bytes.len() && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_') {
                i += 1;
            }
            let tok = &text[start..i];
            let prev = start.checked_sub(1).map(|p| bytes[p]);
            if matches!(prev, Some(b'$' | b'%' | b'.' | b'@')) || (prev.is_some_and(|p| p.is_ascii_digit())) {
                out.push_str(tok);
                continue;
            }
            let key = tok.to_ascii_lowercase();
            if let Some(q) = map.get(&key) {
                out.push_str(q);
            } else if let Some((head, tail)) = key.split_once("__").filter(|(h, _)| !h.is_empty()) {
                match map.get(head) {
                    Some(q) => {
                        out.push_str(q);
                        out.push_str("__");
                        out.push_str(&tok[tok.len() - tail.len()..]);
                    }
                    None => out.push_str(tok),
                }
            } else {
                out.push_str(tok);
            }
            continue;
        }
        let len = text[i..].chars().next().map_or(1, char::len_utf8);
        out.push_str(&text[i..i + len]);
        i += len;
    }
    out
}


/// Built-in macro packs loaded by `.macpack name` (ca65-compatible names).
fn expand_macpacks(src: &str) -> Result<String, AsmError> {
    let mut out = String::with_capacity(src.len());
    for raw in src.lines() {
        let line = strip_comment(raw);
        if is_directive(line, ".macpack") {
            let name = line[".macpack".len()..].trim().trim_matches('"').to_ascii_lowercase();
            let pack = macpack_source(&name).ok_or_else(|| {
                here(raw.trim_end_matches('\n'), format!(".macpack desconhecido: {name}"))
            })?;
            let tag = raw.find(TAG).map_or("", |i| &raw[i..]);
            for pline in pack.lines() {
                out.push_str(pline);
                out.push_str(tag);
                out.push('\n');
            }
            continue;
        }
        out.push_str(raw);
        out.push('\n');
    }
    Ok(out)
}

fn macpack_source(name: &str) -> Option<&'static str> {
    Some(match name {
        "longbranch" => MACPACK_LONGBRANCH,
        "generic" => MACPACK_GENERIC,
        _ => return None,
    })
}

/// Long branches: `jcc`/`jcs`/`jeq`/`jne`/`jmi`/`jpl`/`jvc`/`jvs`/`jbra`.
const MACPACK_LONGBRANCH: &str = r#".macro jcc dest
 bcc @ok
 jmp \dest
@ok:
.endmacro
.macro jcs dest
 bcs @ok
 jmp \dest
@ok:
.endmacro
.macro jeq dest
 beq @ok
 jmp \dest
@ok:
.endmacro
.macro jne dest
 bne @ok
 jmp \dest
@ok:
.endmacro
.macro jmi dest
 bmi @ok
 jmp \dest
@ok:
.endmacro
.macro jpl dest
 bpl @ok
 jmp \dest
@ok:
.endmacro
.macro jvc dest
 bvc @ok
 jmp \dest
@ok:
.endmacro
.macro jvs dest
 bvs @ok
 jmp \dest
@ok:
.endmacro
.macro jbra dest
 jmp \dest
.endmacro
"#;

/// 16-bit add/sub on a zero-page pointer: `add16 ptr, value` / `sub16 ptr, value`.
const MACPACK_GENERIC: &str = r#".macro add16 ptr, value
 clc
 lda \ptr
 adc #<\value
 sta \ptr
 lda \ptr+1
 adc #>\value
 sta \ptr+1
.endmacro
.macro sub16 ptr, value
 sec
 lda \ptr
 sbc #<\value
 sta \ptr
 lda \ptr+1
 sbc #>\value
 sta \ptr+1
.endmacro
"#;



fn expr_is_const(expr: &str, values: &HashMap<String, u16>) -> bool {
    parse_expr(expr, values).is_ok()
}

fn note_references(line: &str, ctx: &mut CtrlCtx<'_>) {
    let (_, rest) = split_label(strip_comment(line));
    if rest.is_empty() || rest.starts_with('.') {
        return;
    }
    let (_, arg) = split_word(rest);
    for tok in arg.split(|c: char| !c.is_ascii_alphanumeric() && c != '_') {
        if tok.is_empty() { continue; }
        let mut chars = tok.chars();
        if chars.next().is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
            && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
        {
            ctx.referenced.insert(tok.to_ascii_lowercase());
        }
    }
}


fn expand_string_funcs(src: &str) -> Result<String, AsmError> {
    let mut out = String::with_capacity(src.len());
    for raw in src.lines() {
        let tag = raw.find(TAG).map_or("", |i| &raw[i..]);
        let body = match raw.find(TAG) { Some(i) => &raw[..i], None => raw };
        if ![".concat", ".left", ".right", ".mid", ".sprintf"].iter().any(|n| find_ci_unquoted(body, n).is_some()) {
            out.push_str(raw); out.push('\n'); continue;
        }
        let replaced = replace_string_funcs(body).map_err(|e| here(raw.trim_end_matches('\n'), e.0))?;
        out.push_str(&replaced); out.push_str(tag); out.push('\n');
    }
    Ok(out)
}

fn replace_string_funcs(text: &str) -> Result<String, AsmError> {
    let mut rest = text.to_string();
    for _ in 0..32 {
        let Some((start, name)) = [".concat", ".left", ".right", ".mid", ".sprintf"]
            .into_iter()
            .filter_map(|n| find_ci_unquoted(&rest, n).map(|i| (i, n)))
            .min_by_key(|(i, _)| *i)
        else { return Ok(rest); };
        let open_rel = rest[start + name.len()..].find('(')
            .ok_or_else(|| AsmError(format!("{name} exige ( ... )")))?;
        let open_at = start + name.len() + open_rel;
        let inner = &rest[open_at + 1..];
        let close = matching_paren(inner).ok_or_else(|| AsmError(format!("{name} sem ')'")))?;
        let args = split_func_args(&inner[..close])?;
        let value = eval_string_func(name, &args)?;
        let end = open_at + 1 + close + 1;
        rest = format!("{}{}{}", &rest[..start], quote_asm_string(&value), &rest[end..]);
    }
    Err(AsmError("funções de string aninhadas demais".into()))
}

fn matching_paren(s: &str) -> Option<usize> {
    let mut depth = 0i32;
    let mut i = 0;
    while i < s.len() {
        let c = s[i..].chars().next()?;
        match c {
            '(' => depth += 1,
            ')' => { if depth == 0 { return Some(i); } depth -= 1; }
            '"' => { let end = crate::escape::string_end(&s[i..])?; i += end + 1; continue; }
            '\'' => { i += crate::escape::char_literal_len(&s[i..]).unwrap_or(1); continue; }
            _ => {}
        }
        i += c.len_utf8();
    }
    None
}

fn split_func_args(s: &str) -> Result<Vec<String>, AsmError> {
    if s.trim().is_empty() { return Ok(Vec::new()); }
    let mut args = Vec::new();
    let mut start = 0usize;
    let mut depth = 0i32;
    let mut i = 0usize;
    while i < s.len() {
        let c = s[i..].chars().next().unwrap();
        match c {
            '(' => depth += 1,
            ')' => depth -= 1,
            '"' => {
                let end = crate::escape::string_end(&s[i..]).ok_or_else(|| AsmError("string sem aspas".into()))?;
                i += end + 1; continue;
            }
            '\'' => { i += crate::escape::char_literal_len(&s[i..]).unwrap_or(1); continue; }
            ',' if depth == 0 => { args.push(s[start..i].trim().to_string()); start = i + 1; }
            _ => {}
        }
        i += c.len_utf8();
    }
    args.push(s[start..].trim().to_string());
    Ok(args)
}

fn eval_string_func(name: &str, args: &[String]) -> Result<String, AsmError> {
    let as_str = |a: &str| -> Result<String, AsmError> {
        let a = a.trim();
        let end = crate::escape::string_end(a).ok_or_else(|| AsmError(format!("{name}: argumento não é string: {a}")))?;
        if !(a.starts_with('"') && end == a.len() - 1) {
            return Err(AsmError(format!("{name}: argumento não é string: {a}")));
        }
        let bytes = crate::escape::decode_string(&a[1..end]).map_err(AsmError)?;
        String::from_utf8(bytes).map_err(|_| AsmError(format!("{name}: string não é UTF-8")))
    };
    let as_num = |a: &str| crate::expr::eval(a, &|_| None).map_err(|e| AsmError(e.0));
    match name {
        ".concat" => {
            if args.is_empty() { return Err(AsmError(".concat exige argumentos".into())); }
            let mut out = String::new();
            for a in args { out.push_str(&as_str(a)?); }
            Ok(out)
        }
        ".left" => {
            if args.len() != 2 { return Err(AsmError(".left exige string, n".into())); }
            let s = as_str(&args[0])?;
            Ok(s.chars().take(usize::from(as_num(&args[1])?)).collect())
        }
        ".right" => {
            if args.len() != 2 { return Err(AsmError(".right exige string, n".into())); }
            let s = as_str(&args[0])?;
            let n = usize::from(as_num(&args[1])?);
            let chars: Vec<char> = s.chars().collect();
            Ok(chars[chars.len().saturating_sub(n)..].iter().collect())
        }
        ".mid" => {
            if args.len() != 3 { return Err(AsmError(".mid exige string, início, n".into())); }
            let s = as_str(&args[0])?;
            Ok(s.chars().skip(usize::from(as_num(&args[1])?)).take(usize::from(as_num(&args[2])?)).collect())
        }
        ".sprintf" => {
            if args.is_empty() { return Err(AsmError(".sprintf exige formato".into())); }
            let fmt = as_str(&args[0])?;
            let mut out = String::new();
            let mut ai = 1usize;
            let mut chars = fmt.chars().peekable();
            while let Some(c) = chars.next() {
                if c != '%' { out.push(c); continue; }
                match chars.next() {
                    Some('%') => out.push('%'),
                    Some('s') => { out.push_str(&as_str(args.get(ai).ok_or_else(|| AsmError(".sprintf: falta %s".into()))?)?); ai += 1; }
                    Some('d') => { out.push_str(&format!("{}", as_num(args.get(ai).ok_or_else(|| AsmError(".sprintf: falta %d".into()))?)?)); ai += 1; }
                    Some('x') => { out.push_str(&format!("{:x}", as_num(args.get(ai).ok_or_else(|| AsmError(".sprintf: falta %x".into()))?)?)); ai += 1; }
                    Some('X') => { out.push_str(&format!("{:X}", as_num(args.get(ai).ok_or_else(|| AsmError(".sprintf: falta %X".into()))?)?)); ai += 1; }
                    Some('0') => {
                        let mut width = 0usize;
                        while let Some(d) = chars.peek().and_then(|c| c.to_digit(10)) {
                            width = width * 10 + d as usize; chars.next();
                        }
                        match chars.next() {
                            Some('X') => { out.push_str(&format!("{:0width$X}", as_num(args.get(ai).ok_or_else(|| AsmError(".sprintf: falta %0nX".into()))?)?, width=width)); ai += 1; }
                            Some('x') => { out.push_str(&format!("{:0width$x}", as_num(args.get(ai).ok_or_else(|| AsmError(".sprintf: falta %0nx".into()))?)?, width=width)); ai += 1; }
                            other => return Err(AsmError(format!(".sprintf: formato %0n{other:?} não suportado"))),
                        }
                    }
                    other => return Err(AsmError(format!(".sprintf: %{other:?} não suportado"))),
                }
            }
            if ai != args.len() { return Err(AsmError(".sprintf: argumentos sobrando".into())); }
            Ok(out)
        }
        _ => Err(AsmError(format!("função desconhecida: {name}"))),
    }
}

fn quote_asm_string(s: &str) -> String {
    let mut out = String::from("\"");
    for b in s.bytes() {
        match b {
            b'"' => out.push_str("\\\""),
            b'\\' => out.push_str("\\\\"),
            b'\n' => out.push_str("\\n"),
            b'\r' => out.push_str("\\r"),
            b'\t' => out.push_str("\\t"),
            b'\0' => out.push_str("\\0"),
            0x20..=0x7e => out.push(b as char),
            _ => out.push_str(&format!("\\x{b:02X}")),
        }
    }
    out.push('"');
    out
}
