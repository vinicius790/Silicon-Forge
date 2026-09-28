/**
 * MOS 6502 / 65C02 Virtual Machine & In-Browser Retro IDE
 */

// Examples stay within the instruction set implemented by the browser prototype.
const EXAMPLES = {
  fibonacci: `; Gera oito termos e deixa os dois últimos em $00/$01
.org $8000
  LDA #$00
  STA $00
  LDA #$01
  STA $01
  LDX #$08
loop:
  LDA $00
    CLC
  ADC $01
  STA $02
  LDA $01
  STA $00
  LDA $02
  STA $01
  DEX
  BNE loop
  BRK
`,

  sum_loop: `; Soma 5+4+3+2+1; resultado 15 em $00
.org $8000
  LDX #$05
  LDA #$00
  CLC
loop:
    STX $01
  ADC $01
    DEX
  BNE loop
  STA $00
    BRK
`,
};

// Estrutura da CPU Virtual no Browser
class Virtual6502 {
  constructor() {
    this.a = 0;
    this.x = 0;
    this.y = 0;
    this.sp = 0xFD;
    this.pc = 0x8000;
    this.entryPoint = 0x8000;
    this.p = 0x24; // NV-BDIZC (I=1, U=1)
    this.cycles = 0;
    this.ram = new Uint8Array(65536);
    this.halted = false;
    this.fault = null;
  }

  reset() {
    this.a = 0;
    this.x = 0;
    this.y = 0;
    this.sp = 0xFD;
    this.p = 0x24;
    this.pc = this.entryPoint;
    this.cycles = 7;
    this.halted = false;
    this.fault = null;
  }

  load(addr, bytes) {
    if (!Number.isInteger(addr) || addr < 0 || addr > 0xFFFF) {
      throw new RangeError("Program origin must be a 16-bit address");
    }
    if (bytes.length > 0x10000 - addr) {
      throw new RangeError("Program image exceeds the 64 KiB address space");
    }
    this.ram.fill(0);
    for (let i = 0; i < bytes.length; i++) {
      this.ram[addr + i] = bytes[i];
    }
    this.entryPoint = addr;
  }

  setFlag(bit, val) {
    if (val) this.p |= bit;
    else this.p &= ~bit;
  }

  getFlag(bit) {
    return (this.p & bit) !== 0;
  }

  setZN(v) {
    this.setFlag(0x02, (v & 0xFF) === 0);   // Zero
    this.setFlag(0x80, (v & 0x80) !== 0);  // Negative
  }

