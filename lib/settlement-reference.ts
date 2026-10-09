import { readBackendJson } from "@/lib/backend";
import type { PostImage } from "@/lib/gubernia-publications";
import type { PostDocument } from "@/lib/post-content";

export type SettlementReference = {
  body: PostDocument | null;
  images: PostImage[];
};

/** Reads one settlement's public reference block and its ordered image gallery. */
export async function getSettlementReference(settlementId: string): Promise<SettlementReference> {
  const response = await readBackendJson<SettlementReference>(
    `/internal/settlement-reference/${encodeURIComponent(settlementId)}`,
  );
  return {
    body: response?.body ?? null,
    images: Array.isArray(response?.images) ? response.images : [],
  };
}
