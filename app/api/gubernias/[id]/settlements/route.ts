import { NextResponse, type NextRequest } from "next/server";
import { apiErrorResponse, assertAdminMutation, readJsonObject } from "@/lib/admin-api";
import { createSettlement } from "@/lib/gubernia-publications";

export const dynamic = "force-dynamic";

/** Name, district id, type, and a "latitude, longitude" string: well under 4 KB. */
const SETTLEMENT_REQUEST_BYTES = 4_096;

type RouteParams = { params: Promise<{ id: string }> };

export async function POST(request: NextRequest, { params }: RouteParams) {
  try {
    await assertAdminMutation(request);
    const { id } = await params;
    const body = await readJsonObject(request, SETTLEMENT_REQUEST_BYTES);

    const settlement = await createSettlement(
      id,
      body.name,
      body.uyezdId,
      body.coordinates,
      body.url,
      body.type,
    );
    return NextResponse.json(settlement, {
      status: 201,
      headers: { "Cache-Control": "no-store" },
    });
  } catch (error) {
    return apiErrorResponse(error);
  }
}
