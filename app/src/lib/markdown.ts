import DOMPurify from "dompurify";
import { marked } from "marked";

marked.setOptions({ gfm: true, breaks: false });

/** A reply's Markdown as safe HTML. Fine on a reply still being written: it is parsed anew
    each time it grows. */
export function render(text: string): string {
  return DOMPurify.sanitize(marked.parse(text, { async: false }));
}
