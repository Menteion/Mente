// Checks a built book: every relative link and every fragment resolves inside it, and no
// `@assert-*` or `@spec-item` directive of the sources is shown to a reader. Run by build.mjs
// after mdBook, and on its own as `node site/check.mjs <book directory> [site url]`.

import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

function walk(dir) {
  const out = [];
  for (const entry of fs.readdirSync(dir, { withFileTypes: true })) {
    const full = path.join(dir, entry.name);
    if (entry.isDirectory()) out.push(...walk(full));
    else out.push(full);
  }
  return out;
}

function decodeEntities(text) {
  return text
    .replace(/&quot;/g, '"')
    .replace(/&#39;/g, "'")
    .replace(/&lt;/g, '<')
    .replace(/&gt;/g, '>')
    .replace(/&amp;/g, '&');
}

/** The ids and names a page defines, which a fragment may point at. */
export function anchorsOf(html) {
  const anchors = new Set();
  for (const m of html.matchAll(/\s(?:id|name)="([^"]*)"/g)) anchors.add(decodeEntities(m[1]));
  return anchors;
}

/** The `href` and `src` values of a page, entities decoded. */
export function linksOf(html) {
  return [...html.matchAll(/\s(?:href|src)="([^"]*)"/g)].map((m) => decodeEntities(m[1]));
}

/**
 * The HTML with its comments removed, each ending where the HTML tokenizer ends it: at the first
 * `-->` or `--!>` after its `<!--`, at once for `<!-->` and `<!--->`, and at the end of the text if
 * it is never closed. A scan from one comment boundary to the next, so that markers nested in or
 * straddling a comment are judged as a browser judges them, never pieced back together.
 */
export function stripComments(html) {
  let out = '';
  let i = 0;
  for (;;) {
    const open = html.indexOf('<!--', i);
    if (open === -1) return out + html.slice(i);
    out += html.slice(i, open);
    const body = open + '<!--'.length;
    if (html.startsWith('>', body)) {
      i = body + 1;
      continue;
    }
    if (html.startsWith('->', body)) {
      i = body + 2;
      continue;
    }
    const ends = [html.indexOf('-->', body), html.indexOf('--!>', body)].filter((n) => n !== -1);
    if (ends.length === 0) return out;
    const end = Math.min(...ends);
    i = end + (html.startsWith('-->', end) ? '-->'.length : '--!>'.length);
  }
}

/** The index of the next `<tag>` or `<tag ...>` at or after `from`, or -1. */
function openTag(html, tag, from) {
  for (let i = html.indexOf(`<${tag}`, from); i !== -1; i = html.indexOf(`<${tag}`, i + 1)) {
    if (/[\s>]/.test(html[i + tag.length + 1] ?? '')) return i;
  }
  return -1;
}

/** Splits HTML into the `<tag>` elements it holds and the text around them, by scanning. */
export function splitElements(html, tag) {
  const close = `</${tag}>`;
  const inside = [];
  let outside = '';
  let i = 0;
  for (;;) {
    const open = openTag(html, tag, i);
    if (open === -1) break;
    outside += html.slice(i, open);
    const end = html.indexOf(close, open);
    if (end === -1) {
      inside.push(html.slice(open));
      i = html.length;
      break;
    }
    inside.push(html.slice(open, end + close.length));
    i = end + close.length;
  }
  return { inside, outside: outside + html.slice(i) };
}

/**
 * Errors for a directive of the sources that a page shows instead of hiding: its comment opener
 * rendered as text, in a code block (an over-indented directive) or in prose. A directive written
 * on purpose inside inline code, as spec/README.md does to describe one, is allowed.
 */
export function directiveErrors(rel, html) {
  const main = /<main>([\s\S]*)<\/main>/.exec(html)?.[1] ?? '';
  const visible = stripComments(main);
  const directive = /&lt;!--\s*@(?:assert-|spec-item)/;
  const errors = [];
  const blocks = splitElements(visible, 'pre');
  if (blocks.inside.some((pre) => directive.test(pre))) {
    errors.push(`${rel}: a directive is shown as a code block`);
  }
  if (directive.test(splitElements(blocks.outside, 'code').outside)) {
    errors.push(`${rel}: a directive is shown as text`);
  }
  return errors;
}

/**
 * Every broken link, missing fragment and visible directive of the book in `bookDir`, which is
 * served under `siteUrl` (such as `/Mente/`).
 */
export function checkBook(bookDir, siteUrl = '/') {
  const root = path.resolve(bookDir);
  const pages = walk(root).filter((f) => f.endsWith('.html'));
  const cache = new Map();
  const anchors = (file) => {
    if (!cache.has(file)) cache.set(file, anchorsOf(fs.readFileSync(file, 'utf8')));
    return cache.get(file);
  };
  const errors = [];
  for (const file of pages) {
    const rel = path.relative(root, file).split(path.sep).join('/');
    const html = fs.readFileSync(file, 'utf8');
    const base = /<base href="([^"]*)"/.exec(html)?.[1];
    for (const url of linksOf(html)) {
      if (url === '' || /^[a-z][a-z0-9+.-]*:/i.test(url) || url.startsWith('//')) continue;
      const hash = url.indexOf('#');
      const target = (hash === -1 ? url : url.slice(0, hash)).replace(/\?.*$/, '');
      const fragment = hash === -1 ? '' : decodeURIComponent(url.slice(hash + 1));
      let resolved;
      if (target === '') {
        // A fragment alone names a place in the page itself.
        resolved = file;
      } else {
        let fromRoot = null;
        if (target.startsWith('/')) fromRoot = target;
        else if (base && base.startsWith('/')) fromRoot = path.posix.join(base, target);
        if (fromRoot !== null) {
          if (!fromRoot.startsWith(siteUrl)) {
            errors.push(`${rel}: ${url} points outside the site's address ${siteUrl}`);
            continue;
          }
          resolved = path.join(root, decodeURIComponent(fromRoot.slice(siteUrl.length)));
        } else {
          resolved = path.join(path.dirname(file), decodeURIComponent(target));
        }
      }
      if (fs.existsSync(resolved) && fs.statSync(resolved).isDirectory()) {
        resolved = path.join(resolved, 'index.html');
      }
      if (!fs.existsSync(resolved)) {
        errors.push(`${rel}: broken link ${url}`);
        continue;
      }
      if (fragment !== '' && resolved.endsWith('.html') && !anchors(resolved).has(fragment)) {
        errors.push(`${rel}: ${url} names a fragment the page does not have`);
      }
    }
    errors.push(...directiveErrors(rel, html));
  }
  return { pages: pages.length, errors };
}

if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  const [bookDir, siteUrl = '/'] = process.argv.slice(2);
  if (!bookDir) {
    console.error('usage: node site/check.mjs <book directory> [site url]');
    process.exit(2);
  }
  const { pages, errors } = checkBook(bookDir, siteUrl);
  for (const e of errors) console.error(e);
  console.log(`${pages} pages checked, ${errors.length} problems`);
  process.exit(errors.length === 0 ? 0 : 1);
}
