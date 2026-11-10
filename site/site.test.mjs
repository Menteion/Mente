// Tests of the documentation site's transformations and of the check run on the built book.
// Run with `npm run test:site`.

import assert from 'node:assert/strict';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { test } from 'node:test';
import { checkBook, directiveErrors, splitElements, stripComments } from './check.mjs';
import {
  annotateRegister,
  countTasks,
  escapeHtml,
  escapeMarkdownText,
  fenceMask,
  inlineHtml,
  insertAfterTitle,
  labelMeanings,
  mapOutsideCode,
  markClaims,
  parseExplain,
  parseExplainList,
  resolveLink,
  rewriteLinks,
  splitFrontMatter,
  splitRow,
} from './markdown.mjs';

const MEANINGS = {
  Implemented: 'Done today.',
  Specified: 'Fixed in spec/.',
  Target: 'A goal.',
  Hypothesis: 'An assumption.',
};

test('front matter is split from the body and read as key-value pairs', () => {
  const { meta, body } = splitFrontMatter('---\ntitle: A "quoted" title\nversion: 0.1.0\n---\n\n# A\n');
  assert.deepEqual(meta, { title: 'A "quoted" title', version: '0.1.0' });
  assert.equal(body, '\n# A\n');
  assert.deepEqual(splitFrontMatter('# No front matter\n'), { meta: {}, body: '# No front matter\n' });
});

test('the label meanings are read from the table of spec/README.md', () => {
  const readme = fs.readFileSync(new URL('../spec/README.md', import.meta.url), 'utf8').replace(/\r\n/g, '\n');
  const meanings = labelMeanings(readme);
  assert.deepEqual(Object.keys(meanings).sort(), ['Hypothesis', 'Implemented', 'Specified', 'Target']);
  assert.match(meanings.Implemented, /unit test/);
  assert.throws(() => labelMeanings('| **Implemented** | x | y |\n'), /no table row for the label Specified/);
});

test('each claim label becomes a badge, and an Implemented claim names its evidence', () => {
  const src = [
    '1. `Specified` **A rule**:',
    '   - `Implemented`: the compiler does it.',
    '     <!-- @assert-count target="a.rs" symbol="fn test_one" min="1" reason="r" -->',
    '     <!-- @assert-count target="a.rs" symbol="pub fn helper" min="1" reason="r" -->',
    '   - `Target` **Fast**: under 1 ms.',
    '- Not a claim: the word `Implemented` inside a sentence.',
    '- (`Hypothesis`) in parentheses is not a claim either.',
    '```',
    '- `Implemented` inside a code block stays as written',
    '```',
  ].join('\n');
  const { body, counts } = markClaims(src, MEANINGS);
  assert.deepEqual(counts, { Implemented: 1, Specified: 1, Target: 1, Hypothesis: 0 });
  const lines = body.split('\n');
  assert.equal(
    lines[0],
    '1. <span class="maturity maturity-specified" title="Specified: Fixed in spec/.">Specified</span> **A rule**:',
  );
  assert.match(lines[1], /^ {3}- <span class="maturity maturity-implemented" title="Implemented: Done today\. Evidence: test_one\.">Implemented<\/span>: the compiler/);
  assert.match(lines[4], /maturity-target/);
  assert.equal(lines[5], src.split('\n')[5]);
  assert.equal(lines[6], src.split('\n')[6]);
  assert.equal(lines[8], src.split('\n')[8]);
  // The directives stay in place, so they remain HTML comments.
  assert.equal(lines[2], src.split('\n')[2]);
});

test('fenced code blocks are recognised with their fences', () => {
  assert.deepEqual(fenceMask(['a', '```mente', 'b', '````', 'c']), [false, true, true, true, false]);
  assert.deepEqual(fenceMask(['````', '```', 'x', '````', 'y']), [true, true, true, true, false]);
  assert.deepEqual(fenceMask(['~~~', '```', '~~~', 'z']), [true, true, true, false]);
});

