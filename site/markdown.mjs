// The transformations the documentation site applies to the repository's Markdown on its way into
// the book (docs/design-review.md, `docs-site`). Each one reads a source as it is and adds what a
// reader of the site needs: the maturity of every claim, the status of every register entry, and
// links that still resolve once a page lives at a new address. None of them edits a source file.

import path from 'node:path';

/** The four maturity labels of spec/README.md, in its order. */
export const LABELS = ['Implemented', 'Specified', 'Target', 'Hypothesis'];

/** The statuses of a design-register entry, in the order docs/design-review.md gives them. */
export const REGISTER_STATUSES = ['open', 'proposed', 'decided', 'done'];

/** A claim: a list item that starts with its maturity label written as code (spec/README.md). */
const CLAIM = /^(\s*(?:[-*+]|\d+[.)])\s+)`(Implemented|Specified|Target|Hypothesis)`/;

/** A line that holds nothing but one `@assert-*` directive. */
const DIRECTIVE_LINE = /^\s*<!--\s*@assert-[\s\S]*-->\s*$/;

/** The start of a list item, a heading, or a blank line: where a claim's own lines end. */
const BLOCK_START = /^\s*(?:$|[-*+]\s|\d+[.)]\s|#)/;

export function escapeHtml(text) {
  return text
    .replace(/&/g, '&amp;')
    .replace(/</g, '&lt;')
    .replace(/>/g, '&gt;')
    .replace(/"/g, '&quot;');
}

/** Markdown reduced to plain text, for an HTML attribute. */
export function plainText(markdown) {
  return markdown
    .replace(/\[([^\]]*)\]\([^)]*\)/g, '$1')
    .replace(/[`*]/g, '')
    .replace(/\s+/g, ' ')
    .trim();
}

/** Splits YAML front matter of `key: value` lines from the body. */
export function splitFrontMatter(text) {
  const m = /^---\n([\s\S]*?)\n---\n/.exec(text);
  if (!m) return { meta: {}, body: text };
  const meta = {};
  for (const line of m[1].split('\n')) {
    const kv = /^([A-Za-z_][\w-]*):\s*(.*)$/.exec(line);
    if (kv) meta[kv[1]] = kv[2].replace(/^(["'])(.*)\1$/, '$2');
  }
  return { meta, body: text.slice(m[0].length) };
}

/** For each line, whether it belongs to a fenced code block, its fences included. */
export function fenceMask(lines) {
  const mask = [];
  let fence = null;
  for (const line of lines) {
    if (fence) {
      mask.push(true);
      const close = /^\s*(`{3,}|~{3,})\s*$/.exec(line);
      if (close && close[1][0] === fence[0] && close[1].length >= fence.length) fence = null;
    } else {
      const open = /^\s*(`{3,}|~{3,})/.exec(line);
      if (open) fence = open[1];
      mask.push(Boolean(open));
    }
  }
  return mask;
}

/** Applies `fn` to the parts of one line that lie outside code spans. */
export function mapOutsideCode(line, fn) {
  let out = '';
  let text = '';
  let i = 0;
  while (i < line.length) {
    if (line[i] !== '`') {
      text += line[i];
      i += 1;
      continue;
    }
    let n = 0;
    while (line[i + n] === '`') n += 1;
    let close = -1;
    for (let j = i + n; j < line.length; ) {
      if (line[j] !== '`') {
        j += 1;
        continue;
      }
      let m = 0;
      while (line[j + m] === '`') m += 1;
      if (m === n) {
        close = j;
        break;
      }
      j += m;
    }
    if (close === -1) {
      text += line.slice(i, i + n);
      i += n;
      continue;
    }
    out += fn(text) + line.slice(i, close + n);
    text = '';
    i = close + n;
  }
  return out + fn(text);
}

/** What each maturity label means, read from the label table of spec/README.md. */
export function labelMeanings(specReadme) {
  const meanings = {};
  for (const line of specReadme.split('\n')) {
    const m = /^\|\s*\*\*(Implemented|Specified|Target|Hypothesis)\*\*\s*\|([^|]*)\|/.exec(line);
    if (m) meanings[m[1]] = plainText(m[2]);
  }
  for (const label of LABELS) {
    if (!meanings[label]) throw new Error(`spec/README.md has no table row for the label ${label}`);
  }
  return meanings;
}

export function maturityBadge(label, title) {
  const attr = title ? ` title="${escapeHtml(title)}"` : '';
  return `<span class="maturity maturity-${label.toLowerCase()}"${attr}>${label}</span>`;
}

