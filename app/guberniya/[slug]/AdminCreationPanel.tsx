"use client";

import { useId, useState } from "react";
import type { DistrictOption, PublishedOption, Settlement } from "@/lib/gubernia-publications";
import ProvincePostEditor from "./ProvincePostEditor";
import SettlementEditor from "./SettlementEditor";
import styles from "./AdminCreationPanel.module.css";

type AdminCreationPanelProps = {
  guberniaId: string;
  provinces: PublishedOption[];
  districts: DistrictOption[];
  settlements: Settlement[];
  categories: string[];
  settlementTypes: string[];
  /** Preselects the settlement for a new post, e.g. on a settlement page. */
  defaultSettlementId?: string;
  /** Preselects a district for new posts and settlements when no settlement default applies. */
  defaultUyezdId?: string;
};

type ActiveEditor = "post" | "settlement";

export default function AdminCreationPanel({
  guberniaId,
  provinces,
  districts,
  settlements,
  categories,
  settlementTypes,
  defaultSettlementId,
  defaultUyezdId,
}: AdminCreationPanelProps) {
  const editorPanelId = useId();
  const [active, setActive] = useState<ActiveEditor | null>(null);

  function toggle(editor: ActiveEditor) {
    // One slot: opening an editor closes the other, clicking the open one closes it.
    setActive((previous) => (previous === editor ? null : editor));
  }

  return (
    <section className={styles.panel} aria-label="Добавление материалов">
      <div className={styles.buttons}>
        <button
          className={styles.creationButton}
          type="button"
          aria-expanded={active === "post"}
          aria-controls={editorPanelId}
          onClick={() => toggle("post")}
        >
          НОВАЯ ЗАПИСЬ
        </button>
        <button
          className={styles.creationButton}
          type="button"
          aria-expanded={active === "settlement"}
          aria-controls={editorPanelId}
          onClick={() => toggle("settlement")}
        >
          НОВЫЙ НАСЕЛЁННЫЙ ПУНКТ
        </button>
      </div>

      <div id={editorPanelId} className={styles.editor}>
        {active === "post" ? (
          <ProvincePostEditor
            guberniaId={guberniaId}
            provinces={provinces}
            districts={districts}
            settlements={settlements}
            categories={categories}
            defaultSettlementId={defaultSettlementId}
            defaultUyezdId={defaultUyezdId}
            onDone={() => setActive(null)}
          />
        ) : active === "settlement" ? (
          <SettlementEditor
            guberniaId={guberniaId}
            provinces={provinces}
            districts={districts}
            settlementTypes={settlementTypes}
            defaultUyezdId={defaultUyezdId}
            onDone={() => setActive(null)}
          />
        ) : null}
      </div>
    </section>
  );
}
