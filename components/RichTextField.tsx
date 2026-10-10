"use client";

import {
  useCallback,
  useEffect,
  useId,
  useImperativeHandle,
  useRef,
  useState,
  type ReactNode,
  type Ref,
} from "react";
import { Color, TextStyle } from "@tiptap/extension-text-style";
import { Link } from "@tiptap/extension-link";
import { OrderedList } from "@tiptap/extension-list";
import { TextAlign } from "@tiptap/extension-text-align";
import { TableKit } from "@tiptap/extension-table";
import { cellAround, isInTable, selectedRect } from "@tiptap/pm/tables";
import { EditorContent, useEditor, useEditorState, type Editor } from "@tiptap/react";
import { StarterKit } from "@tiptap/starter-kit";
import type { PostDocument } from "@/lib/post-content";
import styles from "./RichTextField.module.css";

const MAX_TABLE_ROWS = 100;
const MAX_TABLE_COLUMNS = 100;
const DEFAULT_COLOR = "#272622";
const ALIGNMENTS = ["left", "center", "right", "justify"] as const;
type Alignment = (typeof ALIGNMENTS)[number];

const ALIGNMENT_LABELS: Record<Alignment, string> = {
  left: "По левому краю",
  center: "По центру",
  right: "По правому краю",
  justify: "По ширине",
};

/**
 * Mirrors the server rule for links: http(s) addresses and same-origin paths
 * survive, everything else (javascript:, mailto:, //host) is rejected.
 */
