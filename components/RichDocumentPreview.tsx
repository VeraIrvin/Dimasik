"use client";

import { useId, useMemo } from "react";
import PostBody from "@/app/guberniya/[slug]/PostBody";
import { truncatePostDocument, type PostDocument } from "@/lib/post-content";
import styles from "./RichDocumentPreview.module.css";

type RichDocumentPreviewProps = {
  body: PostDocument;
  expanded: boolean;
  onExpandedChange: (expanded: boolean) => void;
};

/**
 * Format-preserving collapsed view for long rich-text documents. Content past
 * the truncation point is not mounted until the reader expands the document.
 */
export default function RichDocumentPreview({
  body,
  expanded,
  onExpandedChange,
}: RichDocumentPreviewProps) {
  const regionId = useId();
  const previewBody = useMemo(() => truncatePostDocument(body), [body]);

  // The truncation helper returns the original object when the document fits.
  if (previewBody === body) return <PostBody body={body} />;

  return (
    <div className={styles.preview}>
      <div id={regionId}>
        <PostBody body={expanded ? body : previewBody} />
      </div>
      <button
        className={styles.previewToggle}
        type="button"
        aria-expanded={expanded}
        aria-controls={regionId}
        onClick={() => onExpandedChange(!expanded)}
      >
        {expanded ? "Свернуть" : "Читать далее"}
      </button>
    </div>
  );
}
