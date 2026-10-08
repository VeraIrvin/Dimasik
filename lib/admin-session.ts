import { readRequiredBackendJson } from "@/lib/backend";

type SessionResponse = { isAdmin: boolean };

/** True when the Rust backend accepts this request's admin session cookie. */
export async function hasAdminSession(): Promise<boolean> {
  const { isAdmin } = await readRequiredBackendJson<SessionResponse>("/internal/session");
  return isAdmin === true;
}
