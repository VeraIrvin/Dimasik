"use client";

import { RichPageContentEditor } from "@/app/o-proekte/AboutContentEditor";
import type { PostDocument } from "@/lib/post-content";
import SettlementDeleteButton from "./SettlementDeleteButton";
import styles from "./SettlementReferenceEditor.module.css";

type SettlementReferenceEditorProps = {
  slug: string;
  initialBody: PostDocument | null;
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
 * reveals the full document; a short note replaces it when nothing is stored.
 * Only the admin gets the shared rich-text editor (always the whole document)
 * plus the adjacent settlement removal action. The admin row itself renders in
 * the record's header slot, so it sits under the page title, before the Уезд
 * row, while the editor stays with the text.
 */
export default function SettlementReferenceEditor({
  slug,
  initialBody,
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
