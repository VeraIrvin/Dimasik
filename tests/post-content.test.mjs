import assert from "node:assert/strict";
import test from "node:test";
import {
  normalizePostDocument,
  truncatePostDocument,
  POST_DOCUMENT_PREVIEW_MAX_CHARACTERS,
  PostContentValidationError,
} from "../lib/post-content.ts";

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

/** Flattens the visible text the same way the preview cut counts it. */
function previewText(nodes) {
  let text = "";
  for (const node of nodes) {
    if (node.type === "text") text += node.text ?? "";
    if (node.content) text += previewText(node.content);
  }
  return text;
}

function paragraphDocument(text, marks) {
  return normalizePostDocument({
    type: "doc",
    content: [{ type: "paragraph", content: [{ type: "text", text, ...(marks ? { marks } : {}) }] }],
  });
}

test("collapsed preview is a formatted tree prefix of the same document", () => {
  const document = normalizePostDocument({
    type: "doc",
    content: [
      {
        type: "heading",
        attrs: { level: 3, textAlign: "center" },
        content: [{ type: "text", text: "Возникновение села" }],
      },
      {
        type: "paragraph",
        content: [
          { type: "text", text: "Начало ", marks: [{ type: "bold" }, { type: "italic" }] },
          {
            type: "text",
            text: "ссылка",
            marks: [{ type: "link", attrs: { href: "https://example.com/x", title: "Пример" } }],
          },
          { type: "text", text: ` ${"слово ".repeat(150)}скрытый хвост` },
        ],
      },
    ],
  });

  const preview = truncatePostDocument(document);

  assert.notEqual(preview, document);
  // The heading stays a separate heading with its attributes instead of
  // merging into the paragraph that follows it.
  assert.equal(preview.content[0].type, "heading");
  assert.deepEqual(preview.content[0].attrs, { level: 3, textAlign: "center" });
  assert.deepEqual(preview.content[0], document.content[0]);
  assert.equal(preview.content[1].type, "paragraph");
  // Marks and link attributes survive inside the retained prefix.
  assert.deepEqual(preview.content[1].content[0].marks, [{ type: "bold" }, { type: "italic" }]);
  assert.deepEqual(preview.content[1].content[1].marks, [
    { type: "link", attrs: { href: "https://example.com/x", title: "Пример" } },
  ]);
  // Only the prefix is kept; the hidden remainder is absent from the tree.
  assert.doesNotMatch(JSON.stringify(preview), /скрытый хвост/u);
  const retained = previewText(preview.content);
  assert.ok(retained.endsWith("…"));
  assert.ok(previewText(document.content).startsWith(retained.slice(0, -1)));
  // The prefix is still a canonical rich-text document on its own.
  assert.deepEqual(normalizePostDocument(preview), preview);
});

test("collapsed preview does not mutate the stored document", () => {
  const document = paragraphDocument(`${"длинный ".repeat(120)}финал`, [{ type: "underline" }]);
  const snapshot = structuredClone(document);

  const preview = truncatePostDocument(document);

  assert.notEqual(preview, document);
  assert.deepEqual(document, snapshot);
  // The ellipsis is added to a copy, never to the source text node.
  assert.ok(!snapshot.content[0].content[0].text.includes("…"));
  assert.ok(preview.content[0].content[0].text.endsWith("…"));
});

test("a document that fits the limit is returned unchanged", () => {
  const document = paragraphDocument("Короткая справка со ссылкой");
  assert.equal(truncatePostDocument(document), document);

  const atLimit = paragraphDocument("я".repeat(POST_DOCUMENT_PREVIEW_MAX_CHARACTERS));
  assert.equal(truncatePostDocument(atLimit), atLimit);

  const overLimit = paragraphDocument("я".repeat(POST_DOCUMENT_PREVIEW_MAX_CHARACTERS + 1));
  assert.notEqual(truncatePostDocument(overLimit), overLimit);
});

test("collapsed preview cuts at a word boundary with an ellipsis", () => {
  const text = "абвгд ".repeat(120).trimEnd();
  const document = paragraphDocument(text);

  const preview = truncatePostDocument(document);
  const cut = previewText(preview.content).slice(0, -1);

  assert.ok(previewText(preview.content).endsWith("…"));
  assert.ok(cut.length > 500);
  assert.ok(cut.length <= POST_DOCUMENT_PREVIEW_MAX_CHARACTERS);
  assert.equal(text.startsWith(`${cut} `), true);
  assert.equal(/\s$/u.test(cut), false);
});

test("collapsed preview hard-cuts an unbroken run at the limit", () => {
  const document = paragraphDocument("я".repeat(800));
  const retained = previewText(truncatePostDocument(document).content);

  assert.equal(retained, `${"я".repeat(POST_DOCUMENT_PREVIEW_MAX_CHARACTERS)}…`);
});

test("collapsed preview never splits surrogate pairs at the cut", () => {
  const astral = paragraphDocument(`${"𝔞".repeat(560)} ${"хвост ".repeat(80)}`);
  const astralCut = previewText(truncatePostDocument(astral).content).slice(0, -1);

  assert.equal([...astralCut].length, 560);
  assert.equal(astralCut.length, 1120);
  assert.doesNotMatch(astralCut, /[\uD800-\uDBFF](?![\uDC00-\uDFFF])|(?<![\uD800-\uDBFF])[\uDC00-\uDFFF]/u);

  const boundary = paragraphDocument(`${"𝔞".repeat(505)} ${"хвост ".repeat(40)}`);
  const boundaryCut = previewText(truncatePostDocument(boundary).content).slice(0, -1);

  assert.equal(boundaryCut.startsWith("𝔞".repeat(505)), true);
  assert.equal([...boundaryCut].length <= POST_DOCUMENT_PREVIEW_MAX_CHARACTERS, true);
  assert.doesNotMatch(boundaryCut, /[\uD800-\uDBFF](?![\uDC00-\uDFFF])|(?<![\uD800-\uDBFF])[\uDC00-\uDFFF]/u);
});

test("collapsed preview keeps the list ancestors of a truncated item", () => {
  const document = normalizePostDocument({
    type: "doc",
    content: [
      {
        type: "bulletList",
        content: [
          {
            type: "listItem",
            content: [{ type: "paragraph", content: [{ type: "text", text: "Первый пункт", marks: [{ type: "strike" }] }] }],
          },
          {
            type: "listItem",
            content: [{ type: "paragraph", content: [{ type: "text", text: "второй ".repeat(120).trimEnd() }] }],
          },
        ],
      },
      {
        type: "orderedList",
        attrs: { start: 3 },
        content: [{
          type: "listItem",
          content: [{ type: "paragraph", content: [{ type: "text", text: "поздний пункт" }] }],
        }],
      },
    ],
  });

  const preview = truncatePostDocument(document);

  // The whole ordered list sits past the cut and must not be rendered.
  assert.equal(preview.content.length, 1);
  const list = preview.content[0];
  assert.equal(list.type, "bulletList");
  assert.equal(list.content.length, 2);
  assert.equal(list.content[0].type, "listItem");
  assert.deepEqual(list.content[0], document.content[0].content[0]);
  assert.equal(list.content[1].type, "listItem");
  assert.equal(list.content[1].content[0].type, "paragraph");
  assert.ok(previewText(preview.content).endsWith("…"));
  assert.equal(normalizePostDocument(preview).content.length, 1);
});
