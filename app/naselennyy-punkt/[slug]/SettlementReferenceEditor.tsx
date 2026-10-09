"use client";

import { RichPageContentEditor } from "@/app/o-proekte/AboutContentEditor";
import type { PostImage } from "@/lib/gubernia-publications";
import type { PostDocument } from "@/lib/post-content";
import SettlementDeleteButton from "./SettlementDeleteButton";
import styles from "./SettlementReferenceEditor.module.css";

type SettlementReferenceEditorProps = {
  slug: string;
  initialBody: PostDocument | null;
  /** Saved reference images in display order; empty when there are none. */
  initialImages: PostImage[];
  isAdmin: boolean;
  settlementId: string;
  guberniaId: string;
  settlementName: string;
  backHref: string;
  /** Element id of the page slot that hosts the admin row, above the details. */
  adminSlotId: string;
};

/**
 * Public reference block of a settlement page. Every visitor sees a truncated
 * rich-text prefix of the stored text with a «Читать далее» disclosure that
 * reveals the full document; a short note replaces the block only when neither
 * text nor images are stored, and images render as a gallery above the text.
 * Only the admin gets the shared rich-text editor (always the whole document)
 * plus the adjacent settlement removal action. The admin row itself renders in
 * the record's header slot, so it sits under the page title, before the Уезд
 * row, while the editor stays with the text.
 */
export default function SettlementReferenceEditor({
  slug,
  initialBody,
  initialImages,
  isAdmin,
  settlementId,
  guberniaId,
  settlementName,
  backHref,
  adminSlotId,
}: SettlementReferenceEditorProps) {
  return (
    <RichPageContentEditor
      initialBody={initialBody}
      initialImages={initialImages}
      imageSupport
      imagesAriaLabel="Изображения справки"
      isAdmin={isAdmin}
      endpoint={`/api/naselennyy-punkt/${encodeURIComponent(slug)}/reference`}
      ariaLabel="Справочная информация"
      editorLabel="Текст справки"
      editLabel="Редактировать населённый пункт"
      adminActionsTargetId={adminSlotId}
      collapsedPreview
      emptyState={<p className={styles.emptyState}>Справочная информация пока не добавлена.</p>}
      emptyValidationMessage="Добавьте текст справки."
      saveErrorMessage="Не удалось сохранить справочные сведения. Попробуйте ещё раз."
      secondaryAction={
        <SettlementDeleteButton
          settlementId={settlementId}
          guberniaId={guberniaId}
          settlementName={settlementName}
          backHref={backHref}
        />
      }
    />
  );
}
