import assert from "node:assert/strict";
import test from "node:test";
import {
  SETTLEMENT_URL_SLUG_MAX_LENGTH,
  SETTLEMENT_URL_PREFIX,
  isValidSettlementSlug,
  settlementUrlForSlug,
} from "../lib/settlement-url.ts";

test("settlement slugs keep the store's lower-case hyphenated shape", () => {
  assert.equal(isValidSettlementSlug("pavelec"), true);
  assert.equal(isValidSettlementSlug("gubernia-1897-2"), true);
  assert.equal(isValidSettlementSlug(""), false);
  assert.equal(isValidSettlementSlug("Pavelec"), false);
  assert.equal(isValidSettlementSlug("Rязань"), false);
  assert.equal(isValidSettlementSlug("pavelec_2"), false);
  assert.equal(isValidSettlementSlug("-pavelec"), false);
  assert.equal(isValidSettlementSlug("pavelec-"), false);
  assert.equal(isValidSettlementSlug("pavelec--2"), false);
  assert.equal(isValidSettlementSlug("pavelec 2"), false);
  assert.equal(
    isValidSettlementSlug("a".repeat(SETTLEMENT_URL_SLUG_MAX_LENGTH)),
    true,
  );
  assert.equal(
    isValidSettlementSlug("a".repeat(SETTLEMENT_URL_SLUG_MAX_LENGTH + 1)),
    false,
  );
});

test("the stored address joins the fixed prefix with the slug", () => {
  assert.equal(SETTLEMENT_URL_PREFIX, "/naselennyy-punkt/");
  assert.equal(settlementUrlForSlug("pavelec"), "/naselennyy-punkt/pavelec");
  assert.equal(
    settlementUrlForSlug("gubernia-1897-2"),
    "/naselennyy-punkt/gubernia-1897-2",
  );
});
