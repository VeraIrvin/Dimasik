import assert from "node:assert/strict";
import test from "node:test";
import {
  findPostHighlightRanges,
  findTextHighlightRanges,
  normalizePostHighlightQuery,
} from "../lib/post-search-highlight.ts";

function documentWith(...content) {
  return { type: "doc", content };
}

function paragraph(...content) {
  return { type: "paragraph", content };
}

test("normalizes Unicode whitespace, preserves case, and rejects overlong queries", () => {
  assert.equal(normalizePostHighlightQuery("  Род\u00a0\n ДОМ  "), "Род ДОМ");
  assert.equal(normalizePostHighlightQuery("я".repeat(200)), "я".repeat(200));
  assert.equal(normalizePostHighlightQuery("я".repeat(201)), "");
  assert.equal(normalizePostHighlightQuery(`${" ".repeat(250)}род`), "род");
});

test("matches a phrase across adjacent differently marked inline leaves", () => {
  const first = { type: "text", text: "Старый ", marks: [{ type: "bold" }] };
  const second = { type: "text", text: "дворян", marks: [{ type: "link", attrs: { href: "/" } }] };
  const third = { type: "text", text: "ский род", marks: [{ type: "italic" }] };
  const ranges = findPostHighlightRanges(
    documentWith(paragraph(first, second, third)),
    "дворянский род",
  );

  assert.deepEqual(ranges.get(first), undefined);
  assert.deepEqual(ranges.get(second), [{ start: 0, end: 6 }]);
  assert.deepEqual(ranges.get(third), [{ start: 0, end: 8 }]);
});

test("block and hard-break boundaries become whitespace without joining words", () => {
  const alpha = { type: "text", text: "Альфа" };
  const beta = { type: "text", text: "Бета" };
  const hard = { type: "text", text: "Гамма" };
  const delta = { type: "text", text: "Дельта" };
  const body = documentWith(
    paragraph(alpha),
    paragraph(beta),
    paragraph(hard, { type: "hardBreak" }, delta),
  );

  assert.equal(findPostHighlightRanges(body, "альфабета").size, 0);
  assert.deepEqual(findPostHighlightRanges(body, "альфа бета").get(alpha), [{ start: 0, end: 5 }]);
  assert.deepEqual(findPostHighlightRanges(body, "гамма дельта").get(delta), [{ start: 0, end: 6 }]);
});

test("table cells participate with structural whitespace", () => {
  const left = { type: "text", text: "Петров" };
  const right = { type: "text", text: "Москва" };
  const body = documentWith({
    type: "table",
    content: [{
      type: "tableRow",
      content: [
        { type: "tableCell", content: [paragraph(left)] },
        { type: "tableCell", content: [paragraph(right)] },
      ],
    }],
  });
  const ranges = findPostHighlightRanges(body, "петров москва");

  assert.deepEqual(ranges.get(left), [{ start: 0, end: 6 }]);
  assert.deepEqual(ranges.get(right), [{ start: 0, end: 6 }]);
});

test("case expansion and non-BMP characters retain original UTF-16 offsets", () => {
  assert.deepEqual(findTextHighlightRanges("Aİ😀Б", "İ😀"), [{ start: 1, end: 4 }]);
  assert.deepEqual(findTextHighlightRanges("Ёлка ёлка", "ёЛкА"), [
    { start: 0, end: 4 },
    { start: 5, end: 9 },
  ]);
  assert.deepEqual(findTextHighlightRanges("е елка", "ё"), []);
});

test("literal script-like and regexp punctuation is matched repeatedly", () => {
  assert.deepEqual(findTextHighlightRanges("<script>.*</script> и <SCRIPT>.*</SCRIPT>", "<script>.*</script>"), [
    { start: 0, end: 19 },
    { start: 22, end: 41 },
  ]);
});
