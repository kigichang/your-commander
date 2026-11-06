/** Re-indents JSON text without parsing it into values, so numbers keep
 *  every digit (no float rounding of big integers), key order and
 *  duplicate keys survive, and a file cut off by the preview size cap
 *  still formats as far as it goes. Strings are copied verbatim. */
export function prettyJson(text: string, indent = "  "): string {
  let out = "";
  let depth = 0;
  let i = 0;
  const n = text.length;
  const newline = () => {
    out += "\n" + indent.repeat(depth);
  };
  while (i < n) {
    const c = text[i];
    if (c === '"') {
      let j = i + 1;
      while (j < n && text[j] !== '"') j += text[j] === "\\" ? 2 : 1;
      out += text.slice(i, j + 1);
      i = j + 1;
      continue;
    }
    if (c === " " || c === "\t" || c === "\n" || c === "\r") {
      i++;
      continue;
    }
    if (c === "{" || c === "[") {
      // Keep empty containers on one line: {} and [].
      let j = i + 1;
      while (j < n && /\s/.test(text[j])) j++;
      const close = c === "{" ? "}" : "]";
      if (text[j] === close) {
        out += c + close;
        i = j + 1;
        continue;
      }
      out += c;
      depth++;
      newline();
    } else if (c === "}" || c === "]") {
      depth = Math.max(0, depth - 1);
      newline();
      out += c;
    } else if (c === ",") {
      out += ",";
      newline();
    } else if (c === ":") {
      out += ": ";
    } else {
      out += c;
    }
    i++;
  }
  return out;
}

/** True when `text` is one well-formed JSON document. */
export function isJson(text: string): boolean {
  try {
    JSON.parse(text);
    return true;
  } catch {
    return false;
  }
}

export type JsonTokKind = "key" | "str" | "num" | "lit" | "b0" | "b1" | "b2" | "plain";
export interface JsonTok {
  kind: JsonTokKind;
  text: string;
}
export interface JsonLine {
  /** Indent levels at the start of the line (one per `indent`). */
  level: number;
  toks: JsonTok[];
}

/** Splits `prettyJson` output into lines of colorable tokens. Brackets get
 *  b0/b1/b2 by nesting depth (cycling), like editor bracket-pair colors. */
export function highlightJson(pretty: string, indent = "  "): JsonLine[] {
  const lines: JsonLine[] = [];
  let depth = 0;
  for (const raw of pretty.split("\n")) {
    let level = 0;
    let i = 0;
    while (raw.startsWith(indent, i)) {
      level++;
      i += indent.length;
    }
    const toks: JsonTok[] = [];
    const push = (kind: JsonTokKind, text: string) => {
      const last = toks[toks.length - 1];
      if (kind === "plain" && last?.kind === "plain") last.text += text;
      else toks.push({ kind, text });
    };
    const n = raw.length;
    while (i < n) {
      const c = raw[i];
      if (c === '"') {
        let j = i + 1;
        while (j < n && raw[j] !== '"') j += raw[j] === "\\" ? 2 : 1;
        j = Math.min(j + 1, n);
        let k = j;
        while (k < n && raw[k] === " ") k++;
        push(raw[k] === ":" ? "key" : "str", raw.slice(i, j));
        i = j;
      } else if (c === "{" || c === "[") {
        push(`b${depth % 3}` as JsonTokKind, c);
        depth++;
        i++;
      } else if (c === "}" || c === "]") {
        depth = Math.max(0, depth - 1);
        push(`b${depth % 3}` as JsonTokKind, c);
        i++;
      } else if (c === "-" || (c >= "0" && c <= "9")) {
        const m = /^-?\d*(?:\.\d*)?(?:[eE][+-]?\d*)?/.exec(raw.slice(i))!;
        const len = Math.max(1, m[0].length);
        push("num", raw.slice(i, i + len));
        i += len;
      } else if (/[A-Za-z]/.test(c)) {
        const m = /^[A-Za-z]+/.exec(raw.slice(i))!;
        push("lit", m[0]);
        i += m[0].length;
      } else {
        push("plain", c);
        i++;
      }
    }
    lines.push({ level, toks });
  }
  return lines;
}