test('text outside code spans is transformed and code spans are kept', () => {
  const upper = (s) => s.toUpperCase();
  assert.equal(mapOutsideCode('a `b` c ``d ` e`` f', upper), 'A `b` C ``d ` e`` F');
  assert.equal(mapOutsideCode('an ` unmatched tick', upper), 'AN ` UNMATCHED TICK');
});

test('a title is escaped for Markdown with its backslashes first, and its code spans kept', () => {
  // A backslash already in the text is doubled, so it cannot escape what follows it.
  assert.equal(escapeMarkdownText('a\\b'), 'a\\\\b');
  assert.equal(escapeMarkdownText('a\\[b]'), 'a\\\\\\[b\\]');
  assert.equal(escapeMarkdownText('x | *y* <T>'), 'x \\| \\*y\\* \\<T\\>');
  // Inside a code span the text is literal already: nothing is added.
  assert.equal(escapeMarkdownText('`&&`, `||` or `\\` from C'), '`&&`\\, `||` or `\\` from C');
  assert.equal(escapeMarkdownText('an ` unmatched tick'), 'an \\` unmatched tick');
});

test('a title is rendered as HTML with its code spans as code', () => {
  assert.equal(inlineHtml('`&&`, `||` or `!` from C'), '<code>&amp;&amp;</code>, <code>||</code> or <code>!</code> from C');
  assert.equal(inlineHtml('a\\b `c\\d` <T> "q"'), 'a\\b <code>c\\d</code> &lt;T&gt; &quot;q&quot;');
  assert.equal(inlineHtml('`` a`b ``'), '<code>a`b</code>');
  assert.equal(escapeHtml(`\\"'<&>`), '\\&quot;&#39;&lt;&amp;&gt;');
});

test('comments end where the HTML tokenizer ends them, nested or overlapping markers included', () => {
  assert.equal(stripComments('a<!-- b -->c'), 'ac');
  // A nested opener does not nest: the first `-->` closes the comment, and the rest is text.
  assert.equal(stripComments('a<!-- <!-- b -->c-->d'), 'ac-->d');
  // Two comments in a row, and a closer with no opener, which is text.
  assert.equal(stripComments('<!-- a --><!-- b -->c-->'), 'c-->');
  assert.equal(stripComments('a<!-->b<!--->c'), 'abc');
  assert.equal(stripComments('a<!--b--!>c'), 'ac');
  assert.equal(stripComments('a<!--b -- c-->d'), 'ad');
  assert.equal(stripComments('a<!-- never closed'), 'a');
});

test('elements are split from the text around them by scanning', () => {
  const { inside, outside } = splitElements('a<pre>1</pre>b<prefix>c<pre class="x">2</pre>d<pre>3', 'pre');
  assert.deepEqual(inside, ['<pre>1</pre>', '<pre class="x">2</pre>', '<pre>3']);
  assert.equal(outside, 'ab<prefix>cd');
});

test('table rows split at unescaped pipes only', () => {
  assert.deepEqual(splitRow('| `a` | b \\| c | d |'), [' `a` ', ' b \\| c ', ' d ']);
  assert.equal(splitRow('not a row'), null);
});

test('register entries get an anchor named by their key and a status badge', () => {
  const src = [
    '| Key | Problem | Proposed resolution | Status |',
    '| :--- | :--- | :--- | :--- |',
    '| `docs-site` | A problem. | A resolution with a \\| pipe. | done: [x](x.md) (commit "c") |',
    '| `open-one` | P | R | open |',
    '| `not-an-entry` | no status cell |',
  ].join('\n');
  const { body, counts } = annotateRegister(src);
  assert.deepEqual(counts, { open: 1, proposed: 0, decided: 0, done: 1 });
  const lines = body.split('\n');
  assert.equal(
    lines[2],
    '| <a id="docs-site"></a>`docs-site` | A problem. | A resolution with a \\| pipe. | <span class="status status-done">done</span>: [x](x.md) (commit "c") |',
  );
  assert.match(lines[3], /<a id="open-one"><\/a>.*status-open/);
  assert.equal(lines[4], src.split('\n')[4]);
});

