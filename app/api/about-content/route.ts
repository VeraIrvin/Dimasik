import { NextResponse, type NextRequest } from "next/server";
import {
  AdminRequestError,
  MAX_POST_REQUEST_BYTES,
  apiErrorResponse,
  assertAdminMutation,
  readJsonObject,
} from "@/lib/admin-api";
import { saveAboutContent } from "@/lib/about-content";
import { PostContentValidationError } from "@/lib/post-content";

export const dynamic = "force-dynamic";

const REQUEST_KEYS: Record<string, true> = { body: true };

/** Replaces the editable About page body; the page header and credits stay code-owned. */
export async function PATCH(request: NextRequest) {
  try {
    await assertAdminMutation(request);
    const payload = await readJsonObject(request, MAX_POST_REQUEST_BYTES);
    const keys = Object.keys(payload);
    if (keys.length !== 1 || REQUEST_KEYS[keys[0]] !== true) {
      throw new AdminRequestError("Ожидается документ содержимого страницы.", 400);
    }

    const body = await saveAboutContent(payload.body);
    return NextResponse.json({ body }, { headers: { "Cache-Control": "no-store" } });
  } catch (error) {
    if (error instanceof PostContentValidationError) {
      return apiErrorResponse(new AdminRequestError(error.message, 400));
    }
    return apiErrorResponse(error);
  }
}
