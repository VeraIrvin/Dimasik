import { readBackendJson } from "@/lib/backend";
import type { PostDocument } from "@/lib/post-content";

type SettlementReferenceResponse = { body: PostDocument | null };

/** Reads one settlement's public reference block; null when it was never written. */
export async function getSettlementReference(settlementId: string): Promise<PostDocument | null> {
  const response = await readBackendJson<SettlementReferenceResponse>(
    `/internal/settlement-reference/${encodeURIComponent(settlementId)}`,
  );
  return response?.body ?? null;
}
