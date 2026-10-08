import { randomUUID } from "node:crypto";
import { mkdir, open, rename, unlink, type FileHandle } from "node:fs/promises";
import path from "node:path";
import {
  MAX_POST_DOCUMENT_JSON_CHARACTERS,
  normalizePostDocument,
  type PostDocument,
} from "@/lib/post-content";

const DEFAULT_REFERENCE_PATH = path.join(process.cwd(), "data", "settlement-references.json");
// JSON.stringify bounds each stored document by MAX_POST_DOCUMENT_JSON_CHARACTERS
// UTF-16 code units, each occupying at most three UTF-8 bytes; the extra bytes
// cover the settlement id and the envelope around it. Reference blocks are short
// prose, so even two dozen maximal documents are far more than the atlas stores;
// the cap keeps reads bounded, and a write that would cross it is refused instead
// of leaving behind a file readers cannot buffer.
const MAX_STORED_BYTES = 24 * (MAX_POST_DOCUMENT_JSON_CHARACTERS * 3 + 512);
// Settlement ids are opaque strings checked by the publication store; the same
// 100-character ceiling keeps stored map keys bounded here.
const MAX_SETTLEMENT_ID_LENGTH = 100;
const STORED_KEYS: Record<string, true> = { version: true, references: true };

type SettlementReferenceRuntime = {
  queue: Promise<void>;
};

const RUNTIME_KEY = Symbol.for("dimasik.settlement-reference.runtime");
const settlementReferenceGlobal = globalThis as typeof globalThis & {
  [key: symbol]: SettlementReferenceRuntime | undefined;
};
const runtime: SettlementReferenceRuntime = settlementReferenceGlobal[RUNTIME_KEY] ?? {
  queue: Promise.resolve(),
};
settlementReferenceGlobal[RUNTIME_KEY] = runtime;

function serialized<T>(operation: () => Promise<T>): Promise<T> {
  const result = runtime.queue.then(operation, operation);
  runtime.queue = result.then(
    () => undefined,
    () => undefined,
  );
  return result;
}

function referencePath() {
  const configuredPath = process.env.SETTLEMENT_REFERENCE_PATH;
  return configuredPath ? path.resolve(configuredPath) : DEFAULT_REFERENCE_PATH;
}

function isValidSettlementId(value: unknown): value is string {
  return typeof value === "string" && value.length > 0 && value.length <= MAX_SETTLEMENT_ID_LENGTH;
}

/** Reads stored entries into a map so no settlement id can reach a prototype hook. */
function parseStoredReferences(source: string): Map<string, unknown> {
  let parsed: unknown;
  try {
    parsed = JSON.parse(source);
  } catch (error) {
    throw new Error("Settlement reference storage contains invalid JSON.", { cause: error });
  }

  if (typeof parsed !== "object" || parsed === null || Array.isArray(parsed)) {
    throw new Error("Settlement reference storage is not an object.");
  }
  const candidate = parsed as Record<string, unknown>;
  if (
    candidate.version !== 1 ||
    Object.keys(candidate).length !== 2 ||
    !Object.keys(candidate).every((key) => STORED_KEYS[key] === true)
  ) {
    throw new Error("Settlement reference storage has an unsupported format.");
  }

  const references = candidate.references;
  if (typeof references !== "object" || references === null || Array.isArray(references)) {
    throw new Error("Settlement reference storage has an invalid reference map.");
  }
  const entries = Object.entries(references);
  for (const [settlementId, body] of entries) {
    if (!isValidSettlementId(settlementId)) {
      throw new Error("Settlement reference storage contains an invalid settlement id.");
    }
    if (typeof body !== "object" || body === null || Array.isArray(body)) {
      throw new Error("Settlement reference storage contains an invalid document.");
    }
  }
  return new Map(entries);
}

