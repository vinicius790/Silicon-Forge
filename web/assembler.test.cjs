const assert = require("node:assert/strict");
const test = require("node:test");
const { assemble6502 } = require("./assembler.js");

test("assembles labels and calculates each relative branch", () => {
  const result = assemble6502(".org $8000\nloop: DEX\nBNE loop\nBRK");

  assert.equal(result.origin, 0x8000);
  assert.deepEqual(result.bytes, [0xCA, 0xD0, 0xFD, 0x00]);
  assert.equal(result.labels.LOOP, 0x8000);
});

test("preserves the supported sum example result encoding", () => {
  const source = [
    "LDX #$05", "LDA #$00", "CLC", "loop:", "STX $01", "ADC $01",
    "DEX", "BNE loop", "STA $00", "BRK",
  ].join("\n");
  assert.deepEqual(assemble6502(source).bytes, [
    0xA2, 0x05, 0xA9, 0x00, 0x18, 0x86, 0x01, 0x65, 0x01,
    0xCA, 0xD0, 0xF9, 0x85, 0x00, 0x00,
  ]);
});

test("rejects unknown opcodes and out-of-range branches instead of dropping code", () => {
  assert.throws(() => assemble6502("LDA #$01\nMYSTERY $10"), /unsupported instruction/);
  assert.throws(
    () => assemble6502(`BNE far\n${"NOP\n".repeat(128)}far: BRK`),
    /branch target is out of range/,
  );
});

test("rejects duplicate labels and invalid byte operands", () => {
  assert.throws(() => assemble6502("again: NOP\nagain: BRK"), /duplicate label/);
  assert.throws(() => assemble6502("LDA #$100"), /byte operand out of range/);
});
