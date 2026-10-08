export const MAX_POST_DOCUMENT_JSON_CHARACTERS = 200_000;
const MAX_DOCUMENT_DEPTH = 20;
const MAX_DOCUMENT_NODES = 5_000;
const MAX_DOCUMENT_TEXT = 100_000;
const MAX_LINK_HREF_LENGTH = 2_048;
const MAX_LINK_TITLE_LENGTH = 512;

const EMPTY_DOCUMENT_MESSAGE = "Текст публикации не должен быть пустым.";

// Static lookup tables: membership checks run on attacker-controlled keys, and
// a record lookup keeps every rejected key out of prototype-polluting paths.
const ALIGNMENTS: Record<string, true> = {
  left: true,
  center: true,
  right: true,
  justify: true,
};

const ALLOWED_PARENTS: Record<string, Record<string, true>> = {
  paragraph: { doc: true, blockquote: true, listItem: true },
  heading: { doc: true, blockquote: true, listItem: true },
  blockquote: { doc: true, blockquote: true, listItem: true },
  bulletList: { doc: true, blockquote: true, listItem: true },
  orderedList: { doc: true, blockquote: true, listItem: true },
  listItem: { bulletList: true, orderedList: true },
  text: { inline: true },
  hardBreak: { inline: true },
};

const NODE_KEYS: Record<string, Record<string, true>> = {
  doc: { type: true, content: true },
  paragraph: { type: true, attrs: true, content: true },
  heading: { type: true, attrs: true, content: true },
  blockquote: { type: true, content: true },
  bulletList: { type: true, content: true },
  orderedList: { type: true, attrs: true, content: true },
  listItem: { type: true, content: true },
  text: { type: true, text: true, marks: true },
  hardBreak: { type: true },
  alignmentAttrs: { textAlign: true },
  headingAttrs: { level: true, textAlign: true },
  orderedListAttrs: { start: true },
  mark: { type: true, attrs: true },
  textStyleAttrs: { color: true },
  linkAttrs: { href: true, target: true, rel: true, class: true, title: true },
};

const COLOR_PATTERN = /^#(?:[0-9a-f]{3}|[0-9a-f]{6})$/i;
const RGB_COLOR_PATTERN =
  /^rgb\(\s*(\d{1,3})\s*,\s*(\d{1,3})\s*,\s*(\d{1,3})\s*\)$/i;
const CONTROL_CHARACTER_PATTERN = /[\u0000-\u001f\u007f]/;

export type PostMark = {
  type: string;
  attrs?: {
    href?: string;
    title?: string;
    color?: string;
  };
};

export type PostNode = {
  type: string;
  attrs?: {
    level?: number;
    textAlign?: string;
    start?: number;
  };
  text?: string;
  marks?: PostMark[];
  content?: PostNode[];
};

export type PostDocument = {
  type: "doc";
  content: PostNode[];
};

/** Raised for bodies that are not a safe, bounded rich-text document. */
export class PostContentValidationError extends Error {
  constructor(message = "Некорректное содержимое публикации.") {
    super(message);
    this.name = "PostContentValidationError";
  }
}

type ParentKind = "doc" | "blockquote" | "bulletList" | "orderedList" | "listItem" | "inline";

type ValidationContext = {
  nodes: number;
  textLength: number;
  hasVisibleContent: boolean;
};

function fail(): never {
  throw new PostContentValidationError();
}

function hasOnlyKeys(value: Record<string, unknown>, allowed: Record<string, true>) {
  return Object.keys(value).every((key) => allowed[key] === true);
}

function asRecord(value: unknown): Record<string, unknown> {
  if (typeof value !== "object" || value === null || Array.isArray(value)) fail();
  return value as Record<string, unknown>;
}

function normalizeLinkHref(value: unknown): string {
  if (typeof value !== "string" || value.length === 0 || value.length > MAX_LINK_HREF_LENGTH) fail();
  if (CONTROL_CHARACTER_PATTERN.test(value) || value.includes("\\")) fail();

  if (value.startsWith("/")) {
    // Root-relative paths only: "//host" is protocol-relative, not a path.
    if (value.startsWith("//")) fail();
    return value;
  }

  let url: URL;
  try {
    url = new URL(value);
  } catch {
    fail();
  }
  if (url.protocol !== "http:" && url.protocol !== "https:") fail();
  return value;
}

