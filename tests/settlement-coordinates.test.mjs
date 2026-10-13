import assert from "node:assert/strict";
import test from "node:test";
import {
  formatSettlementCoordinates,
  hasSettlementCoordinates,
  parseSettlementCoordinates,
} from "../lib/settlement-coordinates.ts";

test("coordinate guards preserve real zeroes and reject absent or invalid pairs", () => {
  assert.equal(hasSettlementCoordinates({ latitude: 0, longitude: 0 }), true);
  assert.equal(hasSettlementCoordinates({ latitude: -90, longitude: 180 }), true);
  for (const settlement of [
    { latitude: null, longitude: null },
    { latitude: null, longitude: 40 },
    { latitude: 56, longitude: null },
    { latitude: NaN, longitude: 40 },
    { latitude: 56, longitude: Infinity },
    { latitude: 91, longitude: 40 },
    { latitude: 56, longitude: -181 },
  ]) {
    assert.equal(hasSettlementCoordinates(settlement), false);
  }
});

test("coordinate formatting keeps the existing latitude, longitude text", () => {
  assert.equal(formatSettlementCoordinates({ latitude: 56.1234, longitude: 40.5678 }), "56.1234, 40.5678");
  assert.equal(formatSettlementCoordinates({ latitude: 0, longitude: 0 }), "0, 0");
  assert.equal(formatSettlementCoordinates({ latitude: null, longitude: null }), "");
});

test("blank coordinate input explicitly represents no point", () => {
  assert.equal(parseSettlementCoordinates(""), null);
  assert.equal(parseSettlementCoordinates("  \t\n "), null);
});

test("coordinate parsing accepts complete finite pairs within inclusive ranges", () => {
  assert.deepEqual(parseSettlementCoordinates(" 56.1234, 40.5678 "), [56.1234, 40.5678]);
  assert.deepEqual(parseSettlementCoordinates("0,0"), [0, 0]);
  assert.deepEqual(parseSettlementCoordinates("-90, -180"), [-90, -180]);
  assert.deepEqual(parseSettlementCoordinates("90,180"), [90, 180]);
});

test("nonempty malformed or out-of-range coordinate input raises a user-facing error", () => {
  for (const input of [
    "56", "56,", ",40", ",", "56,40,1", "north, east",
    "NaN,40", "56,Infinity", "90.1,40", "-90.1,40", "56,180.1", "56,-180.1",
  ]) {
    assert.throws(
      () => parseSettlementCoordinates(input),
      /Укажите координаты в формате «широта, долгота»/,
    );
  }
});
