import { NextResponse, type NextRequest } from "next/server";
import { apiErrorResponse, assertAdminMutation } from "@/lib/admin-api";
import { deleteSettlement } from "@/lib/gubernia-publications";
import { removeSettlementReference } from "@/lib/settlement-reference";

export const dynamic = "force-dynamic";

type RouteParams = { params: Promise<{ id: string; settlementId: string }> };

export async function DELETE(request: NextRequest, { params }: RouteParams) {
  try {
    await assertAdminMutation(request);
    const { id, settlementId } = await params;

    await deleteSettlement(id, settlementId);
    // The publication is already gone by the time the reference is cleaned up,
    // so a stale reference block must not turn a successful removal into a
    // failure the administration would retry: log it and report success.
    try {
      await removeSettlementReference(settlementId);
    } catch (error) {
      console.error("Settlement reference cleanup failed:", error);
    }
    return new NextResponse(null, { status: 204, headers: { "Cache-Control": "no-store" } });
  } catch (error) {
    return apiErrorResponse(error);
  }
}