export function statusBadge(status, title) {
  const attr = title ? ` title="${escapeHtml(title)}"` : '';
  return `<span class="status status-${status}"${attr}>${status}</span>`;
}

/**
 * Turns the label that opens each claim into a badge whose tooltip says what the label means and,
 * for an `Implemented` claim, names the tests its directives cite as evidence. Returns the new
 * body and the number of claims under each label.
 */
export function markClaims(body, meanings) {
  const counts = Object.fromEntries(LABELS.map((l) => [l, 0]));
  const lines = body.split('\n');
  const mask = fenceMask(lines);
  for (let i = 0; i < lines.length; i += 1) {
    if (mask[i]) continue;
    const m = CLAIM.exec(lines[i]);
    if (!m) continue;
    const label = m[2];
    counts[label] += 1;
    const evidence = [];
    for (let j = i + 1; j < lines.length && !mask[j]; j += 1) {
      if (DIRECTIVE_LINE.test(lines[j])) {
        const s = /symbol="fn (test_\w+)"/.exec(lines[j]);
        if (s) evidence.push(s[1]);
      } else if (BLOCK_START.test(lines[j])) {
        break;
      }
    }
    let title = `${label}: ${meanings[label]}`;
    if (label === 'Implemented' && evidence.length > 0) {
      title += ` Evidence: ${evidence.join(', ')}.`;
    }
    lines[i] = m[1] + maturityBadge(label, title) + lines[i].slice(m[0].length);
  }
  return { body: lines.join('\n'), counts };
}

/** The cells of a table row, as written, split at each `|` that is not escaped. */
export function splitRow(line) {
  const trimmed = line.trim();
  if (!trimmed.startsWith('|') || !trimmed.endsWith('|')) return null;
  const cells = [];
  let cell = '';
  for (let i = 1; i < trimmed.length - 1; i += 1) {
    const c = trimmed[i];
    if (c === '\\') {
      cell += c + (trimmed[i + 1] ?? '');
      i += 1;
    } else if (c === '|') {
      cells.push(cell);
      cell = '';
    } else {
      cell += c;
    }
  }
  cells.push(cell);
  return cells;
}

/**
 * Gives each entry of the design register an anchor named by its key, so that the links the
 * documents write as `design-review.md#key` resolve, and turns the status that opens its last cell
 * into a badge. Returns the new body and the number of entries under each status.
 */
export function annotateRegister(body) {
  const counts = Object.fromEntries(REGISTER_STATUSES.map((s) => [s, 0]));
  const lines = body.split('\n');
  const mask = fenceMask(lines);
  for (let i = 0; i < lines.length; i += 1) {
    if (mask[i] || !lines[i].startsWith('| `')) continue;
    const cells = splitRow(lines[i]);
    if (!cells || cells.length < 4) continue;
    const key = /^\s*`([a-z0-9][a-z0-9-]*)`\s*$/.exec(cells[0]);
    const last = cells.length - 1;
    const status = /^(\s*)(open|proposed|decided|done)\b/.exec(cells[last]);
    if (!key || !status) continue;
    counts[status[2]] += 1;
    cells[0] = ` <a id="${key[1]}"></a>\`${key[1]}\` `;
    cells[last] = status[1] + statusBadge(status[2]) + cells[last].slice(status[0].length);
    lines[i] = `|${cells.join('|')}|`;
  }
  return { body: lines.join('\n'), counts };
}

/** Counts the checked and unchecked task items of a checklist such as ROADMAP.md. */
export function countTasks(body) {
  const lines = body.split('\n');
  const mask = fenceMask(lines);
  let done = 0;
  let open = 0;
  lines.forEach((line, i) => {
    if (mask[i]) return;
    const m = /^\s*[-*+]\s+\[([ xX])\]\s/.exec(line);
    if (m) {
      if (m[1] === ' ') open += 1;
      else done += 1;
    }
  });
  return { done, open };
}

/**
 * Where a relative link of the page `page` (a repository path) points once the page is in the
 * book. A target that is a page of the book keeps its link; a file or directory of the repository
 * that is not becomes a link to it on GitHub at `ref`; anything else is broken.
 */