  step() {
    if (this.halted) return 0;
    const op = this.ram[this.pc];
    this.pc = (this.pc + 1) & 0xFFFF;

    switch (op) {
      case 0xA9: { // LDA #imm
        this.a = this.ram[this.pc];
        this.pc = (this.pc + 1) & 0xFFFF;
        this.setZN(this.a);
        this.cycles += 2;
        break;
      }
      case 0xA2: { // LDX #imm
        this.x = this.ram[this.pc];
        this.pc = (this.pc + 1) & 0xFFFF;
        this.setZN(this.x);
        this.cycles += 2;
        break;
      }
      case 0xA0: { // LDY #imm
        this.y = this.ram[this.pc];
        this.pc = (this.pc + 1) & 0xFFFF;
        this.setZN(this.y);
        this.cycles += 2;
        break;
      }
      case 0x85: { // STA zp
        const a = this.ram[this.pc];
        this.pc = (this.pc + 1) & 0xFFFF;
        this.ram[a] = this.a;
        this.cycles += 3;
        break;
      }
      case 0xA5: { // LDA zp
        const a = this.ram[this.pc];
        this.pc = (this.pc + 1) & 0xFFFF;
        this.a = this.ram[a];
        this.setZN(this.a);
        this.cycles += 3;
        break;
      }
      case 0x86: { // STX zp
        const a = this.ram[this.pc];
        this.pc = (this.pc + 1) & 0xFFFF;
        this.ram[a] = this.x;
        this.cycles += 3;
        break;
      }
      case 0x84: { // STY zp
        const a = this.ram[this.pc];
        this.pc = (this.pc + 1) & 0xFFFF;
        this.ram[a] = this.y;
        this.cycles += 3;
        break;
      }
      case 0x8D: { // STA abs
        const lo = this.ram[this.pc];
        const hi = this.ram[(this.pc + 1) & 0xFFFF];
        this.pc = (this.pc + 2) & 0xFFFF;
        const addr = (hi << 8) | lo;
        this.ram[addr] = this.a;
        this.cycles += 4;
        break;
      }
      case 0xE8: { // INX
        this.x = (this.x + 1) & 0xFF;
        this.setZN(this.x);
        this.cycles += 2;
        break;
      }
      case 0xCA: { // DEX
        this.x = (this.x - 1) & 0xFF;
        this.setZN(this.x);
        this.cycles += 2;
        break;
      }
      case 0xC8: { // INY
        this.y = (this.y + 1) & 0xFF;
        this.setZN(this.y);
        this.cycles += 2;
        break;
      }
      case 0x88: { // DEY
        this.y = (this.y - 1) & 0xFF;
        this.setZN(this.y);
        this.cycles += 2;
        break;
      }
      case 0x18: { // CLC
        this.setFlag(0x01, false);
        this.cycles += 2;
        break;
      }
      case 0x38: { // SEC
        this.setFlag(0x01, true);
        this.cycles += 2;
        break;
      }
      case 0x65: { // ADC zp
        const a = this.ram[this.pc];
        this.pc = (this.pc + 1) & 0xFFFF;
        const v = this.ram[a];
        const sum = this.a + v + (this.getFlag(0x01) ? 1 : 0);
        this.setFlag(0x01, sum > 0xFF);
        this.setFlag(0x40, (!((this.a ^ v) & 0x80) && ((this.a ^ sum) & 0x80)) !== 0);
        this.a = sum & 0xFF;
        this.setZN(this.a);
        this.cycles += 3;
        break;
      }
      case 0x69: { // ADC #imm
        const value = this.ram[this.pc];
        this.pc = (this.pc + 1) & 0xFFFF;
        const sum = this.a + value + (this.getFlag(0x01) ? 1 : 0);
        this.setFlag(0x01, sum > 0xFF);
        this.setFlag(0x40, ((~(this.a ^ value) & (this.a ^ sum)) & 0x80) !== 0);
        this.a = sum & 0xFF;
        this.setZN(this.a);
        this.cycles += 2;
        break;
      }
      case 0xAA: { // TAX
        this.x = this.a;
        this.setZN(this.x);
        this.cycles += 2;
        break;
      }
      case 0xA8: { // TAY
        this.y = this.a;
        this.setZN(this.y);
        this.cycles += 2;
        break;
      }
      case 0x8A: { // TXA
        this.a = this.x;
        this.setZN(this.a);
        this.cycles += 2;
        break;
      }
      case 0x98: { // TYA
        this.a = this.y;
        this.setZN(this.a);
        this.cycles += 2;
        break;
      }
      case 0xE0: { // CPX #imm
        const v = this.ram[this.pc];
        this.pc = (this.pc + 1) & 0xFFFF;
        this.setFlag(0x01, this.x >= v);
        this.setZN((this.x - v) & 0xFF);
        this.cycles += 2;
        break;
      }
      case 0xC0: { // CPY #imm
        const v = this.ram[this.pc];
        this.pc = (this.pc + 1) & 0xFFFF;
        this.setFlag(0x01, this.y >= v);
        this.setZN((this.y - v) & 0xFF);
        this.cycles += 2;
        break;
      }
      case 0xD0: { // BNE rel
        const off = (this.ram[this.pc] << 24) >> 24; // Signed i8
        this.pc = (this.pc + 1) & 0xFFFF;
        if (!this.getFlag(0x02)) {
          const target = (this.pc + off) & 0xFFFF;
          const crossedPage = (this.pc & 0xFF00) !== (target & 0xFF00);
          this.pc = target;
          this.cycles += crossedPage ? 4 : 3;
        } else {
          this.cycles += 2;
        }
        break;
      }
      case 0xF0: { // BEQ rel
        const off = (this.ram[this.pc] << 24) >> 24;
        this.pc = (this.pc + 1) & 0xFFFF;
        if (this.getFlag(0x02)) {
          const target = (this.pc + off) & 0xFFFF;
          const crossedPage = (this.pc & 0xFF00) !== (target & 0xFF00);
          this.pc = target;
          this.cycles += crossedPage ? 4 : 3;
        } else {
          this.cycles += 2;
        }
        break;
      }
      case 0x00: { // BRK
        this.halted = true;
        this.cycles += 7;
        break;
      }
      case 0xEA: { // NOP
        this.cycles += 2;
        break;
      }
      default: {
        const opcode = op.toString(16).toUpperCase().padStart(2, "0");
        const address = ((this.pc - 1) & 0xFFFF).toString(16).toUpperCase().padStart(4, "0");
        this.fault = "Unsupported opcode $" + opcode + " at $" + address;
        this.halted = true;
        break;
      }
    }
  }
}

// Inicialização da interface
const cpu = new Virtual6502();
const editor = document.getElementById("asm-editor");
const exampleSelect = document.getElementById("example-select");

function loadExample(key) {
  if (EXAMPLES[key]) {
    editor.value = EXAMPLES[key];
    assembleAndLoad();
  }
}

