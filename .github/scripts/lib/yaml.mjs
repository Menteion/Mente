// A reader for the subset of YAML the workflows of .github/workflows use, so that the workflow
// policy test (policy.test.mjs) reads them as data and not line by line, with no dependency
// added. It reads block mappings and sequences, plain, quoted and block scalars (`|`, `>` and
// their chomping indicators), flow sequences of scalars, and comments. Anything else (anchors,
// aliases, tags, flow mappings, several documents, a plain scalar continued on the next line)
// throws, naming the line, so that a workflow it cannot read fails the test instead of being
// read wrongly.

/// Parses `text` and returns its value: objects, arrays, strings, numbers, booleans and null.
export function parseYaml(text) {
  const lines = (text.charCodeAt(0) === 0xfeff ? text.slice(1) : text).split(/\r?\n/);
  let i = 0;

  const fail = (message, at = i) => {
    throw new Error(`line ${at + 1}: ${message}: ${JSON.stringify(lines[at] ?? "")}`);
  };
  const indentOf = (line) => {
    const lead = /^[ \t]*/.exec(line)[0];
    if (lead.includes("\t")) fail("a tab in the indentation");
    return lead.length;
  };
  const isBlank = (line) => /^\s*(#.*)?$/.test(line);
  const skipBlank = () => {
    while (i < lines.length && isBlank(lines[i])) i++;
  };
  const KEY = /^([A-Za-z0-9_][A-Za-z0-9_.\-/]*|"[^"]*"|'[^']*'):(?:[ ]+(.*))?$/;

  function node(indent) {
    skipBlank();
    if (i >= lines.length) return null;
    const ind = indentOf(lines[i]);
    if (ind < indent) return null;
    const content = lines[i].slice(ind);
    if (content === "---" || content === "...") fail("several documents");
    return content === "-" || content.startsWith("- ") ? sequence(ind) : mapping(ind);
  }

  function mapping(indent) {
    const out = {};
    for (;;) {
      skipBlank();
      if (i >= lines.length) break;
      const ind = indentOf(lines[i]);
      if (ind < indent) break;
      if (ind > indent) fail("unexpected indentation");
      const content = lines[i].slice(ind);
      if (content === "-" || content.startsWith("- ")) break;
      const m = KEY.exec(content);
      if (!m) fail("not a key and a value");
      // GitHub decodes the escapes of a double-quoted key, so `"if"` spelled with one is `if`:
      // the policy would read another key, and so the reader refuses them.
      if (m[1].startsWith('"') && m[1].includes("\\")) fail("an escape in a quoted key");
      const key = unquote(m[1]);
      if (Object.hasOwn(out, key)) fail(`the key ${key} twice`);
      const at = i;
      i++;
      out[key] = value(m[2] ?? "", indent, at);
    }
    return out;
  }

  function sequence(indent) {
    const out = [];
    for (;;) {
      skipBlank();
      if (i >= lines.length) break;
      const ind = indentOf(lines[i]);
      const content = lines[i].slice(ind);
      if (ind !== indent || !(content === "-" || content.startsWith("- "))) {
        if (ind > indent) fail("unexpected indentation");
        break;
      }
      const item = content.slice(1).replace(/^ +/, "");
      const itemIndent = ind + (content.length - item.length);
      if (item === "" || item.startsWith("#")) {
        i++;
        out.push(node(indent + 1));
      } else if (/^[A-Za-z0-9_][A-Za-z0-9_.\-/]*:( |$)/.test(item) || /^("[^"]*"|'[^']*'):( |$)/.test(item)) {
        // A mapping whose first key shares the line with the dash: read it as if the dash were
        // a space.
        lines[i] = " ".repeat(itemIndent) + item;
        out.push(mapping(itemIndent));
      } else {
        const at = i;
        i++;
        out.push(inlineScalar(item, at, indent));
      }
    }
    return out;
  }

  function value(rest, keyIndent, at) {
    const text = rest.trim();
    if (text === "" || text.startsWith("#")) {
      skipBlank();
      if (i >= lines.length) return null;
      const ind = indentOf(lines[i]);
      const content = lines[i].slice(ind);
      if (ind > keyIndent) return node(ind);
      // A sequence may sit at its key's own indentation.
      if (ind === keyIndent && (content === "-" || content.startsWith("- "))) return sequence(ind);
      return null;
    }
    if (/^[|>][-+]?(\s+#.*)?$/.test(text)) return blockScalar(text[0], text[1], keyIndent);
    return inlineScalar(text, at, keyIndent);
  }

  function blockScalar(style, chomp, keyIndent) {
    const body = [];
    let blockIndent = -1;
    while (i < lines.length) {
      const line = lines[i];
      if (line.trim() === "") {
        body.push("");
        i++;
        continue;
      }
      const ind = indentOf(line);
      if (ind <= keyIndent) break;
      if (blockIndent < 0) blockIndent = ind;
      if (ind < blockIndent) fail("a block scalar line less indented than its first");
      body.push(line.slice(blockIndent));
      i++;
    }
    while (body.length > 0 && body[body.length - 1] === "") body.pop();
    let text;
    if (style === "|") {
      text = body.join("\n");
    } else {
      // Folded: lines joined by spaces, an empty line kept as a line break.
      text = "";
      for (const [n, line] of body.entries()) {
        if (n === 0) text = line;
        else if (line === "") text += "\n";
        else text += text.endsWith("\n") || text === "" ? line : ` ${line}`;
      }
    }
    if (chomp !== "-" && body.length > 0) text += "\n";
    return text;
  }

  function inlineScalar(text, at, keyIndent) {
    // A plain scalar continued on a more indented line is not part of the subset.
    const next = (() => {
      let j = i;
      while (j < lines.length && isBlank(lines[j])) j++;
      return j;
    })();
    if (next < lines.length && indentOf(lines[next]) > keyIndent) {
      const nextContent = lines[next].trimStart();
      const nested = nextContent.startsWith("- ") || KEY.test(nextContent);
      if (!nested) fail("a scalar continued on the next line", next);
    }
    return scalar(text, at);
  }

  function scalar(raw, at) {
    const text = raw.trim();
    if (text.startsWith('"')) {
      const m = /^"((?:[^"\\]|\\.)*)"\s*(#.*)?$/.exec(text);
      if (!m) fail("an unterminated double-quoted scalar", at);
      // The escapes the reader decodes; any other (`\x41`, `A`, `\e`) is refused, not kept as
      // written, which would let the policy read text GitHub reads otherwise.
      const escapes = { n: "\n", t: "\t", '"': '"', "\\": "\\", "/": "/" };
      return m[1].replace(/\\(.)/g, (_, c) => (Object.hasOwn(escapes, c) ? escapes[c] : fail(`an escape the reader does not decode: \\${c}`, at)));
    }
    if (text.startsWith("'")) {
      const m = /^'((?:[^']|'')*)'\s*(#.*)?$/.exec(text);
      if (!m) fail("an unterminated single-quoted scalar", at);
      return m[1].replace(/''/g, "'");
    }
    if (text.startsWith("[")) {
      const m = /^\[(.*)\]\s*(#.*)?$/.exec(text);
      if (!m) fail("a flow sequence not closed on its line", at);
      if (/[[\]{}]/.test(m[1])) fail("a nested flow collection", at);
      return m[1].trim() === "" ? [] : splitFlow(m[1]).map((item) => scalar(item, at));
    }
    if (text.startsWith("{")) fail("a flow mapping", at);
    if (/^[&*!]/.test(text)) fail("an anchor, alias or tag", at);
    const plain = text.replace(/\s+#.*$/, "").trim();
    if (plain === "true") return true;
    if (plain === "false") return false;
    if (plain === "null" || plain === "~") return null;
    if (/^-?\d+$/.test(plain)) return Number(plain);
    return plain;
  }

  const root = node(0);
  skipBlank();
  if (i < lines.length) fail("text after the document");
  return root;
}

/// Splits the inside of a flow sequence at its commas, outside quotes.
function splitFlow(text) {
  const items = [];
  let current = "";
  let quote = "";
  for (const c of text) {
    if (quote) {
      if (c === quote) quote = "";
    } else if (c === '"' || c === "'") {
      quote = c;
    } else if (c === ",") {
      items.push(current);
      current = "";
      continue;
    }
    current += c;
  }
  items.push(current);
  return items;
}

function unquote(key) {
  if (key.startsWith('"')) return key.slice(1, -1);
  if (key.startsWith("'")) return key.slice(1, -1).replace(/''/g, "'");
  return key;
}