export function resolveLink(url, { page, inBook, kindOf, repoUrl, ref }) {
  if (/^[a-z][a-z0-9+.-]*:/i.test(url) || url.startsWith('//') || url.startsWith('#')) {
    return { url };
  }
  const hash = url.indexOf('#');
  const target = hash === -1 ? url : url.slice(0, hash);
  const fragment = hash === -1 ? '' : url.slice(hash);
  if (target === '') return { url };
  let decoded;
  try {
    decoded = decodeURI(target);
  } catch {
    return { url, broken: true };
  }
  const resolved = path.posix
    .normalize(path.posix.join(path.posix.dirname(page), decoded))
    .replace(/\/$/, '');
  if (resolved === '..' || resolved.startsWith('../') || path.posix.isAbsolute(resolved)) {
    return { url, broken: true };
  }
  if (inBook(resolved)) return { url };
  const kind = kindOf(resolved);
  if (!kind) return { url, broken: true };
  const tree = kind === 'dir' ? 'tree' : 'blob';
  return { url: `${repoUrl}/${tree}/${ref}/${encodeURI(resolved)}${fragment}` };
}

const INLINE_LINK = /(\]\()(<[^>]*>|[^)\s]+)((?:\s+"[^"]*")?\))/g;
const REFERENCE_DEFINITION = /^(\s{0,3}\[[^\]]+\]:\s*)(\S+)(.*)$/;

/** Rewrites every Markdown link of a page with `resolveLink`; returns the links that are broken. */
export function rewriteLinks(body, context) {
  const broken = [];
  const fix = (raw) => {
    const angled = raw.startsWith('<') && raw.endsWith('>');
    const url = angled ? raw.slice(1, -1) : raw;
    const r = resolveLink(url, context);
    if (r.broken) broken.push(url);
    return angled ? `<${r.url}>` : r.url;
  };
  const lines = body.split('\n');
  const mask = fenceMask(lines);
  for (let i = 0; i < lines.length; i += 1) {
    if (mask[i]) continue;
    const def = REFERENCE_DEFINITION.exec(lines[i]);
    if (def) {
      lines[i] = def[1] + fix(def[2]) + def[3];
      continue;
    }
    lines[i] = mapOutsideCode(lines[i], (text) =>
      text.replace(INLINE_LINK, (_, open, url, close) => open + fix(url) + close),
    );
  }
  return { body: lines.join('\n'), broken };
}

/** Inserts `block` after the page's first level-one heading, or at its top if it has none. */
export function insertAfterTitle(body, block) {
  const lines = body.split('\n');
  const mask = fenceMask(lines);
  const at = lines.findIndex((line, i) => !mask[i] && /^# /.test(line));
  if (at === -1) return `${block}\n\n${body}`;
  lines.splice(at + 1, 0, '', block);
  return lines.join('\n');
}

/** The text of the page's first level-one heading. */
export function firstTitle(body) {
  const lines = body.split('\n');
  const mask = fenceMask(lines);
  const at = lines.findIndex((line, i) => !mask[i] && /^# /.test(line));
  return at === -1 ? null : lines[at].slice(2).trim();
}

/** The list of codes `mn explain` prints with no argument. */
export function parseExplainList(text) {
  return text
    .split(/\r?\n/)
    .filter((line) => line.trim() !== '')
    .map((line) => {
      const m = /^(E\d{4}) {2}(.*?)(?: \((reserved|retired)\))?$/.exec(line);
      if (!m) throw new Error(`mn explain printed a line the site cannot read: ${line}`);
      return { code: m[1], title: m[2], status: m[3] ?? 'emitted' };
    });
}

/** One entry as `mn explain <code>` prints it. */
export function parseExplain(text) {
  const lines = text.replace(/\r\n/g, '\n').split('\n');
  const head = /^(E\d{4}): (.*?)(?: \((reserved|retired)(?::[^)]*)?\))?$/.exec(lines[0]);
  const gov = lines.findIndex((l) => /^Governed by \S+\. Failure class: .+\.$/.test(l));
  if (!head || gov === -1) {
    throw new Error(`mn explain printed an entry the site cannot read:\n${text}`);
  }
  const governed = /^Governed by (\S+)\. Failure class: (.+)\.$/.exec(lines[gov]);
  const failsAt = lines.findIndex((l) => /^This fails with E\d{4}:$/.test(l));
  const passesAt = lines.findIndex((l) => l === 'This passes:');
  const program = (from, to) =>
    lines
      .slice(from, to)
      .filter((l) => l.startsWith('    '))
      .map((l) => l.slice(4))
      .join('\n');
  const entry = {
    code: head[1],
    title: head[2],
    status: head[3] ?? 'emitted',
    explanation: lines.slice(1, gov).join('\n').trim(),
    spec: governed[1],
    failureClass: governed[2],
    fails: '',
    passes: '',
  };
  if (failsAt !== -1 && passesAt > failsAt) {
    entry.fails = program(failsAt + 1, passesAt);
    entry.passes = program(passesAt + 1, lines.length);
  }
  return entry;
}
