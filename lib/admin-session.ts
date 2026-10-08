import { createHash, createHmac, timingSafeEqual } from "node:crypto";
import { cookies } from "next/headers";

export const ADMIN_COOKIE = "dimasik_admin_session";
export const SESSION_MAX_AGE = 60 * 60 * 8;

function configuredAdminCredentials() {
  const username = process.env.ADMIN_USERNAME;
  const password = process.env.ADMIN_PASSWORD;
  if (!username || !password) return null;
  return { username, password };
}

export function checkAdminCredentials(username: string, password: string) {
  const configured = configuredAdminCredentials();
  if (!configured) return false;

  const usernameMatches = timingSafeEqual(
    createHash("sha256").update(username).digest(),
    createHash("sha256").update(configured.username).digest(),
  );
  const passwordMatches = timingSafeEqual(
    createHash("sha256").update(password).digest(),
    createHash("sha256").update(configured.password).digest(),
  );
  return usernameMatches && passwordMatches;
}

function signingSecret() {
  const secret = process.env.ADMIN_SESSION_SECRET;
  if (!secret || secret.length < 32) {
    throw new Error("ADMIN_SESSION_SECRET must be at least 32 characters long.");
  }
  return secret;
}

function signature(payload: string) {
  return createHmac("sha256", signingSecret()).update(payload).digest("base64url");
}

export function createAdminSession() {
  const expires = Math.floor(Date.now() / 1000) + SESSION_MAX_AGE;
  const payload = `admin.${expires}`;
  return `${payload}.${signature(payload)}`;
}

export function verifyAdminSession(value: string | undefined) {
  if (!value) return false;
  const parts = value.split(".");
  if (parts.length !== 3 || parts[0] !== "admin" || !/^\d+$/.test(parts[1])) return false;
  const expires = Number(parts[1]);
  const now = Math.floor(Date.now() / 1000);
  if (!Number.isSafeInteger(expires) || expires <= now || expires > now + SESSION_MAX_AGE) {
    return false;
  }
  try {
    const expected = Buffer.from(signature(`${parts[0]}.${parts[1]}`), "base64url");
    const actual = Buffer.from(parts[2], "base64url");
    return expected.length === actual.length && timingSafeEqual(expected, actual);
  } catch {
    return false;
  }
}

export async function hasAdminSession() {
  const cookie = (await cookies()).get(ADMIN_COOKIE)?.value;
  return verifyAdminSession(cookie);
}
