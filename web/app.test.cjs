const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");
const test = require("node:test");
const vm = require("node:vm");

function loadApp() {
  const elements = new Map();
  const handlers = new Map();
  function getElementById(id) {
    if (!elements.has(id)) {
      elements.set(id, {
        value: "",
        textContent: "",
        innerHTML: "",
        className: "",
        style: {},
        scrollHeight: 0,
        addEventListener(event, handler) { handlers.set(`${id}:${event}`, handler); },
        appendChild() {},
        classList: { add() {}, remove() {} },
      });
    }
    return elements.get(id);
  }

  const context = vm.createContext({
    document: { getElementById, createElement: () => getElementById(`created-${elements.size}`) },
  });
  const webDir = __dirname;
  vm.runInContext(fs.readFileSync(path.join(webDir, "assembler.js"), "utf8"), context);
  const appSource = fs.readFileSync(path.join(webDir, "app.js"), "utf8");
  vm.runInContext(appSource, context);
  const app = vm.runInContext("({ cpu, loadExample, assembleAndLoad, editor, EXAMPLES })", context);
  return { ...app, elements, handlers };
}

test("browser examples assemble and execute to their documented results", () => {
  const app = loadApp();
  app.loadExample("fibonacci");
  app.handlers.get("btn-run:click")();
  assert.equal(app.cpu.ram[0x00], 0x15);
  assert.equal(app.cpu.ram[0x01], 0x22);
  assert.equal(app.cpu.halted, true);

  app.loadExample("sum_loop");
  app.handlers.get("btn-run:click")();
  assert.equal(app.cpu.ram[0x00], 15);
  assert.equal(app.cpu.halted, true);
});

test("unsupported source does not replace the loaded program", () => {
  const app = loadApp();
  const previousProgramCounter = app.cpu.pc;
  app.editor.value = "NOT_AN_OPCODE $10";
  app.assembleAndLoad();

  assert.equal(app.elements.get("asm-stats").textContent, "Assembly failed");
  assert.equal(app.cpu.pc, previousProgramCounter);
});

test("unknown machine opcodes halt with an address-bearing fault", () => {
  const app = loadApp();
  app.cpu.load(0x8000, [0x02]);
  app.cpu.reset();
  app.cpu.step();

  assert.equal(app.cpu.halted, true);
  assert.equal(app.cpu.fault, "Unsupported opcode $02 at $8000");
});

test("loading a newly assembled program clears memory left by the prior one", () => {
  const app = loadApp();
  app.cpu.ram[0x9000] = 0xA9;
  app.editor.value = "BRK";
  app.assembleAndLoad();

  assert.equal(app.cpu.ram[0x8000], 0x00);
  assert.equal(app.cpu.ram[0x9000], 0x00);
});

test("loading code at the reset-vector address preserves its first opcode", () => {
  const app = loadApp();
  app.cpu.load(0xFFFC, [0xEA]);
  app.cpu.reset();
  app.cpu.step();

  assert.equal(app.cpu.pc, 0xFFFD);
  assert.equal(app.cpu.fault, null);
  assert.equal(app.cpu.cycles, 9);
});

test("taken conditional branches add a cycle when crossing a page", () => {
  for (const opcode of [0xD0, 0xF0]) {
    const app = loadApp();
    app.cpu.load(0x80FD, [opcode, 0x01]);
    app.cpu.reset();
    if (opcode === 0xF0) app.cpu.setFlag(0x02, true);
    app.cpu.step();

    assert.equal(app.cpu.pc, 0x8100);
    assert.equal(app.cpu.cycles, 11);
  }
});

test("invalid program images are rejected without altering the active program", () => {
  const app = loadApp();
  app.cpu.load(0x8000, [0xEA]);

  assert.throws(() => app.cpu.load(0xFFFF, [0xEA, 0x00]), /exceeds the 64 KiB/);
  assert.throws(() => app.cpu.load(-1, [0xEA]), /16-bit address/);
  assert.equal(app.cpu.ram[0x8000], 0xEA);
  assert.equal(app.cpu.ram[0xFFFF], 0x00);
});
