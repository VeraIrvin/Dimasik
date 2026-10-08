import { NextResponse, type NextRequest } from "next/server";
import { apiErrorResponse, assertAdminMutation, readJsonObject } from "@/lib/admin-api";
import { unpublishGubernia, updateGuberniaPublication } from "@/lib/gubernia-publications";

export const dynamic = "force-dynamic";

type RouteParams = { params: Promise<{ id: string }> };

export async function PATCH(request: NextRequest, { params }: RouteParams) {
  try {
    await assertAdminMutation(request);
    const { id } = await params;
    const body = await readJsonObject(request);

    const gubernia = await updateGuberniaPublication(id, body.slug, body.description);
    return NextResponse.json(gubernia, { headers: { "Cache-Control": "no-store" } });
  } catch (error) {
    return apiErrorResponse(error);
  }
}

export async function DELETE(request: NextRequest, { params }: RouteParams) {
  try {
    await assertAdminMutation(request);
    const { id } = await params;

    await unpublishGubernia(id);
    return new NextResponse(null, { status: 204, headers: { "Cache-Control": "no-store" } });
  } catch (error) {
    return apiErrorResponse(error);
  }
}
