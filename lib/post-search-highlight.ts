import type { PostDocument, PostNode } from "./post-content";

export const MAX_POST_HIGHLIGHT_QUERY_LENGTH = 200;

type SourcePosition = {
  node: PostNode;
  start: number;
  end: number;
};

export type HighlightRange = {
  start: number;
  end: number;
};

type Projection = {
  text: string;
  positions: Array<SourcePosition | null>;
};

const BLOCK_TYPES: Record<string, true> = {
  paragraph: true,
  heading: true,
  blockquote: true,
  bulletList: true,
  orderedList: true,
  listItem: true,
  table: true,
  tableRow: true,
  tableCell: true,
  tableHeader: true,
};
const UNICODE_WHITESPACE = /\s/u;

/** Collapses Unicode whitespace without changing the query's display case. */
export function normalizePostHighlightQuery(value: string | null | undefined): string {
  if (typeof value !== "string") return "";
  const normalized = value.trim().replace(/\s+/gu, " ");
  return normalized.length <= MAX_POST_HIGHLIGHT_QUERY_LENGTH ? normalized : "";
}


function createProjection(document: PostDocument): Projection {
  let text = "";
  const positions: Array<SourcePosition | null> = [];
  let pendingSeparator: SourcePosition | null | undefined;

  function requestSeparator(source: SourcePosition | null = null) {
    if (text.length > 0 && pendingSeparator === undefined) pendingSeparator = source;
  }

  function appendCharacter(character: string, source: SourcePosition) {
    if (UNICODE_WHITESPACE.test(character)) {
      requestSeparator(source);
      return;
    }

    if (pendingSeparator !== undefined) {
      text += " ";
      positions.push(pendingSeparator);
      pendingSeparator = undefined;
    }

    text += character;
    for (let index = 0; index < character.length; index += 1) positions.push(source);
  }

  function visit(node: PostNode) {
    if (node.type === "text") {
      const sourceText = node.text ?? "";
      let offset = 0;
      for (const character of sourceText) {
        const end = offset + character.length;
        appendCharacter(character, { node, start: offset, end });
        offset = end;
      }
      return;
    }

    if (node.type === "hardBreak") {
      requestSeparator();
      return;
    }

    for (const child of node.content ?? []) visit(child);
    if (BLOCK_TYPES[node.type]) requestSeparator();
  }

  for (const node of document.content) visit(node);

  const loweredPositions: Array<SourcePosition | null> = [];
  let offset = 0;
  for (const character of text) {
    const source = positions[offset] ?? null;
    for (let index = 0; index < character.toLowerCase().length; index += 1) {
      loweredPositions.push(source);
    }
    offset += character.length;
  }
  return { text: text.toLowerCase(), positions: loweredPositions };
}

function findMatchOffsets(text: string, query: string): HighlightRange[] {
  if (!query) return [];
  const matches: HighlightRange[] = [];
  let offset = 0;
  while (offset <= text.length - query.length) {
    const start = text.indexOf(query, offset);
    if (start === -1) break;
    matches.push({ start, end: start + query.length });
    offset = start + query.length;
  }
  return matches;
}

function mergeRange(ranges: HighlightRange[], next: HighlightRange) {
  const previous = ranges.at(-1);
  if (previous && next.start <= previous.end) {
    previous.end = Math.max(previous.end, next.end);
  } else {
    ranges.push(next);
  }
}

/** Finds literal, case-insensitive matches in plain visible text using UTF-16 offsets. */
export function findTextHighlightRanges(text: string, query: string): HighlightRange[] {
  const normalizedQuery = normalizePostHighlightQuery(query);
  if (!normalizedQuery) return [];

  const node: PostNode = { type: "text", text };
  const projection = createProjection({ type: "doc", content: [node] });
  const matches = findMatchOffsets(projection.text, normalizedQuery.toLowerCase());
  const ranges: HighlightRange[] = [];

  for (const match of matches) {
    let start = Number.POSITIVE_INFINITY;
    let end = -1;
    for (let index = match.start; index < match.end; index += 1) {
      const source = projection.positions[index];
      if (!source) continue;
      start = Math.min(start, source.start);
      end = Math.max(end, source.end);
    }
    if (end >= 0) mergeRange(ranges, { start, end });
  }

  return ranges;
}

/**
 * Projects rendered rich text while retaining leaf offsets. Inline leaves stay
 * contiguous; hard breaks and block/table boundaries become one space.
 */
export function findPostHighlightRanges(
  body: PostDocument,
  query: string,
): Map<PostNode, HighlightRange[]> {
  const normalizedQuery = normalizePostHighlightQuery(query);
  const result = new Map<PostNode, HighlightRange[]>();
  if (!normalizedQuery) return result;

  const projection = createProjection(body);
  const matches = findMatchOffsets(projection.text, normalizedQuery.toLowerCase());

  for (const match of matches) {
    const rangesByNode = new Map<PostNode, HighlightRange>();
    for (let index = match.start; index < match.end; index += 1) {
      const source = projection.positions[index];
      if (!source) continue;
      const range = rangesByNode.get(source.node);
      if (range) {
        range.start = Math.min(range.start, source.start);
        range.end = Math.max(range.end, source.end);
      } else {
        rangesByNode.set(source.node, { start: source.start, end: source.end });
      }
    }

    for (const [node, range] of rangesByNode) {
      const ranges = result.get(node) ?? [];
      mergeRange(ranges, range);
      result.set(node, ranges);
    }
  }

  return result;
}