test('checklist items are counted', () => {
  assert.deepEqual(countTasks('- [x] a\n- [ ] b\n  - [X] c\n```\n- [ ] in code\n```\n'), { done: 2, open: 1 });
});

const CONTEXT = {
  page: 'spec/core.md',
  inBook: (p) => ['spec/README.md', 'docs/STATUS.md', 'ROADMAP.md'].includes(p),
  kindOf: (p) => ({ 'compiler/a.rs': 'file', examples: 'dir' })[p] ?? null,
  repoUrl: 'https://github.com/o/r',
  ref: 'm1',
};

test('a link to a page of the book is kept, and one to another repository file goes to GitHub', () => {
  assert.deepEqual(resolveLink('README.md#how', CONTEXT), { url: 'README.md#how' });
  assert.deepEqual(resolveLink('../ROADMAP.md#protocols', CONTEXT), { url: '../ROADMAP.md#protocols' });
  assert.deepEqual(resolveLink('../compiler/a.rs', CONTEXT), { url: 'https://github.com/o/r/blob/m1/compiler/a.rs' });
  assert.deepEqual(resolveLink('../examples/', CONTEXT), { url: 'https://github.com/o/r/tree/m1/examples' });
  assert.deepEqual(resolveLink('https://example.com/x', CONTEXT), { url: 'https://example.com/x' });
  assert.deepEqual(resolveLink('#local', CONTEXT), { url: '#local' });
});

test('a link to nothing in the repository, or outside it, is broken', () => {
  assert.equal(resolveLink('../compiler/gone.rs', CONTEXT).broken, true);
  assert.equal(resolveLink('../../outside.md', CONTEXT).broken, true);
});

test('links are rewritten outside code only, reference definitions included', () => {
  const src = [
    'See [status](../docs/STATUS.md) and [code](../compiler/a.rs "title").',
    'Not in code: `[x](../compiler/gone.rs)`.',
    '```',
    '[x](../compiler/gone.rs)',
    '```',
    '[ref]: ../compiler/a.rs',
    'Broken: [gone](../compiler/gone.rs).',
  ].join('\n');
  const { body, broken } = rewriteLinks(src, CONTEXT);
  const lines = body.split('\n');
  assert.equal(lines[0], 'See [status](../docs/STATUS.md) and [code](https://github.com/o/r/blob/m1/compiler/a.rs "title").');
  assert.equal(lines[1], src.split('\n')[1]);
  assert.equal(lines[3], src.split('\n')[3]);
  assert.equal(lines[5], '[ref]: https://github.com/o/r/blob/m1/compiler/a.rs');
  assert.deepEqual(broken, ['../compiler/gone.rs']);
});

test('the header goes after the first level-one heading', () => {
  assert.equal(insertAfterTitle('```\n# not a title\n```\n# Title\ntext', 'HEADER'), '```\n# not a title\n```\n# Title\n\nHEADER\ntext');
  assert.equal(insertAfterTitle('no title', 'HEADER'), 'HEADER\n\nno title');
});

test('the list mn explain prints is read code by code', () => {
  assert.deepEqual(parseExplainList('E0101  A name was expected\nE0310  A view outlives it (reserved)\n'), [
    { code: 'E0101', title: 'A name was expected', status: 'emitted' },
    { code: 'E0310', title: 'A view outlives it', status: 'reserved' },
  ]);
  assert.throws(() => parseExplainList('something else\n'), /cannot read/);
});

