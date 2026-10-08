import { NextResponse, type NextRequest } from "next/server";
import { AdminRequestError, apiErrorResponse, assertAdminMutation, readJsonObject } from "@/lib/admin-api";
import { getGuberniasCollection, publishGubernia } from "@/lib/gubernia-publications";

export const dynamic = "force-dynamic";

// Publication state lives on disk, so every read must bypass caches.
export async function GET() {
  try {
    const collection = await getGuberniasCollection();
    return NextResponse.json(collection, { headers: { "Cache-Control": "no-store" } });
  } catch (error) {
    return apiErrorResponse(error);
  }
}

export async function POST(request: NextRequest) {
  try {
    await assertAdminMutation(request);
    const body = await readJsonObject(request);
    const id = body.id;
    if (typeof id !== "string" || id.length === 0 || id.length > 100) {
      throw new AdminRequestError("Некорректный идентификатор губернии.", 400);
    }

    const gubernia = await publishGubernia(id, body.slug);
    return NextResponse.json(gubernia, {
      status: 201,
      headers: { "Cache-Control": "no-store" },
    });
  } catch (error) {
    return apiErrorResponse(error);
  }
}