function normalizeAlignmentAttrs(value: unknown): PostNode["attrs"] | undefined {
  if (value === undefined) return undefined;
  const attrs = asRecord(value);
  if (!hasOnlyKeys(attrs, NODE_KEYS.alignmentAttrs)) fail();

  const alignment = attrs.textAlign;
  if (alignment === null || alignment === undefined || alignment === "left") return undefined;
  if (typeof alignment !== "string" || ALIGNMENTS[alignment] !== true) fail();
  return { textAlign: alignment };
}

function normalizeHeadingAttrs(value: unknown): PostNode["attrs"] {
  const attrs = asRecord(value);
  if (!hasOnlyKeys(attrs, NODE_KEYS.headingAttrs)) fail();

  const level = attrs.level;
  if (level !== 2 && level !== 3) fail();

  const alignment = attrs.textAlign;
  if (alignment === null || alignment === undefined || alignment === "left") return { level };
  if (typeof alignment !== "string" || ALIGNMENTS[alignment] !== true) fail();
  return { level, textAlign: alignment };
}

function normalizeOrderedListAttrs(value: unknown): PostNode["attrs"] | undefined {
  if (value === undefined) return undefined;
  const attrs = asRecord(value);
  if (!hasOnlyKeys(attrs, NODE_KEYS.orderedListAttrs)) fail();

  const start = attrs.start;
  if (start === null || start === undefined || start === 1) return undefined;
  if (typeof start !== "number" || !Number.isSafeInteger(start) || start < 1 || start > 1_000_000) fail();
  return { start };
}

function normalizeLinkAttrs(value: unknown): PostMark["attrs"] {
  const attrs = asRecord(value);
  if (!hasOnlyKeys(attrs, NODE_KEYS.linkAttrs)) fail();

  const { target, rel, class: className, title } = attrs;
  if (target !== undefined && target !== null && target !== "_blank") fail();
  if (rel !== undefined && rel !== null && rel !== "noopener noreferrer nofollow") fail();
  if (className !== undefined && className !== null) fail();

  const href = normalizeLinkHref(attrs.href);
  if (title === undefined || title === null) return { href };
  if (
    typeof title !== "string" ||
    title.length === 0 ||
    title.length > MAX_LINK_TITLE_LENGTH ||
    CONTROL_CHARACTER_PATTERN.test(title)
  ) {
    fail();
  }
  return { href, title };
}

function normalizeColor(value: unknown): string {
  if (typeof value !== "string") fail();
  if (COLOR_PATTERN.test(value)) return value.toLowerCase();

  const match = RGB_COLOR_PATTERN.exec(value);
  if (match === null) fail();
  const channels = match.slice(1).map(Number);
  if (channels.some((channel) => channel > 255)) fail();
  return `#${channels.map((channel) => channel.toString(16).padStart(2, "0")).join("")}`;
}

function normalizeMark(value: unknown): PostMark | null {
  const mark = asRecord(value);
  if (!hasOnlyKeys(mark, NODE_KEYS.mark) || typeof mark.type !== "string") fail();

  switch (mark.type) {
    case "bold":
    case "italic":
    case "strike":
    case "underline":
      if (mark.attrs !== undefined && mark.attrs !== null) {
        const attrs = asRecord(mark.attrs);
        if (Object.keys(attrs).length !== 0) fail();
      }
      return { type: mark.type };
    case "textStyle": {
      const attrs = asRecord(mark.attrs);
      if (!hasOnlyKeys(attrs, NODE_KEYS.textStyleAttrs)) fail();
      const color = attrs.color;
      // An unset or empty color leaves a mark that renders nothing: drop it.
      // Browsers serialise computed clipboard colors as rgb(), even when the
      // source used a hex value; anything without a color (`color: ""`) and
      // that exact rgb shape are the only accepted values.
      if (color === undefined || color === null || color === "") return null;
      return { type: "textStyle", attrs: { color: normalizeColor(color) } };
    }
    case "link":
      return { type: "link", attrs: normalizeLinkAttrs(mark.attrs) };
    default:
      fail();
  }
}

function normalizeMarks(value: unknown): PostMark[] | undefined {
  if (value === undefined) return undefined;
  if (!Array.isArray(value) || value.length > 8) fail();
  if (value.length === 0) return undefined;

  const marks: PostMark[] = [];
  const seen = new Set<string>();
  for (const candidate of value) {
    const mark = normalizeMark(candidate);
    if (mark === null) continue;
    if (seen.has(mark.type)) fail();
    seen.add(mark.type);
    marks.push(mark);
  }
  return marks.length === 0 ? undefined : marks;
}

