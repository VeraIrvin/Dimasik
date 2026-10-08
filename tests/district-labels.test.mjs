import assert from "node:assert/strict";
import test from "node:test";
import { districtLabelCollection } from "../lib/district-labels.ts";

test("each district gets one label inside its largest landmass, not in a hole", () => {
  const labels = districtLabelCollection([
    {
      type: "Feature",
      properties: { id: "district-1", name: "Первый" },
      geometry: {
        type: "MultiPolygon",
        coordinates: [
          [[[0, 0], [1, 0], [1, 1], [0, 1], [0, 0]]],
          [
            [[10, 10], [20, 10], [20, 20], [10, 20], [10, 10]],
            [[13, 13], [17, 13], [17, 17], [13, 17], [13, 13]],
          ],
        ],
      },
    },
    {
      type: "Feature",
      properties: { id: "district-2", name: "Второй" },
      geometry: {
        type: "Polygon",
        coordinates: [[[30, 0], [32, 0], [32, 2], [30, 2], [30, 0]]],
      },
    },
  ]);

  assert.equal(labels.features.length, 2);
  assert.deepEqual(labels.features.map((feature) => feature.properties.name), ["Первый", "Второй"]);
  const [x, y] = labels.features[0].geometry.coordinates;
  assert.ok(x > 10 && x < 20 && y > 10 && y < 20);
  assert.ok(!(x > 13 && x < 17 && y > 13 && y < 17));
  assert.deepEqual(labels.features[1].geometry.coordinates, [31, 1]);
});
