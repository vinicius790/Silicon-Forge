//! Interpretador host-side didático para um subconjunto numérico do BASIC.
//!
//! Não inclui firmware Microsoft/OSI, não roda instruções 6502 e não modela uma ACIA.

use std::collections::BTreeMap;
use std::io::{self, Write};

/// Interpretador didático de um subconjunto numérico do BASIC.
///
/// Este interpretador roda no host e não contém nem executa uma ROM Microsoft BASIC.
pub struct BasicSystem {
    program: BTreeMap<u16, String>,
    variables: BTreeMap<String, f64>,
}

/// Statements one `RUN` may execute before it stops with an error.
pub const STEP_LIMIT: u32 = 100_000;

enum Flow {
    Next,
    End,
    Goto(u16),
    For(String, f64, f64),
    NextVar(Option<String>),
}

struct ForFrame {
    var: String,
    limit: f64,
    step: f64,
    body: usize,
}

impl BasicSystem {
    pub fn new() -> Self {
        Self {
            program: BTreeMap::new(),
            variables: BTreeMap::new(),
        }
    }

    /// Executa uma linha de entrada e retorna as linhas produzidas pelo interpretador.
    pub fn execute_line(&mut self, input: &str) -> Result<Vec<String>, String> {
        let input = input.trim();
        if input.is_empty() {
            return Ok(Vec::new());
        }

        let first_token = input.split_whitespace().next().unwrap_or_default();
        if let Ok(line_number) = first_token.parse::<u16>() {
            if line_number == 0 {
                return Err("Números de linha devem ser maiores que zero.".into());
            }
            let statement = input[first_token.len()..].trim();
            if statement.is_empty() {
                self.program.remove(&line_number);
            } else {
                self.program.insert(line_number, statement.to_string());
            }
            return Ok(Vec::new());
        }

        let command = input.to_ascii_uppercase();
        match command.as_str() {
            "LIST" => Ok(self
                .program
                .iter()
                .map(|(number, statement)| format!("{number} {statement}"))
                .collect()),
            "NEW" => {
                self.program.clear();
                self.variables.clear();
                Ok(Vec::new())
            }
            "RUN" => self.run_program(),
            "HELP" => Ok(vec![
                "Comandos: LIST, NEW, RUN, PRINT, LET, REM, END, GOTO, IF..THEN, FOR..TO..STEP, NEXT".into(),
                "Digite cada linha separadamente: 10 LET A=2+3*4".into(),
                "Depois: 20 PRINT A".into(),
                "Por fim: RUN".into(),
            ]),
            _ => {
                let mut output = Vec::new();
                self.execute_statement(input, &mut output)?;
                Ok(output)
            }
        }
    }

    /// Runs in line order; `GOTO`, `IF ... THEN` and `FOR`/`NEXT` jump. Stops at `END`,
    /// the last line, or after [`STEP_LIMIT`] statements (an error, so loops end).
    fn run_program(&mut self) -> Result<Vec<String>, String> {
        let lines: Vec<(u16, String)> = self.program.iter().map(|(n, s)| (*n, s.clone())).collect();
        let index_of = |target: u16| {
            lines
                .binary_search_by_key(&target, |(n, _)| *n)
                .map_err(|_| format!("Linha {target} não existe."))
        };
        let mut output = Vec::new();
        self.variables.clear();
        let mut loops: Vec<ForFrame> = Vec::new();
        let mut pc = 0usize;
        let mut steps = 0u32;
        while pc < lines.len() {
            steps += 1;
            if steps > STEP_LIMIT {
                return Err(format!("Limite de {STEP_LIMIT} comandos atingido."));
            }
            let (number, statement) = &lines[pc];
            let at = |e: String| format!("Linha {number}: {e}");
            match self.execute_flow(statement, &mut output).map_err(at)? {
                Flow::Next => pc += 1,
                Flow::End => break,
                Flow::Goto(target) => pc = index_of(target).map_err(at)?,
                Flow::For(var, limit, step) => {
                    loops.retain(|f| f.var != var);
                    loops.push(ForFrame {
                        var,
                        limit,
                        step,
                        body: pc + 1,
                    });
                    pc += 1;
                }
                Flow::NextVar(name) => {
                    let frame = match &name {
                        Some(n) => loops.iter().rposition(|f| &f.var == n),
                        None => loops.len().checked_sub(1),
                    }
                    .ok_or_else(|| at("NEXT sem FOR.".into()))?;
                    loops.truncate(frame + 1);
                    let f = &loops[frame];
                    let value = self.variables[&f.var] + f.step;
                    self.variables.insert(f.var.clone(), value);
                    let done = if f.step >= 0.0 {
                        value > f.limit
                    } else {
                        value < f.limit
                    };
                    if done {
                        loops.pop();
                        pc += 1;
                    } else {
                        pc = f.body;
                    }
                }
            }
        }
        Ok(output)
    }

