import type { ReactNode } from "react";
import type { PostDocument, PostMark, PostNode } from "@/lib/post-content";
import styles from "./PostBody.module.css";

function renderText(text: string, marks: PostMark[] | undefined): ReactNode {
  let result: ReactNode = text;

  for (const mark of marks ?? []) {
    switch (mark.type) {
      case "bold":
        result = <strong>{result}</strong>;
        break;
      case "italic":
        result = <em>{result}</em>;
        break;
      case "strike":
        result = <s>{result}</s>;
        break;
      case "underline":
        result = <u>{result}</u>;
        break;
      case "textStyle":
        result = <span style={{ color: mark.attrs?.color }}>{result}</span>;
        break;
      case "link": {
        const href = mark.attrs?.href ?? "";
        const external = !href.startsWith("/");
        result = <a href={href} target={external ? "_blank" : undefined} rel={external ? "noopener noreferrer" : undefined}>{result}</a>;
        break;
      }
    }
  }

  return result;
}

function safeCellSpan(value: number | undefined): number {
  return typeof value === "number" && Number.isInteger(value) && value >= 1 && value <= 100 ? value : 1;
}

function renderNode(node: PostNode, key: number): ReactNode {
  const children = node.content?.map(renderNode);
  const textAlign = node.attrs?.textAlign as "left" | "center" | "right" | "justify" | undefined;

  switch (node.type) {
    case "paragraph":
      return <p key={key} style={{ textAlign }}>{children}</p>;
    case "heading": {
      const Heading = node.attrs?.level === 3 ? "h4" : "h3";
      return <Heading key={key} style={{ textAlign }}>{children}</Heading>;
    }
    case "blockquote":
      return <blockquote key={key}>{children}</blockquote>;
    case "bulletList":
      return <ul key={key}>{children}</ul>;
    case "orderedList":
      return <ol key={key} start={node.attrs?.start}>{children}</ol>;
    case "listItem":
      return <li key={key}>{children}</li>;
    case "table":
      return (
        <div key={key} className={styles.tableWrapper} role="region" aria-label="Таблица" tabIndex={0}>
          <table><tbody>{children}</tbody></table>
        </div>
      );
    case "tableRow":
      return <tr key={key}>{children}</tr>;
    case "tableCell":
    case "tableHeader": {
      const Cell = node.type === "tableHeader" ? "th" : "td";
      const colspan = safeCellSpan(node.attrs?.colspan);
      const rowspan = safeCellSpan(node.attrs?.rowspan);
      const align = node.attrs?.align;
      const cellAlign = align === "left" || align === "center" || align === "right" ? align : undefined;
      const colwidth = node.attrs?.colwidth;
      const width = Array.isArray(colwidth) && colwidth.length === colspan &&
        colwidth.every((value) => Number.isInteger(value) && value > 0 && value <= 10_000)
        ? colwidth.reduce((total, value) => total + value, 0)
        : undefined;
      return (
        <Cell key={key} colSpan={colspan} rowSpan={rowspan} style={{ textAlign: cellAlign, width }}>
          {children}
        </Cell>
      );
    }
    case "hardBreak":
      return <br key={key} />;
    case "text":
      return <span key={key}>{renderText(node.text ?? "", node.marks)}</span>;
    default:
      return null;
  }
}

export default function PostBody({ body }: { body: PostDocument }) {
  return <div className={styles.body}>{body.content.map(renderNode)}</div>;
}
