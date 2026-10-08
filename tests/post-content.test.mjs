import assert from "node:assert/strict";
import test from "node:test";
import { normalizePostDocument, PostContentValidationError } from "../lib/post-content.ts";

function forumClipboardDocument() {
  // The original forum post pastes as one paragraph with hundreds of <br>s.
  // Chromium adds rgb() color marks even to otherwise unformatted black text.
  const lines = Array.from({ length: 411 }, (_, index) => [
    {
      type: "text",
      text: `у него сын Андрей ${index + 1}`,
      marks: [{ type: "textStyle", attrs: { color: "rgb(0, 0, 0)" } }],
    },
    { type: "hardBreak" },
  ]).flat();

  return {
    type: "doc",
    content: [{
      type: "paragraph",
      attrs: { textAlign: null },
      content: [
        { type: "text", text: "РГАДА, Ф. 350", marks: [{ type: "underline" }, { type: "bold" }] },
        ...lines,
        {
          type: "text",
          text: "Гуторов",
          marks: [
            { type: "bold" },
            { type: "textStyle", attrs: { color: "rgb(156, 0, 0)" } },
          ],
        },
      ],
    }],
  };
}

test("forum clipboard text retains line breaks and safe formatting", () => {
  const normalized = normalizePostDocument(forumClipboardDocument());
  const paragraph = normalized.content[0];
  const content = paragraph.content;

  assert.equal(paragraph.attrs, undefined);
  assert.equal(content.filter((node) => node.type === "hardBreak").length, 411);
  assert.deepEqual(content[0].marks, [{ type: "underline" }, { type: "bold" }]);
  assert.deepEqual(content[1].marks, [{ type: "textStyle", attrs: { color: "#000000" } }]);
  assert.deepEqual(content.at(-1).marks, [
    { type: "bold" },
    { type: "textStyle", attrs: { color: "#9c0000" } },
  ]);
  assert.equal(content[1].text, "у него сын Андрей 1");
});

test("unset pasted color does not discard other formatting", () => {
  const normalized = normalizePostDocument({
    type: "doc",
    content: [{
      type: "paragraph",
      content: [{
        type: "text",
        text: "фрагмент",
        marks: [{ type: "textStyle", attrs: { color: "" } }, { type: "bold" }],
      }],
    }],
  });

  assert.deepEqual(normalized.content[0].content[0].marks, [{ type: "bold" }]);
});

test("pasted CSS cannot inject arbitrary styles", () => {
  const unsafe = {
    type: "doc",
    content: [{
      type: "paragraph",
      content: [{
        type: "text",
        text: "фрагмент",
        marks: [{
          type: "textStyle",
          attrs: { color: "rgb(0,0,0);background:url(javascript:alert(1))" },
        }],
      }],
    }],
  };

  assert.throws(() => normalizePostDocument(unsafe), PostContentValidationError);
});