    fn execute_flow(&mut self, statement: &str, output: &mut Vec<String>) -> Result<Flow, String> {
        let upper = statement.trim().to_ascii_uppercase();
        let line_number = |text: &str| {
            text.trim()
                .parse::<u16>()
                .map_err(|_| format!("Número de linha inválido: {}", text.trim()))
        };
        if let Some(target) = upper.strip_prefix("GOTO ") {
            return line_number(target).map(Flow::Goto);
        }
        if let Some(rest) = upper.strip_prefix("IF ") {
            let then = rest.find(" THEN ").ok_or("IF sem THEN.")?;
            let original = &statement.trim()[3..];
            let cond = ExpressionParser::new(original[..then].trim(), &self.variables).parse()?;
            let action = original[then + 6..].trim();
            if cond == 0.0 {
                return Ok(Flow::Next);
            }
            if action.bytes().all(|b| b.is_ascii_digit()) {
                return line_number(action).map(Flow::Goto);
            }
            return self.execute_flow(action, output);
        }
        if let Some(rest) = upper.strip_prefix("FOR ") {
            let original = &statement.trim()[4..];
            let (var, range) = original.split_once('=').ok_or("FOR sem =.")?;
            let var = var.trim().to_ascii_uppercase();
            if !valid_variable_name(&var) {
                return Err(format!("Nome de variável inválido: {var}"));
            }
            let upper_range = range.to_ascii_uppercase();
            let to = upper_range.find(" TO ").ok_or("FOR sem TO.")?;
            let (limit_text, step_text) = match upper_range[to + 4..].find(" STEP ") {
                Some(s) => (&range[to + 4..to + 4 + s], Some(&range[to + 4 + s + 6..])),
                None => (&range[to + 4..], None),
            };
            let eval = |t: &str| ExpressionParser::new(t.trim(), &self.variables).parse();
            let start = eval(&range[..to])?;
            let limit = eval(limit_text)?;
            let step = step_text.map(eval).transpose()?.unwrap_or(1.0);
            if step == 0.0 {
                return Err("STEP 0 não termina.".into());
            }
            let _ = rest;
            self.variables.insert(var.clone(), start);
            return Ok(Flow::For(var, limit, step));
        }
        if upper == "NEXT" {
            return Ok(Flow::NextVar(None));
        }
        if let Some(var) = upper.strip_prefix("NEXT ") {
            return Ok(Flow::NextVar(Some(var.trim().to_string())));
        }
        Ok(if self.execute_statement(statement, output)? {
            Flow::Next
        } else {
            Flow::End
        })
    }

    fn execute_statement(
        &mut self,
        statement: &str,
        output: &mut Vec<String>,
    ) -> Result<bool, String> {
        let statement = statement.trim();
        let command = statement.to_ascii_uppercase();
        if command == "END" {
            return Ok(false);
        }
        if command == "REM" || command.starts_with("REM ") {
            return Ok(true);
        }

        let print_expression = if command.starts_with("PRINT ") {
            Some(statement[6..].trim())
        } else {
            statement.strip_prefix('?').map(str::trim)
        };
        if let Some(expression) = print_expression {
            if expression.len() >= 2 && expression.starts_with('"') && expression.ends_with('"') {
                output.push(expression[1..expression.len() - 1].to_string());
            } else {
                let value = ExpressionParser::new(expression, &self.variables).parse()?;
                output.push(format_number(value));
            }
            return Ok(true);
        }

        let assignment = if command.starts_with("LET ") {
            statement[4..].trim()
        } else {
            statement
        };
        if let Some((name, expression)) = assignment.split_once('=') {
            let name = name.trim().to_ascii_uppercase();
            if !valid_variable_name(&name) {
                return Err(format!("Nome de variável inválido: {name}"));
            }
            let value = ExpressionParser::new(expression.trim(), &self.variables).parse()?;
            self.variables.insert(name, value);
            return Ok(true);
        }

        Err(format!("Comando BASIC não suportado: {statement}"))
    }