function normalizeHref(raw: string): string | null {
  const href = raw.trim();
  if (!href || href.startsWith("//")) return null;
  if (href.startsWith("/")) return href;
  if (/^https?:\/\//i.test(href)) return href;
  // A missing scheme is fine: it is completed with the editor's default one.
  if (/^[a-z][a-z0-9+.-]*:/i.test(href)) return null;
  return `https://${href}`;
}

/** `<input type="color">` only accepts the six-digit form. */
function toColorInputValue(color: string): string {
  const short = /^#([0-9a-f]{3})$/i.exec(color);
  if (short) {
    return `#${short[1].replace(/./g, (digit) => digit + digit)}`.toLowerCase();
  }
  return /^#[0-9a-f]{6}$/i.test(color) ? color.toLowerCase() : DEFAULT_COLOR;
}

/**
 * Links keep only `href`: the server drops every other link attribute, and
 * pasted anchors often carry `target`, `rel` or `title` along with them.
 */
const SafeLink = Link.extend({
  addAttributes() {
    return {
      href: {
        default: null,
        parseHTML: (element: HTMLElement) => {
          const href = element.getAttribute("href");
          return href ? normalizeHref(href) : null;
        },
      },
    };
  },
});

/**
 * Ordered lists keep only `start`: the server whitelist drops the marker
 * `type` that lists pasted from Word or Google Docs routinely carry.
 */
const SafeOrderedList = OrderedList.extend({
  addAttributes() {
    return {
      start: {
        default: 1,
        parseHTML: (element: HTMLElement) => {
          const start = Number.parseInt(element.getAttribute("start") ?? "", 10);
          return Number.isFinite(start) ? start : 1;
        },
      },
    };
  },
});

/** The server whitelist decides what may be stored, so the editor offers no more. */
const EDITOR_EXTENSIONS = [
  StarterKit.configure({
    heading: { levels: [2, 3] },
    code: false,
    codeBlock: false,
    horizontalRule: false,
    link: false,
    orderedList: false,
  }),
  SafeLink.configure({
    openOnClick: false,
    autolink: true,
    linkOnPaste: true,
    defaultProtocol: "https",
    // The library also calls this with a missing href while rendering.
    isAllowedUri: (url: unknown) => typeof url === "string" && normalizeHref(url) !== null,
    // An email would be linkified as `mailto:`, which the server rejects, so the
    // only autolinked hrefs are the ones this editor is allowed to store.
    shouldAutoLink: (url: string) =>
      normalizeHref(url) !== null && (!url.includes("@") || /^[a-z][a-z0-9+.-]*:\/\//i.test(url)),
  }),
  TextStyle,
  Color,
  SafeOrderedList,
  TableKit.configure({
    table: { resizable: false, renderWrapper: true, cellMinWidth: 100 },
  }),
  TextAlign.configure({ types: ["heading", "paragraph"] }),
];

/** Creates the shared rich-text editor: a PostDocument in, a PostDocument out. */
export function usePostDocumentEditor(content: PostDocument | "", ariaLabel: string) {
  return useEditor({
    extensions: EDITOR_EXTENSIONS,
    content,
    immediatelyRender: false,
    editorProps: {
      attributes: {
        "aria-label": ariaLabel,
        "aria-multiline": "true",
      },
    },
  });
}

type ToolbarState = {
  bold: boolean;
  italic: boolean;
  underline: boolean;
  strike: boolean;
  paragraph: boolean;
  heading2: boolean;
  heading3: boolean;
  bulletList: boolean;
  orderedList: boolean;
  blockquote: boolean;
  link: boolean;
  table: boolean;
  canInsertTable: boolean;
  canAddRow: boolean;
  canDeleteRow: boolean;
  canAddColumn: boolean;
  canDeleteColumn: boolean;
  canDeleteTable: boolean;
  alignment: Alignment;
  color: string | null;
  canUndo: boolean;
  canRedo: boolean;
};

const EMPTY_TOOLBAR_STATE: ToolbarState = {
  bold: false,
  italic: false,
  underline: false,
  strike: false,
  paragraph: false,
  heading2: false,
  heading3: false,
  bulletList: false,
  orderedList: false,
  blockquote: false,
  link: false,
  table: false,
  canInsertTable: false,
  canAddRow: false,
  canDeleteRow: false,
  canAddColumn: false,
  canDeleteColumn: false,
  canDeleteTable: false,
  alignment: "left",
  color: null,
  canUndo: false,
  canRedo: false,
};

function readToolbarState(editor: Editor | null): ToolbarState {
  if (!editor) return EMPTY_TOOLBAR_STATE;
  const color = editor.getAttributes("textStyle").color;
  const tableRect = isInTable(editor.state) ? selectedRect(editor.state) : null;
  const cellAlign = cellAround(editor.state.selection.$head)?.nodeAfter?.attrs.align;
  const inheritedAlignment = cellAlign === "center" || cellAlign === "right" ? cellAlign : "left";
  return {
    bold: editor.isActive("bold"),
    italic: editor.isActive("italic"),
    underline: editor.isActive("underline"),
    strike: editor.isActive("strike"),
    paragraph: editor.isActive("paragraph"),
    heading2: editor.isActive("heading", { level: 2 }),
    heading3: editor.isActive("heading", { level: 3 }),
    bulletList: editor.isActive("bulletList"),
    orderedList: editor.isActive("orderedList"),
    blockquote: editor.isActive("blockquote"),
    link: editor.isActive("link"),
    table: editor.isActive("table"),
    canInsertTable: editor.isEditable && editor.can().insertTable({ rows: 3, cols: 3, withHeaderRow: true }),
    canAddRow: editor.isEditable && tableRect !== null && tableRect.map.height < MAX_TABLE_ROWS && editor.can().addRowAfter(),
    // The library's can() check omits its dispatched all-rows/all-columns guard.
    canDeleteRow: editor.isEditable && tableRect !== null &&
      (tableRect.top > 0 || tableRect.bottom < tableRect.map.height) && editor.can().deleteRow(),
    canAddColumn: editor.isEditable && tableRect !== null && tableRect.map.width < MAX_TABLE_COLUMNS && editor.can().addColumnAfter(),
    canDeleteColumn: editor.isEditable && tableRect !== null &&
      (tableRect.left > 0 || tableRect.right < tableRect.map.width) && editor.can().deleteColumn(),
    canDeleteTable: editor.isEditable && editor.can().deleteTable(),
    alignment: ALIGNMENTS.find((alignment) => editor.isActive({ textAlign: alignment })) ?? inheritedAlignment,
    color: typeof color === "string" ? color : null,
    canUndo: editor.can().undo(),
    canRedo: editor.can().redo(),
  };
}

type ToolbarButtonProps = {
  label: string;
  onClick: () => void;
  pressed?: boolean;
  expanded?: boolean;
  disabled?: boolean;
  children: ReactNode;
};

function ToolbarButton({ label, onClick, pressed, expanded, disabled, children }: ToolbarButtonProps) {
  return (
    <button
      type="button"
      className={styles.toolbarButton}
      aria-label={label}
      title={label}
      aria-pressed={pressed}
      aria-expanded={expanded}
      disabled={disabled}
      // Keeping the mousedown away from the button preserves the selection.
      onMouseDown={(event) => event.preventDefault()}
      onClick={onClick}
    >
      {children}
    </button>
  );
}

/** Imperative escape hatch for forms that own the surrounding Escape handling. */
export type RichTextFieldHandle = {
  isLinkFieldOpen: () => boolean;
  closeLinkField: () => void;
};

type RichTextFieldProps = {
  editor: Editor | null;
  /** Keyboard hint rendered under the editable area. */
  hint: string;
  ref?: Ref<RichTextFieldHandle>;
};

export default function RichTextField({ editor, hint, ref }: RichTextFieldProps) {
  const alignmentId = useId();
  const colorId = useId();
  const linkFieldId = useId();
  const linkErrorId = useId();
  const [linkOpen, setLinkOpen] = useState(false);
  const [linkValue, setLinkValue] = useState("");
  const [linkError, setLinkError] = useState("");
  const linkInputRef = useRef<HTMLInputElement>(null);
  const stickyControlsRef = useRef<HTMLDivElement>(null);
  const toolbarState =
    useEditorState({ editor, selector: ({ editor: current }) => readToolbarState(current) }) ??
    EMPTY_TOOLBAR_STATE;

  useEffect(() => {
    if (!linkOpen) return;
    const input = linkInputRef.current;
    const controls = stickyControlsRef.current;
    if (!input || !controls) return;

    // Reveal the input inside the bounded controls, without scrolling the page
    // back to the editor's original toolbar position.
    input.focus({ preventScroll: true });
    const revealFocusedInput = () => {
      if (document.activeElement !== input) return;
      const inputRect = input.getBoundingClientRect();
      const controlsRect = controls.getBoundingClientRect();
      if (inputRect.bottom > controlsRect.bottom) {
        controls.scrollTop += inputRect.bottom - controlsRect.bottom + 8;
      } else if (inputRect.top < controlsRect.top) {
        controls.scrollTop -= controlsRect.top - inputRect.top + 8;
      }
    };
    revealFocusedInput();
    const observer = new ResizeObserver(revealFocusedInput);
    observer.observe(controls, { box: "border-box" });
    return () => observer.disconnect();
  }, [linkOpen]);

  const closeLinkField = useCallback(() => {
    setLinkOpen(false);
    setLinkError("");
    setLinkValue("");
    editor?.chain().focus().run();
  }, [editor]);

  useImperativeHandle(
    ref,
    () => ({ isLinkFieldOpen: () => linkOpen, closeLinkField }),
    [closeLinkField, linkOpen],
  );

  function applyAlignment(alignment: Alignment) {
    if (!editor) return;
    const chain = editor.chain().focus();
    // Outside tables, plain left-aligned text stores no attribute. Inside tables,
    // an explicit left alignment overrides alignment inherited from pasted cells.
    if (alignment === "left" && !isInTable(editor.state)) {
      chain.unsetTextAlign().run();
      return;
    }
    chain.setTextAlign(alignment).run();
  }

  function openLinkField() {
    if (!editor) return;
    const attributes = editor.getAttributes("link");
    setLinkValue(typeof attributes.href === "string" ? attributes.href : "");
    setLinkError("");
    setLinkOpen(true);
  }

  function applyLink() {
    if (!editor) return;
    const href = normalizeHref(linkValue);
    if (!href) {
      setLinkError("Укажите адрес вида https://пример.ру или /put.");
      linkInputRef.current?.focus();
      return;
    }
    editor.chain().focus().extendMarkRange("link").setLink({ href }).run();
    closeLinkField();
  }

  function removeLink() {
    if (!editor) return;
    editor.chain().focus().extendMarkRange("link").unsetLink().run();
    closeLinkField();
  }

  return (
    <div className={styles.editor}>
      <div ref={stickyControlsRef} className={styles.stickyControls}>
      <div className={styles.toolbar} role="toolbar" aria-label="Форматирование текста">
        <div className={styles.toolbarGroup} role="group" aria-label="Начертание">
          <ToolbarButton
            label="Полужирный"
            pressed={toolbarState.bold}
            disabled={!editor}
            onClick={() => editor?.chain().focus().toggleBold().run()}
          >
            <span className={styles.glyphBold}>Ж</span>
          </ToolbarButton>
          <ToolbarButton
            label="Курсив"
            pressed={toolbarState.italic}
            disabled={!editor}
            onClick={() => editor?.chain().focus().toggleItalic().run()}
          >
            <span className={styles.glyphItalic}>К</span>
          </ToolbarButton>
          <ToolbarButton
            label="Подчёркнутый"
            pressed={toolbarState.underline}
            disabled={!editor}
            onClick={() => editor?.chain().focus().toggleUnderline().run()}
          >
            <span className={styles.glyphUnderline}>Ч</span>
          </ToolbarButton>
          <ToolbarButton
            label="Зачёркнутый"
            pressed={toolbarState.strike}
            disabled={!editor}
            onClick={() => editor?.chain().focus().toggleStrike().run()}
          >
            <span className={styles.glyphStrike}>З</span>
          </ToolbarButton>
        </div>

        <div className={styles.toolbarGroup} role="group" aria-label="Блоки текста">
          <ToolbarButton
            label="Обычный текст"
            pressed={toolbarState.paragraph}
            disabled={!editor}
            onClick={() => editor?.chain().focus().setParagraph().run()}
          >
            Абзац
          </ToolbarButton>
          <ToolbarButton
            label="Заголовок 2"
            pressed={toolbarState.heading2}
            disabled={!editor}
            onClick={() => editor?.chain().focus().toggleHeading({ level: 2 }).run()}
          >
            H2
          </ToolbarButton>
          <ToolbarButton
            label="Заголовок 3"
            pressed={toolbarState.heading3}
            disabled={!editor}
            onClick={() => editor?.chain().focus().toggleHeading({ level: 3 }).run()}
          >
            H3
          </ToolbarButton>
          <ToolbarButton
            label="Цитата"
            pressed={toolbarState.blockquote}
            disabled={!editor}
            onClick={() => editor?.chain().focus().toggleBlockquote().run()}
          >
            Цитата
          </ToolbarButton>
        </div>

        <div className={styles.toolbarGroup} role="group" aria-label="Списки">
          <ToolbarButton
            label="Маркированный список"
            pressed={toolbarState.bulletList}
            disabled={!editor}
            onClick={() => editor?.chain().focus().toggleBulletList().run()}
          >
            Список
          </ToolbarButton>
          <ToolbarButton
            label="Нумерованный список"
            pressed={toolbarState.orderedList}
            disabled={!editor}
            onClick={() => editor?.chain().focus().toggleOrderedList().run()}
          >
            Нумерация
          </ToolbarButton>
        </div>

        <div className={styles.toolbarGroup} role="group" aria-label="Таблица">
          <ToolbarButton
            label="Вставить таблицу 3 × 3 с заголовком"
            disabled={!toolbarState.canInsertTable}
            onClick={() => editor?.chain().focus().insertTable({ rows: 3, cols: 3, withHeaderRow: true }).run()}
          >
            Таблица
          </ToolbarButton>
          <ToolbarButton
            label="Добавить строку после текущей"
            disabled={!toolbarState.table || !toolbarState.canAddRow}
            onClick={() => editor?.chain().focus().addRowAfter().run()}
          >
            + Строка
          </ToolbarButton>
          <ToolbarButton
            label="Удалить текущую строку"
            disabled={!toolbarState.table || !toolbarState.canDeleteRow}
            onClick={() => editor?.chain().focus().deleteRow().run()}
          >
            − Строка
          </ToolbarButton>
          <ToolbarButton
            label="Добавить столбец после текущего"
            disabled={!toolbarState.table || !toolbarState.canAddColumn}
            onClick={() => editor?.chain().focus().addColumnAfter().run()}
          >
            + Столбец
          </ToolbarButton>
          <ToolbarButton
            label="Удалить текущий столбец"
            disabled={!toolbarState.table || !toolbarState.canDeleteColumn}
            onClick={() => editor?.chain().focus().deleteColumn().run()}
          >
            − Столбец
          </ToolbarButton>
          <ToolbarButton
            label="Удалить таблицу"
            disabled={!toolbarState.table || !toolbarState.canDeleteTable}
            onClick={() => editor?.chain().focus().deleteTable().run()}
          >
            Убрать таблицу
          </ToolbarButton>
        </div>

        <div className={styles.toolbarGroup} role="group" aria-label="Выравнивание">
          <label className={styles.visuallyHidden} htmlFor={alignmentId}>
            Выравнивание
          </label>
          <select
            id={alignmentId}
            className={styles.select}
            value={toolbarState.alignment}
            disabled={!editor}
            onChange={(event) => applyAlignment(event.target.value as Alignment)}
          >
            {ALIGNMENTS.map((alignment) => (
              <option key={alignment} value={alignment}>
                {ALIGNMENT_LABELS[alignment]}
              </option>
            ))}
          </select>
        </div>

        <div className={styles.toolbarGroup} role="group" aria-label="Цвет текста">
          <label className={styles.visuallyHidden} htmlFor={colorId}>
            Цвет текста
          </label>
          <input
            id={colorId}
            className={styles.colorInput}
            type="color"
            value={toolbarState.color ? toColorInputValue(toolbarState.color) : DEFAULT_COLOR}
            disabled={!editor}
            onChange={(event) => editor?.chain().setColor(event.target.value).run()}
          />
          <ToolbarButton
            label="Убрать цвет текста"
            disabled={!editor}
            onClick={() => editor?.chain().focus().unsetColor().run()}
          >
            Без цвета
          </ToolbarButton>
        </div>

        <div className={styles.toolbarGroup} role="group" aria-label="Ссылка">
          <ToolbarButton
            label="Ссылка"
            pressed={toolbarState.link}
            expanded={linkOpen}
            disabled={!editor}
            onClick={() => (linkOpen ? closeLinkField() : openLinkField())}
          >
            Ссылка
          </ToolbarButton>
        </div>

        <div className={styles.toolbarGroup} role="group" aria-label="Отмена изменений">
          <ToolbarButton
            label="Отменить"
            disabled={!editor || !toolbarState.canUndo}
            onClick={() => editor?.chain().focus().undo().run()}
          >
            ↶
          </ToolbarButton>
          <ToolbarButton
            label="Повторить"
            disabled={!editor || !toolbarState.canRedo}
            onClick={() => editor?.chain().focus().redo().run()}
          >
            ↷
          </ToolbarButton>
        </div>
      </div>

      {linkOpen ? (
        <div className={styles.linkRow}>
          <label className={styles.linkLabel} htmlFor={linkFieldId}>
            Адрес ссылки
          </label>
          <div className={styles.linkControls}>
            <input
              ref={linkInputRef}
              id={linkFieldId}
              className={styles.linkInput}
              type="text"
              inputMode="url"
              value={linkValue}
              placeholder="https://пример.ру или /put"
              autoComplete="off"
              spellCheck={false}
              aria-invalid={linkError ? true : undefined}
              aria-describedby={linkError ? linkErrorId : undefined}
              onChange={(event) => {
                setLinkValue(event.target.value);
                setLinkError("");
              }}
              onKeyDown={(event) => {
                if (event.key === "Escape") {
                  event.preventDefault();
                  event.stopPropagation();
                  closeLinkField();
                  return;
                }
                if (event.key === "Enter" && !event.metaKey && !event.ctrlKey && !event.altKey) {
                  event.preventDefault();
                  applyLink();
                }
              }}
            />
            <button className={styles.smallPrimary} type="button" onClick={applyLink}>
              {toolbarState.link ? "Заменить" : "Применить"}
            </button>
            <button
              className={styles.smallButton}
              type="button"
              disabled={!toolbarState.link}
              onClick={removeLink}
            >
              Убрать
            </button>
            <button className={styles.smallButton} type="button" onClick={closeLinkField}>
              Закрыть
            </button>
          </div>
          {linkError ? (
            <p id={linkErrorId} className={styles.error} role="alert">
              {linkError}
            </p>
          ) : null}
        </div>
      ) : null}
      </div>

      <EditorContent editor={editor} className={styles.editorContent} />
      <p className={styles.hint}>{hint}</p>
    </div>
  );
}