function assembleAndLoad() {
  let result;
  try {
    result = assemble6502(editor.value);
  } catch (error) {
    document.getElementById("asm-stats").textContent = "Assembly failed";
    document.getElementById("cpu-status-text").textContent = "ASSEMBLY ERROR";
    document.getElementById("cpu-status-dot").className = "dot dot-red";
    log(error.message, "red");
    return;
  }

  cpu.load(result.origin, result.bytes);
  cpu.reset();
  const lineCount = editor.value.split(/\r?\n/).length;
  document.getElementById("asm-stats").textContent = lineCount + " lines | " + result.bytes.length + " bytes compiled";
  updateUI();
  const originHex = result.origin.toString(16).toUpperCase().padStart(4, "0");
  log("Assembled " + result.bytes.length + " bytes at $" + originHex + ".");
}

function updateUI() {
  document.getElementById("reg-a").textContent = cpu.a.toString(16).toUpperCase().padStart(2, '0');
  document.getElementById("reg-x").textContent = cpu.x.toString(16).toUpperCase().padStart(2, '0');
  document.getElementById("reg-y").textContent = cpu.y.toString(16).toUpperCase().padStart(2, '0');
  document.getElementById("reg-sp").textContent = cpu.sp.toString(16).toUpperCase().padStart(2, '0');
  document.getElementById("reg-pc").textContent = cpu.pc.toString(16).toUpperCase().padStart(4, '0');
  document.getElementById("cycle-counter").textContent = `Cycles: ${cpu.cycles}`;

  // Flags
  const flags = [
    ["flag-n", 0x80], ["flag-v", 0x40], ["flag-b", 0x10],
    ["flag-d", 0x08], ["flag-i", 0x04], ["flag-z", 0x02], ["flag-c", 0x01]
  ];
  for (const [id, mask] of flags) {
    const el = document.getElementById(id);
    const active = (cpu.p & mask) !== 0;
    if (active) el.classList.add("active");
    else el.classList.remove("active");
  }

  // Status Dot
  const dot = document.getElementById("cpu-status-dot");
  const text = document.getElementById("cpu-status-text");
  if (cpu.fault) {
    dot.className = "dot dot-red";
    text.textContent = "FAULT";
  } else if (cpu.halted) {
    dot.className = "dot";
    dot.style.background = "#ff5555";
    text.textContent = "HALTED (BRK)";
  } else {
    dot.className = "dot dot-green";
    text.textContent = "RUNNING";
  }

  renderMemory(0x0000);
  renderDisasm();
}

function renderMemory(base) {
  const memView = document.getElementById("memory-view");
  let html = "";
  for (let row = 0; row < 4; row++) {
    const addr = (base + row * 16) & 0xFFFF;
    let hex = "";
    let ascii = "";
    for (let col = 0; col < 16; col++) {
      const b = cpu.ram[(addr + col) & 0xFFFF];
      hex += b.toString(16).toUpperCase().padStart(2, '0') + " ";
      ascii += (b >= 32 && b <= 126) ? String.fromCharCode(b) : ".";
    }
    html += `<div>$${addr.toString(16).toUpperCase().padStart(4, '0')}: ${hex} |${ascii}|</div>`;
  }
  memView.innerHTML = html;
}

function renderDisasm() {
  const disView = document.getElementById("disasm-view");
  let html = "";
  let curr = cpu.pc;
  for (let i = 0; i < 5; i++) {
    const op = cpu.ram[curr];
    const isCurrent = i === 0;
    html += `<div class="disasm-line ${isCurrent ? 'current' : ''}">
      <span>$${curr.toString(16).toUpperCase().padStart(4, '0')}</span>
      <span>${op.toString(16).toUpperCase().padStart(2, '0')}</span>
      <span>OP_${op.toString(16).toUpperCase()}</span>
    </div>`;
    curr = (curr + 1) & 0xFFFF;
  }
  disView.innerHTML = html;
}

function log(msg, tone = "green") {
  const c = document.getElementById("console-view");
  const line = document.createElement("div");
  line.className = `console-line text-${tone}`;
  line.textContent = `> ${msg}`;
  c.appendChild(line);
  c.scrollTop = c.scrollHeight;
}

// Event Listeners
document.getElementById("btn-step").addEventListener("click", () => {
  cpu.step();
  updateUI();
});

document.getElementById("btn-run").addEventListener("click", () => {
  let steps = 0;
  while (!cpu.halted && steps < 1000) {
    cpu.step();
    steps++;
  }
  updateUI();
  log(cpu.fault || `Executado até parada ou limite (1000 passos).`, cpu.fault ? "red" : "green");
});

document.getElementById("btn-reset").addEventListener("click", () => {
  cpu.reset();
  updateUI();
  log("CPU reiniciada via Vetor de Reset.");
});

document.getElementById("btn-assemble").addEventListener("click", assembleAndLoad);

document.getElementById("btn-mem-zp").addEventListener("click", () => renderMemory(0x0000));
document.getElementById("btn-mem-stack").addEventListener("click", () => renderMemory(0x0100));
document.getElementById("btn-mem-code").addEventListener("click", () => renderMemory(0x8000));

exampleSelect.addEventListener("change", (e) => loadExample(e.target.value));

// Inicialização com primeiro exemplo
loadExample("fibonacci");
