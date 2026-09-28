(function (root) {
  "use strict";

  const IMPLIED = {
    CLC: 0x18,
    SEC: 0x38,
    DEX: 0xCA,
    INX: 0xE8,
    BRK: 0x00,
    NOP: 0xEA,
  };
  const IMMEDIATE = {
    LDA: 0xA9,
    LDX: 0xA2,
    ADC: 0x69,
    CPX: 0xE0,
  };
  const ZERO_PAGE = {
    LDA: 0xA5,
    STA: 0x85,
    STX: 0x86,
    ADC: 0x65,
  };
  const BRANCH = {
    BNE: 0xD0,
    BEQ: 0xF0,
  };

  function parseValue(text, labels, allowUnresolved = false) {
    const normalized = text.trim().toUpperCase();
    if (Object.hasOwn(labels, normalized)) return labels[normalized];
    if (allowUnresolved && /^[A-Z_][A-Z0-9_]*$/.test(normalized)) return 0;

    let digits = normalized;
    let radix = 16;
    if (digits.startsWith("$")) digits = digits.slice(1);
    else if (digits.startsWith("0X")) digits = digits.slice(2);
    else if (digits.startsWith("%")) {
      digits = digits.slice(1);
      radix = 2;
    } else if (digits.startsWith("0D")) {
      digits = digits.slice(2);
      radix = 10;
    }
    if (!digits || !/^[0-9A-F]+$/.test(digits) || (radix === 2 && /[^01]/.test(digits))) {
      throw new Error(`invalid number or label: ${text}`);
    }
    const value = Number.parseInt(digits, radix);
    if (!Number.isInteger(value) || value > 0xFFFF) throw new Error(`value out of range: ${text}`);
    return value;
  }

  function stripComment(line) {
    let quoted = false;
    for (let index = 0; index < line.length; index += 1) {
      if (line[index] === '"') quoted = !quoted;
      else if (line[index] === ";" && !quoted) return line.slice(0, index).trim();
    }
    return line.trim();
  }

  function parseSourceLine(raw, lineNumber) {
    let text = stripComment(raw);
    if (!text) return null;

    let label = null;
    const labelMatch = text.match(/^([A-Za-z_][A-Za-z0-9_]*):(?:\s*(.*))?$/);
    if (labelMatch) {
      label = labelMatch[1].toUpperCase();
      text = (labelMatch[2] || "").trim();
    }
    if (!text) return { label, lineNumber };

    const match = text.match(/^(\S+)(?:\s+(.*))?$/);
    return {
      label,
      lineNumber,
      mnemonic: match[1].toUpperCase(),
      operand: (match[2] || "").trim(),
    };
  }

  function selectInstruction(instruction, labels, firstPass) {
    const { mnemonic, operand } = instruction;
    if (!operand && Object.hasOwn(IMPLIED, mnemonic)) {
      return { opcode: IMPLIED[mnemonic], size: 1, kind: "implied" };
    }
    if (operand.startsWith("#") && Object.hasOwn(IMMEDIATE, mnemonic)) {
      return { opcode: IMMEDIATE[mnemonic], size: 2, kind: "immediate" };
    }
    if (Object.hasOwn(BRANCH, mnemonic)) {
      return { opcode: BRANCH[mnemonic], size: 2, kind: "branch" };
    }
    if (Object.hasOwn(ZERO_PAGE, mnemonic)) {
      const value = parseValue(operand, labels, firstPass);
      if (value > 0xFF) throw new Error(`zero-page operand out of range: ${operand}`);
      return { opcode: ZERO_PAGE[mnemonic], size: 2, kind: "zero-page" };
    }
    throw new Error(`unsupported instruction or addressing mode: ${mnemonic} ${operand}`.trim());
  }

  function assemble6502(source, defaultOrigin = 0x8000) {
    if (!Number.isInteger(defaultOrigin) || defaultOrigin < 0 || defaultOrigin > 0xFFFF) {
      throw new Error("origin must be a 16-bit address");
    }
    const parsed = source.split(/\r?\n/).map(parseSourceLine).filter(Boolean);
    const labels = Object.create(null);
    const statements = [];
    let origin = defaultOrigin;
    let address = origin;
    let originSeen = false;

    for (const statement of parsed) {
      if (statement.label) {
        if (Object.hasOwn(labels, statement.label)) {
          throw new Error(`line ${statement.lineNumber}: duplicate label ${statement.label}`);
        }
        labels[statement.label] = address;
      }
      if (!statement.mnemonic) continue;
      if (statement.mnemonic === ".ORG") {
        if (originSeen || statements.length) {
          throw new Error(`line ${statement.lineNumber}: only one leading .org is supported`);
        }
        origin = parseValue(statement.operand, labels);
        address = origin;
        originSeen = true;
        continue;
      }
      const instruction = selectInstruction(statement, labels, true);
      statement.address = address;
      statement.encoding = instruction;
      statements.push(statement);
      address += instruction.size;
      if (address > 0x10000) throw new Error(`line ${statement.lineNumber}: program exceeds 64 KiB`);
    }

    const bytes = [];
    for (const statement of statements) {
      const { opcode, kind } = statement.encoding;
      bytes.push(opcode);
      if (kind === "implied") continue;
      if (kind === "immediate" || kind === "zero-page") {
        const value = parseValue(kind === "immediate" ? statement.operand.slice(1) : statement.operand, labels);
        if (value > 0xFF) throw new Error(`line ${statement.lineNumber}: byte operand out of range`);
        bytes.push(value);
      } else {
        const target = parseValue(statement.operand, labels);
        const displacement = target - (statement.address + 2);
        if (displacement < -128 || displacement > 127) {
          throw new Error(`line ${statement.lineNumber}: branch target is out of range`);
        }
        bytes.push(displacement & 0xFF);
      }
    }
    return { origin, bytes, labels };
  }

  root.assemble6502 = assemble6502;
  if (typeof module !== "undefined" && module.exports) module.exports = { assemble6502 };
})(globalThis);
