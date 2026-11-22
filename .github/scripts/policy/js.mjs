// The workflow policy's reading of JavaScript and TypeScript modules (README.md in this
// directory): their tokens and what they import.

/// A JavaScript or TypeScript module as tokens, its comments dropped: identifiers, string literals
/// with their values, template literals (an interpolation's code read as code), regular expression
/// literals, numbers and single punctuators, each with its line. A rule over a module reads these,
/// not its text, so that a comment, a string or a regular expression holds no import and no name
/// of it, and spacing changes nothing (the review of b2be87b: `import/**/(x)`, `import"x"` and
/// `from"x"` past the regular expressions before). A literal or a comment that does not close is
/// marked `bad`, so that a misread `/` hides nothing from a rule that refuses those.
export function jsTokens(text) {
  const out = [];
  const n = text.length;
  const starts = [0];
  for (let i = 0; i < n; i++) if (text[i] === "\n") starts.push(i + 1);
  const lineOf = (at) => {
    let lo = 0;
    let hi = starts.length - 1;
    while (lo < hi) {
      const mid = (lo + hi + 1) >> 1;
      if (starts[mid] <= at) lo = mid;
      else hi = mid - 1;
    }
    return lo + 1;
  };
  const push = (t, v, at, extra = {}) => out.push({ t, v, line: lineOf(at), ...extra });
  let k = text.startsWith("#!") ? Math.max(text.indexOf("\n"), 0) : 0;
  let depth = 0;
  const resume = [];
  const regexAllowed = () => {
    const p = out[out.length - 1];
    if (!p) return true;
    if (p.t === "punct") return !/^[)\]}]$/.test(p.v);
    if (p.t === "tpl") return Boolean(p.open);
    if (p.t === "id") return /^(return|typeof|instanceof|in|of|new|delete|void|throw|case|do|else|yield|await)$/.test(p.v);
    return false;
  };
  const template = (at) => {
    let v = "";
    while (k < n) {
      const c = text[k];
      if (c === "\\") {
        v += text.slice(k, k + 2);
        k += 2;
      } else if (c === "`") {
        k++;
        push("tpl", v, at);
        return;
      } else if (c === "$" && text[k + 1] === "{") {
        k += 2;
        push("tpl", v, at, { open: true });
        resume.push(depth);
        depth = 0;
        return;
      } else {
        v += c;
        k++;
      }
    }
    push("tpl", v, at, { bad: true });
  };
  while (k < n) {
    const c = text[k];
    const at = k;
    if (c === " " || c === "\t" || c === "\n" || c === "\r") {
      k++;
    } else if (c === "/" && text[k + 1] === "/") {
      const e = text.indexOf("\n", k);
      k = e < 0 ? n : e;
    } else if (c === "/" && text[k + 1] === "*") {
      const e = text.indexOf("*/", k + 2);
      if (e < 0) push("punct", "/*", at, { bad: true });
      k = e < 0 ? n : e + 2;
    } else if (c === '"' || c === "'") {
      let j = k + 1;
      let v = "";
      while (j < n && text[j] !== c && text[j] !== "\n") {
        if (text[j] === "\\") {
          v += text.slice(j, j + 2);
          j += 2;
        } else v += text[j++];
      }
      push("str", unescapeJs(v), at, text[j] === c ? {} : { bad: true });
      k = j + 1;
    } else if (c === "`") {
      k++;
      template(at);
    } else if (c === "}" && depth === 0 && resume.length > 0) {
      depth = resume.pop();
      k++;
      template(at);
    } else if (c === "/" && regexAllowed()) {
      let j = k + 1;
      let inClass = false;
      while (j < n && text[j] !== "\n" && (text[j] !== "/" || inClass)) {
        if (text[j] === "\\") j++;
        else if (text[j] === "[") inClass = true;
        else if (text[j] === "]") inClass = false;
        j++;
      }
      const closed = text[j] === "/";
      j++;
      while (j < n && /[a-z]/i.test(text[j])) j++;
      push("re", text.slice(k, j), at, closed ? {} : { bad: true });
      k = j;
    } else if (/[A-Za-z_$]/.test(c)) {
      let j = k + 1;
      while (j < n && /[\w$]/.test(text[j])) j++;
      push("id", text.slice(k, j), at);
      k = j;
    } else if (/[0-9]/.test(c)) {
      let j = k + 1;
      while (j < n && /[\w.]/.test(text[j])) j++;
      push("num", text.slice(k, j), at);
      k = j;
    } else {
      if (c === "{") depth++;
      if (c === "}") depth--;
      push("punct", c, at);
      k++;
    }
  }
  return out;
}

