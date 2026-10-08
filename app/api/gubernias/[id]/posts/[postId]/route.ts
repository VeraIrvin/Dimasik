import { NextResponse, type NextRequest } from "next/server";
import {
  MAX_POST_REQUEST_BYTES,
  apiErrorResponse,
  assertAdminMutation,
  readJsonObject,
} from "@/lib/admin-api";
import { deleteGuberniaPost, updateGuberniaPost } from "@/lib/gubernia-publications";

export const dynamic = "force-dynamic";

/**
 * Placement metadata (ids, year, archive reference, target province) rides
 * beside the post document, so the post request cap gains a matching bound.
 */
const POST_REQUEST_BYTES = MAX_POST_REQUEST_BYTES + 8_192;

type RouteParams = { params: Promise<{ id: string; postId: string }> };

export async function PATCH(request: NextRequest, { params }: RouteParams) {
  try {
    await assertAdminMutation(request);
    const { id, postId } = await params;
    const body = await readJsonObject(request, POST_REQUEST_BYTES);

    const post = await updateGuberniaPost(id, postId, body.title, body.body, {
      category: body.category,
      uyezdId: body.uyezdId,
      settlementId: body.settlementId,
      year: body.year,
      archiveReference: body.archiveReference,
      targetGuberniaId: body.targetGuberniaId,
    });
    return NextResponse.json(post, { headers: { "Cache-Control": "no-store" } });
  } catch (error) {
    return apiErrorResponse(error);
  }
}

export async function DELETE(request: NextRequest, { params }: RouteParams) {
  try {
    await assertAdminMutation(request);
    const { id, postId } = await params;

    await deleteGuberniaPost(id, postId);
    return new NextResponse(null, { status: 204, headers: { "Cache-Control": "no-store" } });
  } catch (error) {
    return apiErrorResponse(error);
  }
}
