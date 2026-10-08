import assert from "node:assert/strict";
import test from "node:test";
import { isPointInUyezd } from "../lib/uyezd-data.ts";

test("a settlement must fall inside its selected historical district", async () => {
  // A point near Tula is in the historical Tulsky district, not in Aleksinsky
  // or in another province. A reversed latitude/longitude must be rejected.
  assert.equal(await isPointInUyezd("tula", "uyezd-1897-306", 37.6, 54.25), true);
  assert.equal(await isPointInUyezd("tula", "uyezd-1897-301", 37.6, 54.25), false);
  assert.equal(await isPointInUyezd("ryazan", "uyezd-1897-306", 37.6, 54.25), false);
  assert.equal(await isPointInUyezd("tula", "uyezd-1897-306", 54.25, 37.6), false);
});
