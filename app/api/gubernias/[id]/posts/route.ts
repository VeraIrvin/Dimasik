import { NextResponse, type NextRequest } from "next/server";
import {
  MAX_POST_REQUEST_BYTES,
  apiErrorResponse,
  assertAdminMutation,
  readJsonObject,
} from "@/lib/admin-api";
import { createGuberniaPost } from "@/lib/gubernia-publications";

export const dynamic = "force-dynamic";

/**
 * Placement metadata (ids, year, archive reference) rides beside the post
 * document, so the post request cap gains a matching bound for those fields.
 */
const POST_REQUEST_BYTES = MAX_POST_REQUEST_BYTES + 8_192;

type RouteParams = { params: Promise<{ id: string }> };

export async function POST(request: NextRequest, { params }: RouteParams) {
  try {
    await assertAdminMutation(request);
    const { id } = await params;
    const body = await readJsonObject(request, POST_REQUEST_BYTES);

    const post = await createGuberniaPost(id, body.title, body.body, {
      category: body.category,
      uyezdId: body.uyezdId,
      settlementId: body.settlementId,
      year: body.year,
      archiveReference: body.archiveReference,
    });
    return NextResponse.json(post, {
      status: 201,
      headers: { "Cache-Control": "no-store" },
    });
  } catch (error) {
    return apiErrorResponse(error);
  }
}
