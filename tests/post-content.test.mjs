import assert from "node:assert/strict";
import test from "node:test";
import {
  normalizePostDocument,
  truncatePostDocument,
  POST_DOCUMENT_PREVIEW_MAX_CHARACTERS,
  MAX_POST_DOCUMENT_JSON_CHARACTERS,
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

function tableCell(text = "", attrs, type = "tableCell") {
  return {
    type,
    ...(attrs === undefined ? {} : { attrs }),
    content: [{
      type: "paragraph",
      ...(text ? { content: [{ type: "text", text }] } : {}),
    }],
  };
}

function table(rows) {
  return { type: "table", content: rows.map((content) => ({ type: "tableRow", content })) };
}

function tableDocument(rows) {
  return { type: "doc", content: [table(rows)] };
}

function tableGeometry(node) {
  return node.content.map((row) => row.content.map((cell) => ({ type: cell.type, attrs: cell.attrs })));
}

const DEFAULT_CELL_ATTRS = { colspan: 1, rowspan: 1, colwidth: null, align: null };

test("real TipTap header grid defaults normalize without mutating editor JSON", () => {
  const document = tableDocument(Array.from({ length: 3 }, (_, row) =>
    Array.from({ length: 3 }, (_, column) => ({
      type: row === 0 ? "tableHeader" : "tableCell",
      attrs: { ...DEFAULT_CELL_ATTRS },
      content: [{
        type: "paragraph",
        attrs: { textAlign: null },
        content: row === 0 && column === 0 ? [{ type: "text", text: "Заголовок" }] : [],
      }],
    })),
  ));
  const snapshot = structuredClone(document);
  const normalized = normalizePostDocument(document);

  assert.deepEqual(document, snapshot);
  assert.equal(normalized.content[0].content.length, 3);
  for (const row of normalized.content[0].content) {
    assert.equal(row.content.length, 3);
    for (const cell of row.content) {
      assert.deepEqual(cell.attrs, DEFAULT_CELL_ATTRS);
      assert.equal(cell.content[0].attrs, undefined);
    }
  }
  assert.deepEqual(normalized.content[0].content[1].content[0].content, [{ type: "paragraph" }]);
  assert.deepEqual(normalizePostDocument(normalized), normalized);
  assert.deepEqual(
    normalizePostDocument(tableDocument([[tableCell("Значение")]])).content[0].content[0].content[0].attrs,
    DEFAULT_CELL_ATTRS,
  );
});

test("table cells preserve spans, unsized widths, safe alignment and rich block formatting", () => {
  const header = tableCell("", { colspan: 2, rowspan: 2, colwidth: [180, 0], align: "center" }, "tableHeader");
  header.content = [
    {
      type: "heading",
      attrs: { level: 2, textAlign: "right" },
      content: [{
        type: "text",
        text: "Источники",
        marks: [
          { type: "bold" }, { type: "italic" }, { type: "underline" }, { type: "strike" },
          { type: "textStyle", attrs: { color: "rgb(156, 0, 0)" } },
          { type: "link", attrs: { href: "/archive/1", title: "Архив" } },
        ],
      }],
    },
    { type: "blockquote", content: [{ type: "paragraph", content: [{ type: "text", text: "Цитата" }, { type: "hardBreak" }] }] },
    { type: "orderedList", attrs: { start: 3 }, content: [{ type: "listItem", content: [{ type: "paragraph" }] }] },
  ];
  const nested = tableCell();
  nested.content = [table([[tableCell("Вложенная", { align: "right" })]])];
  const input = tableDocument([
    [header, tableCell("Год", { align: "left" })],
    [tableCell("1900")],
    [nested, tableCell(), tableCell("Итог")],
  ]);
  const snapshot = structuredClone(input);
  const normalized = normalizePostDocument(input);
  const cells = normalized.content[0].content;
  assert.deepEqual(cells[0].content[0].attrs, { colspan: 2, rowspan: 2, colwidth: [180, 0], align: "center" });
  assert.notEqual(cells[0].content[0].attrs.colwidth, input.content[0].content[0].content[0].attrs.colwidth);
  assert.deepEqual(cells[0].content[0].content[0].content[0].marks, [
    { type: "bold" }, { type: "italic" }, { type: "underline" }, { type: "strike" },
    { type: "textStyle", attrs: { color: "#9c0000" } },
    { type: "link", attrs: { href: "/archive/1", title: "Архив" } },
  ]);
  assert.equal(cells[2].content[0].content[0].type, "table");
  assert.equal(cells[2].content[0].content[0].content[0].content[0].attrs.align, "right");
  assert.deepEqual(normalizePostDocument(normalized), normalized);
  assert.deepEqual(input, snapshot);
});

test("a row fully covered by earlier rowspans may have no explicit cells", () => {
  const normalized = normalizePostDocument(tableDocument([
    [tableCell("Две строки", { colspan: 2, rowspan: 2 })],
    [],
  ]));
  assert.deepEqual(normalized.content[0].content[1], { type: "tableRow", content: [] });
  assert.deepEqual(normalizePostDocument(normalized), normalized);
});

test("table structure rejects misplaced nodes, unsupported keys and missing block content", () => {
  const cases = [
    { type: "doc", content: [{ type: "tableRow", content: [tableCell("x")] }] },
    { type: "doc", content: [tableCell("x")] },
    { type: "doc", content: [{ type: "paragraph", content: [table([[tableCell("x")]])] }] },
    { type: "doc", content: [{ ...table([[tableCell("x")]]), attrs: {} }] },
    { type: "doc", content: [{ ...table([[tableCell("x")]]), marks: [] }] },
    { type: "doc", content: [{ type: "table", content: [{ type: "tableRow", attrs: {}, content: [tableCell("x")] }] }] },
    { type: "doc", content: [{ type: "table", content: [{ type: "paragraph", content: [{ type: "text", text: "x" }] }] }] },
    tableDocument([[{ type: "tableCell", content: [] }]]),
    tableDocument([[{ type: "tableHeader" }]]),
    tableDocument([[{ type: "tableCell", content: [{ type: "text", text: "x" }] }]]),
    tableDocument([[{ type: "tableCell", marks: [], content: [{ type: "paragraph" }] }]]),
    tableDocument([[{ type: "tableCell", content: [{ type: "listItem", content: [{ type: "paragraph" }] }] }]]),
    { type: "doc", content: [{ type: "table" }] },
    { type: "doc", content: [{ type: "table", content: [{ type: "tableRow" }] }] },
  ];
  for (const document of cases) {
    assert.throws(() => normalizePostDocument(document), PostContentValidationError);
  }
});

test("table cell attributes reject arbitrary styles, unsafe values and incompatible width arrays", () => {
  const invalidAttrs = [
    null, [], { style: "background:url(javascript:alert(1))" }, { class: "injected" },
    { onclick: "alert(1)" }, { textAlign: "center" }, { background: "#fff" },
    ...[0, -1, 1.5, 101, Number.MAX_SAFE_INTEGER, "2", null].flatMap((value) => [
      { colspan: value }, { rowspan: value },
    ]),
    { colwidth: [] }, { colwidth: [10, 20] }, { colwidth: [null] },
    { colwidth: [-1] }, { colwidth: [10_001] }, { colwidth: [1.5] }, { colwidth: ["100"] },
    { colwidth: "100" }, { colwidth: { 0: 100, length: 1 } },
    ...["justify", "", "CENTER", "left;position:fixed", 1, {}].map((align) => ({ align })),
  ];
  for (const attrs of invalidAttrs) {
    assert.throws(
      () => normalizePostDocument(tableDocument([[tableCell("x", attrs)]])),
      PostContentValidationError,
      JSON.stringify(attrs),
    );
  }
  for (const type of ["tableCell", "tableHeader"]) {
    const normalized = normalizePostDocument(tableDocument([[tableCell("x", { colwidth: [10_000], align: null }, type)]]));
    assert.deepEqual(normalized.content[0].content[0].content[0].attrs, {
      ...DEFAULT_CELL_ATTRS, colwidth: [10_000],
    });
  }
});

test("table geometry rejects empty, ragged, overlapping, out-of-row and oversized grids", () => {
  const cases = [
    [], [[]],
    [[tableCell("x"), tableCell()], [tableCell()]],
    [[tableCell("x")], [tableCell(), tableCell()]],
    [[tableCell("x", { rowspan: 2 })]],
    [[tableCell("x"), tableCell("", { rowspan: 2 }), tableCell()], [tableCell("", { colspan: 2 })]],
    [[tableCell("x"), tableCell("", { rowspan: 2 })], []],
    [[tableCell("x", { colspan: 100 }), tableCell()]],
    [Array.from({ length: 101 }, () => tableCell("x"))],
    Array.from({ length: 101 }, () => [tableCell("x")]),
  ];
  for (const rows of cases) {
    assert.throws(() => normalizePostDocument(tableDocument(rows)), PostContentValidationError);
  }
});

test("bounded table edges and nesting still enforce the existing document resource caps", () => {
  const wide = normalizePostDocument(tableDocument([[tableCell("x", { colspan: 100, colwidth: Array(100).fill(0) })]]));
  assert.equal(wide.content[0].content[0].content[0].attrs.colspan, 100);
  const tall = normalizePostDocument(tableDocument([
    [tableCell("x", { rowspan: 100 })], ...Array.from({ length: 99 }, () => []),
  ]));
  assert.equal(tall.content[0].content.length, 100);

  function nestedTables(count) {
    let block = { type: "paragraph", content: [{ type: "text", text: "x" }] };
    for (let index = 0; index < count; index += 1) {
      block = table([[{ type: "tableCell", content: [block] }]]);
    }
    return { type: "doc", content: [block] };
  }
  assert.deepEqual(normalizePostDocument(normalizePostDocument(nestedTables(6))), normalizePostDocument(nestedTables(6)));
  assert.throws(() => normalizePostDocument(nestedTables(7)), PostContentValidationError);
  // 100 rows × 25 cells × (cell + paragraph) already exceeds 5,000 nodes,
  // while the input with omitted default attrs is below the JSON-size cap.
  assert.throws(() => normalizePostDocument(tableDocument(
    Array.from({ length: 100 }, (_, row) => Array.from({ length: 25 }, (_, column) =>
      tableCell(row === 0 && column === 0 ? "x" : ""),
    )),
  )), PostContentValidationError);
  assert.throws(() => normalizePostDocument(tableDocument([[tableCell("x".repeat(100_001))]])), PostContentValidationError);
  assert.throws(() => normalizePostDocument(tableDocument([[tableCell("x".repeat(200_001))]])), PostContentValidationError);
});

test("empty grids and nested blank cells do not fabricate visible document content", () => {
  for (const rows of [
    [[tableCell()]],
    [[tableCell(" \n\t")]],
    [[{ type: "tableCell", content: [table([[tableCell()]])] }]],
    [[{ type: "tableCell", content: [{ type: "paragraph", content: [{ type: "hardBreak" }] }] }]],
  ]) {
    assert.throws(
      () => normalizePostDocument(tableDocument(rows)),
      (error) => error instanceof PostContentValidationError && /не должен быть пустым/u.test(error.message),
    );
  }
});

function hiddenLinkCell(text = "hidden-link-text") {
  const cell = tableCell(text);
  cell.content[0].content[0].marks = [{ type: "link", attrs: { href: "https://hidden.example/private", title: "hidden-link-title" } }];
  return cell;
}

function assertSafeTablePreview(document, limit) {
  const snapshot = structuredClone(document);
  const preview = truncatePostDocument(document, limit, limit);
  assert.notEqual(preview, document);
  assert.deepEqual(document, snapshot);
  assert.deepEqual(normalizePostDocument(preview), preview);
  assert.equal([...previewText(preview.content)].length, limit + 1);
  assert.ok(previewText(document.content).startsWith(previewText(preview.content).slice(0, -1)));
  assert.doesNotMatch(JSON.stringify(preview), /hidden-link|hidden\.example/u);
  return preview;
}

test("preview cut inside the first header cell preserves all rows and empties omitted cell links", () => {
  const document = normalizePostDocument(tableDocument([
    [tableCell("𝔞".repeat(12), { colspan: 2, rowspan: 2, colwidth: [120, 0], align: "center" }, "tableHeader"), hiddenLinkCell()],
    [hiddenLinkCell()],
    [hiddenLinkCell(), hiddenLinkCell(), hiddenLinkCell()],
  ]));
  document.content[0].content[0].content[0].content[0].content[0].marks = [{ type: "bold" }];
  const preview = assertSafeTablePreview(document, 5);
  const grid = preview.content[0];
  assert.deepEqual(tableGeometry(grid), tableGeometry(document.content[0]));
  assert.equal(previewText(grid.content), "𝔞𝔞𝔞𝔞𝔞…");
  assert.deepEqual(grid.content[0].content[0].content[0].content[0].marks, [{ type: "bold" }]);
  for (const cell of [grid.content[0].content[1], ...grid.content[1].content, ...grid.content[2].content]) {
    assert.deepEqual(cell.content, [{ type: "paragraph" }]);
  }
});

test("preview prefix can end inside a later cell or exactly between cells and rows", () => {
  for (const limit of [2, 3, 4, 5]) {
    const document = normalizePostDocument(tableDocument([
      [tableCell("ab"), tableCell("cd")],
      [tableCell("ef"), hiddenLinkCell()],
      [hiddenLinkCell(), hiddenLinkCell()],
    ]));
    const preview = assertSafeTablePreview(document, limit);
    assert.deepEqual(tableGeometry(preview.content[0]), tableGeometry(document.content[0]));
    assert.equal(previewText(preview.content), `${"abcdef".slice(0, limit)}…`);
    assert.deepEqual(preview.content[0].content[2].content.map((cell) => cell.content), [
      [{ type: "paragraph" }], [{ type: "paragraph" }],
    ]);
  }
});

test("nested table previews retain each reached grid and remove whole omitted nested contents", () => {
  const nested = {
    type: "tableCell",
    content: [
      { type: "paragraph", content: [{ type: "text", text: "ab" }] },
      table([
        [tableCell("𝔞𝔞𝔞", { rowspan: 2 }), tableCell("cd")],
        [hiddenLinkCell()],
      ]),
      { type: "paragraph", content: [{ type: "text", text: "hidden-link-block" }] },
    ],
  };
  const omittedNested = { type: "tableCell", content: [table([[hiddenLinkCell()]])] };
  const document = normalizePostDocument(tableDocument([[nested, omittedNested]]));
  const preview = assertSafeTablePreview(document, 4);
  const outer = preview.content[0];
  assert.deepEqual(tableGeometry(outer), tableGeometry(document.content[0]));
  const nestedGrid = outer.content[0].content[0].content[1];
  assert.deepEqual(tableGeometry(nestedGrid), tableGeometry(document.content[0].content[0].content[0].content[1]));
  assert.equal(previewText(preview.content), "ab𝔞𝔞…");
  assert.deepEqual(nestedGrid.content[1].content[0].content, [{ type: "paragraph" }]);
  assert.deepEqual(outer.content[0].content[1].content, [{ type: "paragraph" }]);
  assert.equal(outer.content[0].content[0].content.length, 2);
});

test("preview retains a completed table but never mounts tables wholly past the prefix", () => {
  const completed = table([[tableCell("ab"), tableCell("cd")]]);
  const document = normalizePostDocument({
    type: "doc",
    content: [
      completed,
      { type: "paragraph", content: [{ type: "text", text: "efgh" }] },
      table([[hiddenLinkCell()]]),
    ],
  });
  const preview = assertSafeTablePreview(document, 6);
  assert.equal(preview.content.length, 2);
  assert.deepEqual(preview.content[0], document.content[0]);
  assert.equal(previewText(preview.content), "abcdef…");

  const beforeTable = normalizePostDocument({
    type: "doc",
    content: [{ type: "paragraph", content: [{ type: "text", text: "abcdefgh" }] }, table([[hiddenLinkCell()]])],
  });
  assert.equal(assertSafeTablePreview(beforeTable, 4).content.length, 1);
});

test("a short table preview is the original full document, including its visible links", () => {
  const document = normalizePostDocument(tableDocument([[tableCell("Кратко"), hiddenLinkCell("Ссылка")]]));
  assert.equal(truncatePostDocument(document), document);
  assert.match(JSON.stringify(document), /hidden\.example/u);
});

test("sparse table arrays and nonfinite cell numbers are rejected instead of producing invalid canonical JSON", () => {
  const cases = [
    { type: "doc", content: [{ type: "table", content: Array(1) }] },
    tableDocument([Array(1)]),
    tableDocument([[{ type: "tableCell", content: Array(1) }]]),
    tableDocument([[{ type: "tableCell", content: [{ type: "paragraph", content: Array(1) }] }]]),
    tableDocument([[tableCell("x", { colwidth: Array(1) })]]),
    ...[NaN, Infinity, -Infinity].flatMap((value) => [
      tableDocument([[tableCell("x", { colspan: value })]]),
      tableDocument([[tableCell("x", { rowspan: value })]]),
      tableDocument([[tableCell("x", { colwidth: [value] })]]),
    ]),
  ];
  for (const document of cases) {
    assert.throws(() => normalizePostDocument(document), PostContentValidationError);
  }
});

test("unsafe inline content remains rejected when nested inside a table cell", () => {
  for (const marks of [
    [{ type: "link", attrs: { href: "javascript:alert(1)" } }],
    [{ type: "textStyle", attrs: { color: "#fff;position:fixed" } }],
    [{ type: "bold", attrs: { style: "position:fixed" } }],
  ]) {
    const cell = tableCell("x");
    cell.content[0].content[0].marks = marks;
    assert.throws(() => normalizePostDocument(tableDocument([[cell]])), PostContentValidationError);
  }
});

test("table preview word boundaries keep visible link formatting without mounting later links", () => {
  const visible = tableCell("𝔞𝔞𝔞 word tail");
  visible.content[0].content[0].marks = [
    { type: "underline" },
    { type: "link", attrs: { href: "/visible", title: "Сохраненная ссылка" } },
  ];
  const document = normalizePostDocument(tableDocument([[visible, hiddenLinkCell()]]));
  const snapshot = structuredClone(document);
  const preview = truncatePostDocument(document, 10, 3);
  assert.equal(previewText(preview.content), "𝔞𝔞𝔞 word…");
  assert.deepEqual(preview.content[0].content[0].content[0].content[0].content[0].marks, [
    { type: "underline" },
    { type: "link", attrs: { href: "/visible", title: "Сохраненная ссылка" } },
  ]);
  assert.doesNotMatch(JSON.stringify(preview), /hidden-link|hidden\.example|tail/u);
  assert.deepEqual(normalizePostDocument(preview), preview);
  assert.deepEqual(document, snapshot);
});

test("truncating a spanning table keeps even fully covered empty rows", () => {
  const document = normalizePostDocument(tableDocument([
    [tableCell("abcdefgh", { colspan: 2, rowspan: 3, colwidth: [0, 100] })],
    [],
    [],
  ]));
  const preview = assertSafeTablePreview(document, 4);
  assert.deepEqual(tableGeometry(preview.content[0]), tableGeometry(document.content[0]));
  assert.deepEqual(preview.content[0].content[1], { type: "tableRow", content: [] });
  assert.deepEqual(preview.content[0].content[2], { type: "tableRow", content: [] });
});

test("adding canonical table defaults cannot exceed the stored document JSON cap", () => {
  const raw = tableDocument(Array.from({ length: 100 }, (_, row) =>
    Array.from({ length: 20 }, (_, column) => tableCell(row === 0 && column === 0 ? "x" : "")),
  ));
  const withDefaults = structuredClone(raw);
  for (const row of withDefaults.content[0].content) {
    for (const cell of row.content) cell.attrs = { ...DEFAULT_CELL_ATTRS };
  }
  assert.ok(JSON.stringify(raw).length < MAX_POST_DOCUMENT_JSON_CHARACTERS);
  assert.ok(JSON.stringify(withDefaults).length > MAX_POST_DOCUMENT_JSON_CHARACTERS);
  // This shape has only 4,102 nodes and one text character; the final JSON
  // cap, not the node/text/grid caps, rejects the inflated canonical body.
  assert.throws(() => normalizePostDocument(raw), PostContentValidationError);
});

test("a canonical table at the exact JSON boundary remains readable and idempotent", () => {
  const document = normalizePostDocument(tableDocument(Array.from({ length: 100 }, (_, row) =>
    Array.from({ length: 10 }, (_, column) => tableCell(row === 0 && column === 0 ? "x" : "")),
  )));
  const padding = MAX_POST_DOCUMENT_JSON_CHARACTERS - JSON.stringify(document).length;
  assert.ok(padding > 0 && padding < 100_000);
  const text = document.content[0].content[0].content[0].content[0].content[0];
  text.text += "x".repeat(padding);
  assert.equal(JSON.stringify(document).length, MAX_POST_DOCUMENT_JSON_CHARACTERS);
  assert.deepEqual(normalizePostDocument(document), document);
  assert.deepEqual(normalizePostDocument(normalizePostDocument(document)), document);
  text.text += "x";
  assert.throws(() => normalizePostDocument(document), PostContentValidationError);
});

test("explicit left block alignment survives save/read inside center and right aligned cells", () => {
  for (const align of ["center", "right"]) {
    const cell = tableCell("Параграф", { align });
    cell.content[0].attrs = { textAlign: "left" };
    cell.content.push({
      type: "heading",
      attrs: { level: 3, textAlign: "left" },
      content: [{ type: "text", text: "Заголовок" }],
    });
    const document = normalizePostDocument(tableDocument([[cell]]));
    const stored = document.content[0].content[0].content[0];
    assert.equal(stored.attrs.align, align);
    assert.deepEqual(stored.content[0].attrs, { textAlign: "left" });
    assert.deepEqual(stored.content[1].attrs, { level: 3, textAlign: "left" });
    const read = normalizePostDocument(JSON.parse(JSON.stringify(document)));
    assert.deepEqual(read, document);
    assert.deepEqual(normalizePostDocument(read), read);
  }
});