    /// Executa o interpretador didático no terminal.
    pub fn run_interactive(&mut self) -> io::Result<()> {
        println!("\x1b[1;32mMOS 6502 Mini BASIC (interpretador didático)\x1b[0m");
        println!("Não inclui a ROM original do Microsoft BASIC. Digite HELP para comandos; EXIT para sair.\n");

        let stdin = io::stdin();
        let mut line = String::new();

        loop {
            print!("\x1b[1;33mREADY\x1b[0m\n> ");
            let _ = io::stdout().flush();
            line.clear();
            if stdin.read_line(&mut line)? == 0 {
                break;
            }
            let trimmed = line.trim();
            if trimmed.eq_ignore_ascii_case("exit") || trimmed.eq_ignore_ascii_case("quit") {
                break;
            }

            match self.execute_line(trimmed) {
                Ok(lines) => {
                    for line in lines {
                        println!("{line}");
                    }
                }
                Err(error) => println!("? {error}"),
            }
        }
        Ok(())
    }
}

impl Default for BasicSystem {
    fn default() -> Self {
        Self::new()
    }
}

fn valid_variable_name(name: &str) -> bool {
    (1..=2).contains(&name.len())
        && name.as_bytes()[0].is_ascii_alphabetic()
        && name.as_bytes().iter().all(u8::is_ascii_alphanumeric)
}

fn format_number(value: f64) -> String {
    if value.is_finite()
        && value.fract() == 0.0
        && value >= i64::MIN as f64
        && value <= i64::MAX as f64
    {
        format!("{}", value as i64)
    } else {
        value.to_string()
    }
}

struct ExpressionParser<'a> {
    bytes: &'a [u8],
    position: usize,
    variables: &'a BTreeMap<String, f64>,
}

impl<'a> ExpressionParser<'a> {
    fn new(expression: &'a str, variables: &'a BTreeMap<String, f64>) -> Self {
        Self {
            bytes: expression.as_bytes(),
            position: 0,
            variables,
        }
    }

    fn parse(mut self) -> Result<f64, String> {
        let value = self.parse_compare()?;
        self.skip_spaces();
        if self.position != self.bytes.len() || !value.is_finite() {
            return Err("Expressão numérica inválida.".into());
        }
        Ok(value)
    }

    /// `a <op> b` with `= <> < > <= >=` gives 1 (true) or 0; at most one per expression.
    fn parse_compare(&mut self) -> Result<f64, String> {
        let left = self.parse_sum()?;
        self.skip_spaces();
        let rest = &self.bytes[self.position..];
        let op: &[u8] = [&b"<="[..], b">=", b"<>", b"=", b"<", b">"]
            .into_iter()
            .find(|op| rest.starts_with(op))
            .unwrap_or(b"");
        if op.is_empty() {
            return Ok(left);
        }
        self.position += op.len();
        let right = self.parse_sum()?;
        let truth = match op {
            b"=" => left == right,
            b"<>" => left != right,
            b"<" => left < right,
            b">" => left > right,
            b"<=" => left <= right,
            _ => left >= right,
        };
        Ok(f64::from(u8::from(truth)))
    }

    fn parse_sum(&mut self) -> Result<f64, String> {
        let mut value = self.parse_product()?;
        loop {
            self.skip_spaces();
            match self.current() {
                Some(b'+') => {
                    self.position += 1;
                    value += self.parse_product()?;
                }
                Some(b'-') => {
                    self.position += 1;
                    value -= self.parse_product()?;
                }
                _ => return Ok(value),
            }
        }
    }

    fn parse_product(&mut self) -> Result<f64, String> {
        let mut value = self.parse_unary()?;
        loop {
            self.skip_spaces();
            match self.current() {
                Some(b'*') => {
                    self.position += 1;
                    value *= self.parse_unary()?;
                }
                Some(b'/') => {
                    self.position += 1;
                    let divisor = self.parse_unary()?;
                    if divisor == 0.0 {
                        return Err("Divisão por zero.".into());
                    }
                    value /= divisor;
                }
                _ => return Ok(value),
            }
        }
    }

