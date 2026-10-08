import { NextResponse, type NextRequest } from "next/server";
import {
  AdminRequestError,
  MAX_POST_REQUEST_BYTES,
  apiErrorResponse,
  assertAdminMutation,
  readJsonObject,
} from "@/lib/admin-api";
import { GuberniaPublicationError, getPublishedSettlementBySlug } from "@/lib/gubernia-publications";
import { PostContentValidationError } from "@/lib/post-content";
import { saveSettlementReference } from "@/lib/settlement-reference";

export const dynamic = "force-dynamic";

const REQUEST_KEYS: Record<string, true> = { body: true };

type RouteParams = { params: Promise<{ slug: string }> };

/**
 * Replaces one published settlement's public reference text. Unpublished and
 * unknown settlements are invisible here just as they are on their page.
 */
export async function PATCH(request: NextRequest, { params }: RouteParams) {
  try {
    await assertAdminMutation(request);
    const { slug } = await params;
    const payload = await readJsonObject(request, MAX_POST_REQUEST_BYTES);
    const keys = Object.keys(payload);
    if (keys.length !== 1 || REQUEST_KEYS[keys[0]] !== true) {
      throw new AdminRequestError("Ожидается документ содержимого страницы.", 400);
    }

    const publication = await getPublishedSettlementBySlug(slug);
    if (!publication) {
      throw new GuberniaPublicationError("Населённый пункт не найден.", 404);
    }

    const body = await saveSettlementReference(publication.settlement.id, payload.body);
    return NextResponse.json({ body }, { headers: { "Cache-Control": "no-store" } });
  } catch (error) {
    if (error instanceof PostContentValidationError) {
      return apiErrorResponse(new AdminRequestError(error.message, 400));
    }
    return apiErrorResponse(error);
  }
}