test('an entry mn explain prints is read with its programs', () => {
  const emitted = parseExplain([
    'E0113: Comparisons do not chain',
    '',
    'First paragraph.',
    '',
    'Second paragraph.',
    '',
    'Governed by spec/grammar.md. Failure class: syntax.',
    '',
    'This fails with E0113:',
    '',
    '    module m',
    '    ',
    '    pub fn f() -> bool = a < b < c',
    '',
    'This passes:',
    '',
    '    module m',
    '',
  ].join('\n'));
  assert.deepEqual(emitted, {
    code: 'E0113',
    title: 'Comparisons do not chain',
    status: 'emitted',
    explanation: 'First paragraph.\n\nSecond paragraph.',
    spec: 'spec/grammar.md',
    failureClass: 'syntax',
    fails: 'module m\n\npub fn f() -> bool = a < b < c',
    passes: 'module m',
  });
  const reserved = parseExplain([
    'E0601: Units differ (reserved: specified, not reported yet)',
    '',
    'Text.',
    '',
    'Governed by spec/dimensional-units.md. Failure class: type.',
    '',
  ].join('\n'));
  assert.equal(reserved.status, 'reserved');
  assert.equal(reserved.title, 'Units differ');
  assert.equal(reserved.fails, '');
});

test('a directive shown to the reader is an error, one described in inline code is not', () => {
  const page = (main) => `<html><main>${main}</main></html>`;
  assert.deepEqual(directiveErrors('a.html', page('<p>x<!-- @assert-count target="a" --></p>')), []);
  assert.deepEqual(directiveErrors('a.html', page('<p>An <code>&lt;!-- @assert-count --&gt;</code> under a claim.</p>')), []);
  assert.deepEqual(directiveErrors('a.html', page('<pre><code># run every @assert-* directive</code></pre>')), []);
  assert.deepEqual(directiveErrors('a.html', page('<pre><code>&lt;!-- @assert-count target="a" --&gt;</code></pre>')), [
    'a.html: a directive is shown as a code block',
  ]);
  assert.deepEqual(directiveErrors('a.html', page('<p>&lt;!-- @spec-item id="x" --&gt;</p>')), [
    'a.html: a directive is shown as text',
  ]);
  // A directive's text after a comment that a nested opener closed early is shown, and caught.
  assert.deepEqual(directiveErrors('a.html', page('<p><!-- <!-- x --> &lt;!-- @assert-count target="a" --&gt;</p>')), [
    'a.html: a directive is shown as text',
  ]);
  // So is one whose opener straddles a comment the browser removes.
  assert.deepEqual(directiveErrors('a.html', page('<p>&lt;!-<!-- x -->- @assert-count --&gt;</p>')), [
    'a.html: a directive is shown as text',
  ]);
  // An unclosed comment hides everything after it.
  assert.deepEqual(directiveErrors('a.html', page('<p>x<!-- &lt;!-- @assert-count --&gt;</p>')), []);
});

test('the book check finds broken links, missing fragments and links outside the site', () => {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'mente-site-'));
  try {
    fs.mkdirSync(path.join(dir, 'spec'));
    fs.writeFileSync(path.join(dir, 'index.html'), '<main><h1 id="top">T</h1><a href="spec/a.html#rule">ok</a><a href="#top">self</a><a href="https://x.org/">ext</a></main>');
    fs.writeFileSync(path.join(dir, 'spec', 'a.html'), [
      '<main><p id="rule">R</p>',
      '<a href="../index.html#top">ok</a>',
      '<a href="../index.html#missing">no such fragment</a>',
      '<a href="b.html">no such page</a>',
      '<a href="/Mente/index.html">ok, under the site address</a>',
      '<a href="/elsewhere/">outside</a>',
      '<a href="../">ok, a directory with an index</a></main>',
    ].join('\n'));
    const { pages, errors } = checkBook(dir, '/Mente/');
    assert.equal(pages, 2);
    assert.deepEqual(errors, [
      'spec/a.html: ../index.html#missing names a fragment the page does not have',
      'spec/a.html: broken link b.html',
      "spec/a.html: /elsewhere/ points outside the site's address /Mente/",
    ]);
  } finally {
    fs.rmSync(dir, { recursive: true, force: true });
  }
});
