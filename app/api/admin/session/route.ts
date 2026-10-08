import { NextRequest, NextResponse } from "next/server";
import {
  ADMIN_COOKIE,
  SESSION_MAX_AGE,
  checkAdminCredentials,
  createAdminSession,
} from "@/lib/admin-session";

export async function POST(request: NextRequest) {
  let body: unknown;
  try {
    body = await request.json();
  } catch {
    return NextResponse.json({ error: "Неверный формат запроса." }, { status: 400 });
  }

  if (typeof body !== "object" || body === null) {
    return NextResponse.json({ error: "Неверный формат запроса." }, { status: 400 });
  }
  const { login, password } = body as Record<string, unknown>;
  if (
    typeof login !== "string" ||
    typeof password !== "string" ||
    login.length > 128 ||
    password.length > 128 ||
    !checkAdminCredentials(login, password)
  ) {
    return NextResponse.json({ error: "Неверный логин или пароль." }, { status: 401 });
  }

  try {
    const response = NextResponse.json({ authenticated: true });
    response.cookies.set(ADMIN_COOKIE, createAdminSession(), {
      httpOnly: true,
      sameSite: "strict",
      secure: request.nextUrl.protocol === "https:",
      path: "/",
      maxAge: SESSION_MAX_AGE,
    });
    response.headers.set("Cache-Control", "no-store");
    return response;
  } catch {
    return NextResponse.json({ error: "Вход временно недоступен." }, { status: 503 });
  }
}

export async function DELETE(request: NextRequest) {
  const response = NextResponse.json({ authenticated: false });
  response.cookies.set(ADMIN_COOKIE, "", {
    httpOnly: true,
    sameSite: "strict",
    secure: request.nextUrl.protocol === "https:",
    path: "/",
    maxAge: 0,
  });
  response.headers.set("Cache-Control", "no-store");
  return response;
}
