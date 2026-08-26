import { Fragment, type ReactNode } from "react";

type MarkdownMessageProps = { text: string };
type MarkdownBlock =
  | { kind: "heading"; level: number; text: string }
  | { kind: "paragraph"; text: string }
  | { kind: "unordered-list"; items: string[] }
  | { kind: "ordered-list"; items: string[] }
  | { kind: "quote"; text: string }
  | { kind: "code"; language: string; text: string }
  | { kind: "rule" };

const INLINE_MARKDOWN = /(\*\*[^*]+\*\*|`[^`]+`|\[[^\]]+\]\([^)]+\))/g;

function renderInlineMarkdown(text: string): ReactNode[] {
  return text.split(INLINE_MARKDOWN).filter(Boolean).map((part, index) => {
    if (part.startsWith("**") && part.endsWith("**")) return <strong key={index}>{part.slice(2, -2)}</strong>;
    if (part.startsWith("`") && part.endsWith("`")) return <code key={index}>{part.slice(1, -1)}</code>;
    const link = part.match(/^\[([^\]]+)]\(([^)]+)\)$/);
    if (link) return <span className="markdown-link" title={link[2]} key={index}>{link[1]}</span>;
    return <Fragment key={index}>{part}</Fragment>;
  });
}

function parseMarkdownBlocks(text: string): MarkdownBlock[] {
  const lines = text.replace(/\r\n?/g, "\n").split("\n");
  const blocks: MarkdownBlock[] = [];
  let index = 0;

  while (index < lines.length) {
    const line = lines[index];
    if (!line.trim()) { index += 1; continue; }

    const fence = line.match(/^```\s*([\w-]*)\s*$/);
    if (fence) {
      const codeLines: string[] = [];
      index += 1;
      while (index < lines.length && !/^```\s*$/.test(lines[index])) { codeLines.push(lines[index]); index += 1; }
      if (index < lines.length) index += 1;
      blocks.push({ kind: "code", language: fence[1], text: codeLines.join("\n") });
      continue;
    }

    const heading = line.match(/^(#{1,4})\s+(.+)$/);
    if (heading) { blocks.push({ kind: "heading", level: heading[1].length, text: heading[2] }); index += 1; continue; }
    if (/^\s*([-*_])(?:\s*\1){2,}\s*$/.test(line)) { blocks.push({ kind: "rule" }); index += 1; continue; }

    if (/^\s*[-*+]\s+/.test(line)) {
      const items: string[] = [];
      while (index < lines.length) {
        const item = lines[index].match(/^\s*[-*+]\s+(.+)$/);
        if (!item) break;
        items.push(item[1]); index += 1;
      }
      blocks.push({ kind: "unordered-list", items });
      continue;
    }

    if (/^\s*\d+[.)]\s+/.test(line)) {
      const items: string[] = [];
      while (index < lines.length) {
        const item = lines[index].match(/^\s*\d+[.)]\s+(.+)$/);
        if (!item) break;
        items.push(item[1]); index += 1;
      }
      blocks.push({ kind: "ordered-list", items });
      continue;
    }

    if (/^>/.test(line)) {
      const quoteLines: string[] = [];
      while (index < lines.length) {
        const quoted = lines[index].match(/^>\s?(.*)$/);
        if (!quoted) break;
        quoteLines.push(quoted[1]); index += 1;
      }
      blocks.push({ kind: "quote", text: quoteLines.join("\n") });
      continue;
    }

    const paragraphLines: string[] = [];
    while (index < lines.length && lines[index].trim()) {
      if (paragraphLines.length > 0 && (/^```/.test(lines[index]) || /^(#{1,4})\s+/.test(lines[index]) || /^\s*[-*+]\s+/.test(lines[index]) || /^\s*\d+[.)]\s+/.test(lines[index]) || /^>/.test(lines[index]))) break;
      paragraphLines.push(lines[index]); index += 1;
    }
    blocks.push({ kind: "paragraph", text: paragraphLines.join("\n") });
  }
  return blocks;
}

export function MarkdownMessage({ text }: MarkdownMessageProps) {
  return <div className="markdown-message">{parseMarkdownBlocks(text).map((block, index) => {
    if (block.kind === "heading") {
      const Heading = block.level <= 2 ? "h3" : "h4";
      return <Heading key={index}>{renderInlineMarkdown(block.text)}</Heading>;
    }
    if (block.kind === "unordered-list" || block.kind === "ordered-list") {
      const List = block.kind === "unordered-list" ? "ul" : "ol";
      return <List key={index}>{block.items.map((item, itemIndex) => <li key={itemIndex}>{renderInlineMarkdown(item)}</li>)}</List>;
    }
    if (block.kind === "quote") return <blockquote key={index}>{renderInlineMarkdown(block.text)}</blockquote>;
    if (block.kind === "code") return <pre key={index} data-language={block.language || undefined}><code>{block.text}</code></pre>;
    if (block.kind === "rule") return <hr key={index} />;
    return <p key={index}>{renderInlineMarkdown(block.text)}</p>;
  })}</div>;
}