function normalizeBlockContent(
  value: unknown,
  parent: ParentKind,
  depth: number,
  context: ValidationContext,
): PostNode[] {
  if (!Array.isArray(value) || value.length === 0 || value.length > MAX_DOCUMENT_NODES) fail();
  return value.map((child) => normalizeNode(child, parent, depth + 1, context));
}

function normalizeInlineContent(
  value: unknown,
  depth: number,
  context: ValidationContext,
): PostNode[] | undefined {
  if (value === undefined) return undefined;
  if (!Array.isArray(value) || value.length > MAX_DOCUMENT_NODES) fail();
  if (value.length === 0) return undefined;
  return value.map((child) => normalizeNode(child, "inline", depth + 1, context));
}

function normalizeNode(
  value: unknown,
  parent: ParentKind,
  depth: number,
  context: ValidationContext,
): PostNode {
  if (depth > MAX_DOCUMENT_DEPTH) fail();
  const node = asRecord(value);
  const type = node.type;
  if (typeof type !== "string") fail();

  const allowedKeys = NODE_KEYS[type];
  const allowedParents = ALLOWED_PARENTS[type];
  if (allowedKeys === undefined || allowedParents?.[parent] !== true) fail();
  if (!hasOnlyKeys(node, allowedKeys)) fail();

  context.nodes += 1;
  if (context.nodes > MAX_DOCUMENT_NODES) fail();

  switch (type) {
    case "text": {
      if (typeof node.text !== "string" || node.text.length === 0) fail();
      context.textLength += node.text.length;
      if (context.textLength > MAX_DOCUMENT_TEXT) fail();
      if (node.text.trim().length > 0) context.hasVisibleContent = true;
      const marks = normalizeMarks(node.marks);
      return { type: "text", text: node.text, ...(marks === undefined ? {} : { marks }) };
    }
    case "hardBreak":
      return { type: "hardBreak" };
    case "paragraph": {
      const attrs = normalizeAlignmentAttrs(node.attrs);
      const content = normalizeInlineContent(node.content, depth, context);
      return {
        type: "paragraph",
        ...(attrs === undefined ? {} : { attrs }),
        ...(content === undefined ? {} : { content }),
      };
    }
    case "heading": {
      const attrs = normalizeHeadingAttrs(node.attrs);
      const content = normalizeInlineContent(node.content, depth, context);
      return { type: "heading", attrs, ...(content === undefined ? {} : { content }) };
    }
    case "blockquote":
      return { type: "blockquote", content: normalizeBlockContent(node.content, "blockquote", depth, context) };
    case "bulletList":
      return { type: "bulletList", content: normalizeBlockContent(node.content, "bulletList", depth, context) };
    case "orderedList": {
      const attrs = normalizeOrderedListAttrs(node.attrs);
      const content = normalizeBlockContent(node.content, "orderedList", depth, context);
      return { type: "orderedList", ...(attrs === undefined ? {} : { attrs }), content };
    }
    case "listItem": {
      const content = normalizeBlockContent(node.content, "listItem", depth, context);
      if (content[0].type !== "paragraph") fail();
      return { type: "listItem", content };
    }
    default:
      fail();
  }
}

/**
 * Validates and canonicalises an untrusted TipTap JSON body: only whitelisted
 * nodes, marks, attributes and link protocols survive, and every resource
 * bound (JSON characters, depth, nodes, text) is enforced before the body is stored.
 */
export function normalizePostDocument(value: unknown): PostDocument {
  let serialized: string | undefined;
  try {
    serialized = JSON.stringify(value);
  } catch {
    fail();
  }
  if (typeof serialized !== "string" || serialized.length > MAX_POST_DOCUMENT_JSON_CHARACTERS) fail();

  const document = asRecord(value);
  if (document.type !== "doc" || !hasOnlyKeys(document, NODE_KEYS.doc)) fail();

  const content = document.content;
  if (!Array.isArray(content) || content.length > MAX_DOCUMENT_NODES) fail();
  if (content.length === 0) throw new PostContentValidationError(EMPTY_DOCUMENT_MESSAGE);

  const context: ValidationContext = { nodes: 0, textLength: 0, hasVisibleContent: false };
  const normalized = content.map((child) => normalizeNode(child, "doc", 1, context));
  if (!context.hasVisibleContent) throw new PostContentValidationError(EMPTY_DOCUMENT_MESSAGE);
  return { type: "doc", content: normalized };
}
