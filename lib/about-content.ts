import { readRequiredBackendJson } from "@/lib/backend";
import type { PostDocument } from "@/lib/post-content";

type AboutContentResponse = { body: PostDocument };

export async function getAboutContent(): Promise<PostDocument> {
  const { body } = await readRequiredBackendJson<AboutContentResponse>("/internal/about");
  return body;
}
