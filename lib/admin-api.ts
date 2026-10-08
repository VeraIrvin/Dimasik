import { NextResponse, type NextRequest } from "next/server";
import { hasAdminSession } from "@/lib/admin-session";
import { GuberniaPublicationError } from "@/lib/gubernia-publications";
import { MAX_POST_DOCUMENT_JSON_CHARACTERS } from "@/lib/post-content";

const NO_STORE = { "Cache-Control": "no-store" } as const;
const BODY_TOO_LARGE_MESSAGE = "Тело запроса слишком большое.";

/**
 * JSON.stringify bounds post documents by UTF-16 code units. Any such string
 * occupies at most three UTF-8 bytes per code unit; the extra 2 KB covers the
 * validated 200-character title, request keys, and JSON escaping.
 */
export const MAX_POST_REQUEST_BYTES = MAX_POST_DOCUMENT_JSON_CHARACTERS * 3 + 2_000;

/**
 * Failures raised while guarding an admin mutation before it reaches the
 * publication store. Messages are intentionally generic so the API never
 * discloses session or filesystem details.
 */
export class AdminRequestError extends Error {
  constructor(
    message: string,
    readonly status: 400 | 401 | 403 | 415,
  ) {
    super(message);
    this.name = "AdminRequestError";
  }
}

/**
 * Authenticates cookie-based admin mutations and rejects cross-origin callers.
 * The session cookie is SameSite=strict; matching Origin against the request
 * host adds defence in depth for browser-initiated writes.
 */
export async function assertAdminMutation(request: NextRequest) {
  if (!(await hasAdminSession())) {
    throw new AdminRequestError("Требуется вход администратора.", 401);
  }

  const origin = request.headers.get("origin");
  if (origin === null) return;

  const forwardedHost = request.headers.get("x-forwarded-host")?.split(",")[0]?.trim();
  const requestHost = forwardedHost || request.headers.get("host");
  let originHost: string | undefined;
  try {
    originHost = new URL(origin).host;
  } catch {
    originHost = undefined;
  }
  if (!requestHost || originHost !== requestHost) {
    throw new AdminRequestError("Запрос отклонён.", 403);
  }
}

/**
 * Buffers at most `maxBytes` of the request body, cancelling the stream as soon
 * as the limit is crossed. The declared content length is only a fast path; the
 * stream itself is always the source of truth.
 */
async function readBoundedBody(request: NextRequest, maxBytes: number): Promise<string> {
  const declaredLength = Number(request.headers.get("content-length"));
  if (Number.isFinite(declaredLength) && declaredLength > maxBytes) {
    throw new AdminRequestError(BODY_TOO_LARGE_MESSAGE, 400);
  }

  const stream = request.body;
  if (stream === null) {
    throw new AdminRequestError("Неверный формат запроса.", 400);
  }

  const reader = stream.getReader();
  const chunks: Uint8Array[] = [];
  let total = 0;
  try {
    for (;;) {
      const { done, value } = await reader.read();
      if (done) break;
      total += value.byteLength;
      if (total > maxBytes) {
        await reader.cancel().catch(() => undefined);
        throw new AdminRequestError(BODY_TOO_LARGE_MESSAGE, 400);
      }
      chunks.push(value);
    }
  } finally {
    reader.releaseLock();
  }
  return Buffer.concat(chunks).toString("utf8");
}

/**
 * Reads a JSON object body, enforcing the JSON content type first so plain
 * cross-origin form posts can never reach the handlers. DELETE requests carry
 * no body and therefore never call this helper.
 *
 * Passing `maxBytes` switches to a streaming read that rejects oversized bodies
 * before they are buffered or parsed; without it the body is read as before.
 */
export async function readJsonObject(
  request: NextRequest,
  maxBytes?: number,
): Promise<Record<string, unknown>> {
  const contentType = request.headers.get("content-type");
  if (!contentType || !/^application\/json\s*(;|$)/i.test(contentType.trim())) {
    throw new AdminRequestError("Ожидается JSON-запрос.", 415);
  }

  let body: unknown;
  if (maxBytes === undefined) {
    try {
      body = await request.json();
    } catch {
      throw new AdminRequestError("Неверный формат запроса.", 400);
    }
  } else {
    let source: string;
    try {
      source = await readBoundedBody(request, maxBytes);
    } catch (error) {
      if (error instanceof AdminRequestError) throw error;
      throw new AdminRequestError("Неверный формат запроса.", 400);
    }
    try {
      body = JSON.parse(source);
    } catch {
      throw new AdminRequestError("Неверный формат запроса.", 400);
    }
  }
  if (typeof body !== "object" || body === null || Array.isArray(body)) {
    throw new AdminRequestError("Неверный формат запроса.", 400);
  }
  return body as Record<string, unknown>;
}

export function apiErrorResponse(error: unknown) {
  if (error instanceof AdminRequestError || error instanceof GuberniaPublicationError) {
    return NextResponse.json(
      { error: error.message },
      { status: error.status, headers: NO_STORE },
    );
  }
  console.error("Gubernia API failure:", error);
  return NextResponse.json(
    { error: "Внутренняя ошибка сервера." },
    { status: 500, headers: NO_STORE },
  );
}