/// A string literal's value from its source between the quotes: the escapes that spell a character
/// read as that character, so that `"ev\x61l"` is "eval".
export function unescapeJs(raw) {
  const simple = { n: "\n", t: "\t", r: "\r", b: "\b", f: "\f", v: "\v", 0: "\0" };
  return raw.replace(/\\(u\{([0-9a-fA-F]+)\}|u([0-9a-fA-F]{4})|x([0-9a-fA-F]{2})|\n|.)/g, (_, e, cp, u, x) => {
    const hex = cp ?? u ?? x;
    if (hex !== undefined) return String.fromCodePoint(parseInt(hex, 16));
    if (e === "\n") return "";
    return simple[e] ?? e;
  });
}

/// The specifiers a module's tokens import: each static import, `export ... from` and literal
/// `import("...")`, with its line and whether it is dynamic; and the lines of every `import` the
/// reading cannot follow (a computed `import(`, a form it does not know). The closure of what a
/// job runs follows these, and the rule for a secret job's modules refuses the rest.
export function jsImports(toks) {
  const specifiers = [];
  const unreadable = [];
  const at = (j) => toks[j] ?? { t: "end", v: "" };
  for (let i = 0; i < toks.length; i++) {
    const t = toks[i];
    if (t.t !== "id" || at(i - 1).v === ".") continue;
    if (t.v === "import") {
      if (at(i + 1).v === "." && at(i + 2).v === "meta") continue;
      if (at(i + 1).t === "str") {
        specifiers.push({ v: at(i + 1).v, line: t.line, dynamic: false });
        continue;
      }
      if (at(i + 1).v === "(") {
        if (at(i + 2).t === "str" && at(i + 3).v === ")") specifiers.push({ v: at(i + 2).v, line: t.line, dynamic: true });
        else unreadable.push(t.line);
        continue;
      }
      // `import` [`type`] (name | `{` names `}` | `*` `as` name) [`,` (`{` names `}` | `*` `as` name)] `from` "specifier"
      let j = i + 1;
      if (at(j).v === "type" && at(j + 1).v !== "from" && at(j + 1).v !== ",") j++;
      for (let part = 0; part < 2; part++) {
        if (at(j).t === "id" && at(j).v !== "from") j++;
        else if (at(j).v === "*" && at(j + 1).v === "as" && at(j + 2).t === "id") j += 3;
        else if (at(j).v === "{") {
          j++;
          while (at(j).t === "id" || at(j).v === ",") j++;
          if (at(j).v !== "}") break;
          j++;
        } else break;
        if (at(j).v !== ",") break;
        j++;
      }
      if (at(j).v === "from" && at(j + 1).t === "str") specifiers.push({ v: at(j + 1).v, line: t.line, dynamic: false });
      else unreadable.push(t.line);
    } else if (t.v === "export") {
      // `export` [`type`] (`*` [`as` name] | `{` names `}`) `from` "specifier"
      let j = i + 1;
      if (at(j).v === "type") j++;
      if (at(j).v === "*") {
        j++;
        if (at(j).v === "as") j += 2;
      } else if (at(j).v === "{") {
        j++;
        while (at(j).t === "id" || at(j).v === ",") j++;
        // A list the reading does not understand, a name written as a string among them, fails
        // closed (the final pass of ad38011: `export { "q" as w } from "..."` read as nothing).
        if (at(j).v !== "}") {
          unreadable.push(t.line);
          continue;
        }
        j++;
      } else continue;
      if (at(j).v === "from" && at(j + 1).t === "str") specifiers.push({ v: at(j + 1).v, line: t.line, dynamic: false });
      else if (at(j).v === "from") unreadable.push(t.line);
    }
  }
  return { specifiers, unreadable };
}
/// What node runs as a module, by a file's name: what the closure of a job reads for imports.
export const JS_MODULE = /\.(mjs|cjs|js|ts|mts|cts|jsx|tsx)$/;

/// Whether `toks` hold `seq`, a run of token values, anywhere.
export const holds = (toks, seq) => toks.some((t, i) => seq.every((v, k) => toks[i + k]?.v === v));