async function readBoundedFile(filePath: string): Promise<string> {
  const handle = await open(filePath, "r");
  try {
    const { size } = await handle.stat();
    if (!Number.isSafeInteger(size) || size > MAX_STORED_BYTES) {
      throw new Error("Settlement reference storage is too large.");
    }

    const buffer = Buffer.allocUnsafe(size + 1);
    let offset = 0;
    while (offset < buffer.length) {
      const { bytesRead } = await handle.read(buffer, offset, buffer.length - offset, offset);
      if (bytesRead === 0) break;
      offset += bytesRead;
    }
    if (offset > MAX_STORED_BYTES) {
      throw new Error("Settlement reference storage is too large.");
    }

    try {
      return new TextDecoder("utf-8", { fatal: true }).decode(buffer.subarray(0, offset));
    } catch (error) {
      throw new Error("Settlement reference storage is not valid UTF-8.", { cause: error });
    }
  } finally {
    await handle.close();
  }
}

async function readReferences(filePath: string): Promise<Map<string, unknown>> {
  try {
    return parseStoredReferences(await readBoundedFile(filePath));
  } catch (error) {
    if ((error as NodeJS.ErrnoException).code !== "ENOENT") throw error;
    // The store file is created by the first successful edit, so a read-only
    // deployment still serves every settlement without reference text.
    return new Map();
  }
}

async function writeReferences(filePath: string, references: Map<string, unknown>) {
  // Object.fromEntries defines own properties, so even a settlement id such as
  // "__proto__" is written as plain data.
  const source = `${JSON.stringify({ version: 1, references: Object.fromEntries(references) })}\n`;
  if (Buffer.byteLength(source, "utf8") > MAX_STORED_BYTES) {
    throw new Error("Settlement reference storage is too large.");
  }

  await mkdir(path.dirname(filePath), { recursive: true });
  const temporaryPath = `${filePath}.${process.pid}.${randomUUID()}.tmp`;
  let handle: FileHandle | undefined;
  try {
    handle = await open(temporaryPath, "wx", 0o600);
    await handle.writeFile(source, "utf8");
    await handle.sync();
    await handle.close();
    handle = undefined;
    await rename(temporaryPath, filePath);
  } catch (error) {
    await handle?.close().catch(() => undefined);
    await unlink(temporaryPath).catch(() => undefined);
    throw error;
  }
}

/** Reads one settlement's public reference block; null when it was never written. */
export async function getSettlementReference(settlementId: string): Promise<PostDocument | null> {
  return serialized(async () => {
    if (!isValidSettlementId(settlementId)) return null;
    const references = await readReferences(referencePath());
    const stored = references.get(settlementId);
    if (stored === undefined) return null;
    try {
      return normalizePostDocument(stored);
    } catch (error) {
      throw new Error("Settlement reference storage contains an invalid document.", {
        cause: error,
      });
    }
  });
}

/** Stores one settlement's reference block, leaving every other entry untouched. */
export async function saveSettlementReference(
  settlementId: string,
  value: unknown,
): Promise<PostDocument> {
  return serialized(async () => {
    if (!isValidSettlementId(settlementId)) {
      throw new Error("Settlement reference storage received an invalid settlement id.");
    }
    const body = normalizePostDocument(value);
    const references = await readReferences(referencePath());
    // Other settlements keep their stored documents verbatim; this save never
    // rewrites the publications file either.
    references.set(settlementId, body);
    await writeReferences(referencePath(), references);
    return body;
  });
}

/**
 * Drops one settlement's reference block after its settlement was removed,
 * leaving every other entry untouched. A missing store file or a settlement
 * without reference text is a no-op, so cleanup never creates the store.
 */
export async function removeSettlementReference(settlementId: string): Promise<void> {
  return serialized(async () => {
    if (!isValidSettlementId(settlementId)) return;
    const filePath = referencePath();
    const references = await readReferences(filePath);
    if (!references.delete(settlementId)) return;
    await writeReferences(filePath, references);
  });
}
