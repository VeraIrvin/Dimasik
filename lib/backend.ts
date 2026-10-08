import { cookies } from "next/headers";

const DEFAULT_BACKEND_URL = "http://127.0.0.1:8787";

/**
 * Raised when the Rust backend is unreachable or answers with a failure other
 * than the one a caller explicitly tolerates. Outages must never be mistaken
 * for missing content, so only {@link readBackendJson} maps 404 to null.
 */
export class BackendRequestError extends Error {
  constructor(
    message: string,
    readonly status: number,
  ) {
    super(message);
    this.name = "BackendRequestError";
  }
}

/**
 * Reads one server-rendered document from the Rust backend. The request's own
 * cookies ride along so Rust can authorize admin-only endpoints, and every
 * call skips caches so a reload never serves stale publication state.
 */
async function requestBackend(path: string): Promise<Response> {
  const configured = process.env.BACKEND_URL?.trim();
  const baseUrl = (configured ? configured : DEFAULT_BACKEND_URL).replace(/\/+$/u, "");
  const cookieHeader = (await cookies())
    .getAll()
    .map(({ name, value }) => `${name}=${value}`)
    .join("; ");

  try {
    return await fetch(`${baseUrl}${path}`, {
      cache: "no-store",
      headers: {
        accept: "application/json",
        ...(cookieHeader.length > 0 ? { cookie: cookieHeader } : {}),
      },
    });
  } catch {
    throw new BackendRequestError("Сервис публикаций недоступен.", 503);
  }
}

/** Reads a backend document; a 404 resolves to null, every other failure throws. */
export async function readBackendJson<T>(path: string): Promise<T | null> {
  const response = await requestBackend(path);
  if (response.status === 404) return null;
  if (!response.ok) {
    throw new BackendRequestError(
      `Сервис публикаций ответил ошибкой ${response.status}.`,
      response.status,
    );
  }

  try {
    return (await response.json()) as T;
  } catch {
    throw new BackendRequestError("Сервис публикаций вернул некорректный ответ.", 502);
  }
}

/** Reads a backend document the backend must always serve. */
export async function readRequiredBackendJson<T>(path: string): Promise<T> {
  const document = await readBackendJson<T>(path);
  if (document === null) {
    throw new BackendRequestError(`Документ «${path}» не найден.`, 404);
  }
  return document;
}
