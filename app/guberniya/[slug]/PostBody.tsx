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