    fn parse_unary(&mut self) -> Result<f64, String> {
        self.skip_spaces();
        match self.current() {
            Some(b'+') => {
                self.position += 1;
                self.parse_unary()
            }
            Some(b'-') => {
                self.position += 1;
                Ok(-self.parse_unary()?)
            }
            _ => self.parse_primary(),
        }
    }

    fn parse_primary(&mut self) -> Result<f64, String> {
        self.skip_spaces();
        match self.current() {
            Some(b'(') => {
                self.position += 1;
                let value = self.parse_sum()?;
                self.skip_spaces();
                if self.current() != Some(b')') {
                    return Err("Parêntese não fechado.".into());
                }
                self.position += 1;
                Ok(value)
            }
            Some(byte) if byte.is_ascii_digit() || byte == b'.' => self.parse_number(),
            Some(byte) if byte.is_ascii_alphabetic() => self.parse_variable(),
            _ => Err("Esperado número, variável ou parêntese.".into()),
        }
    }

    fn parse_number(&mut self) -> Result<f64, String> {
        let start = self.position;
        while self
            .current()
            .is_some_and(|byte| byte.is_ascii_digit() || byte == b'.')
        {
            self.position += 1;
        }
        std::str::from_utf8(&self.bytes[start..self.position])
            .ok()
            .and_then(|text| text.parse().ok())
            .filter(|value: &f64| value.is_finite())
            .ok_or_else(|| "Número inválido.".into())
    }

    fn parse_variable(&mut self) -> Result<f64, String> {
        let start = self.position;
        while self
            .current()
            .is_some_and(|byte| byte.is_ascii_alphanumeric())
        {
            self.position += 1;
        }
        let name = std::str::from_utf8(&self.bytes[start..self.position])
            .unwrap_or_default()
            .to_ascii_uppercase();
        if !valid_variable_name(&name) {
            return Err(format!("Nome de variável inválido: {name}"));
        }
        Ok(self.variables.get(&name).copied().unwrap_or(0.0))
    }

    fn current(&self) -> Option<u8> {
        self.bytes.get(self.position).copied()
    }

    fn skip_spaces(&mut self) {
        while self
            .current()
            .is_some_and(|byte| byte.is_ascii_whitespace())
        {
            self.position += 1;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::BasicSystem;

    #[test]
    fn numbered_program_lists_and_runs_with_operator_precedence() {
        let mut basic = BasicSystem::new();
        basic.execute_line("20 print A").unwrap();
        basic.execute_line("10 LET A=2+3*4").unwrap();

        assert_eq!(
            basic.execute_line("LIST").unwrap(),
            vec![String::from("10 LET A=2+3*4"), String::from("20 print A")]
        );
        assert_eq!(basic.execute_line("run").unwrap(), vec![String::from("14")]);
    }

    #[test]
    fn supports_immediate_print_and_reports_expression_errors() {
        let mut basic = BasicSystem::new();
        assert_eq!(
            basic.execute_line("? (8-2)/3").unwrap(),
            vec![String::from("2")]
        );
        assert_eq!(
            basic.execute_line("PRINT 1/0").unwrap_err(),
            "Divisão por zero."
        );
    }

    #[test]
    fn deleting_lines_and_new_clear_the_program() {
        let mut basic = BasicSystem::new();
        basic.execute_line("10 PRINT \"HI\"").unwrap();
        basic.execute_line("10").unwrap();
        assert!(basic.execute_line("LIST").unwrap().is_empty());

        basic.execute_line("10 PRINT \"HI\"").unwrap();
        basic.execute_line("NEW").unwrap();
        assert!(basic.execute_line("RUN").unwrap().is_empty());
    }

    #[test]
    fn help_describes_commands_and_separate_input_lines() {
        let mut basic = BasicSystem::new();
        let help = basic.execute_line("HELP").unwrap().join("\n");

        assert!(help.contains("10 LET A=2+3*4"));
        assert!(help.contains("20 PRINT A"));
        assert!(help.contains("Por fim: RUN"));
    }
}
