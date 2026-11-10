// Builds Mente's documentation site with mdBook from the repository's own sources
// (docs/design-review.md, `docs-site`): the specification, the language card, the error-code
// catalogue as `mn explain` prints it, the implementation status, the design register and the
// roadmap. Nothing is written twice by hand: each page is generated from its source at build time,
// with the maturity of what it describes shown on it.
//
//   node site/build.mjs [--out target/site] [--mn <mn binary>] [--mdbook <mdbook binary>]
//                       [--tag <milestone tag>] [--ref <git ref>] [--no-mdbook]
//
// Environment: SITE_TAG, SITE_REF, MN and MDBOOK stand for the options of the same name, and
// GITHUB_REPOSITORY names the repository the source links point at.

import { execFileSync } from 'node:child_process';
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { checkBook } from './check.mjs';
import {
  LABELS,
  annotateRegister,
  countTasks,
  escapeHtml,
  firstTitle,
  insertAfterTitle,
  labelMeanings,
  markClaims,
  maturityBadge,
  parseExplain,
  parseExplainList,
  rewriteLinks,
  splitFrontMatter,
  statusBadge,
} from './markdown.mjs';

const ROOT = path.resolve(fileURLToPath(new URL('..', import.meta.url)));

/** The tests the site names as the evidence that its snippets compile. */
const CARD_SNIPPET_TEST = 'test_language_card_snippets_pass_mn_check';
const CATALOGUE_SNIPPET_TEST = 'test_each_reported_code_fails_its_program_and_passes_its_near_miss';

/** How an error code's catalogue status reads as maturity. */
const CODE_STATUS = {
  emitted: { label: 'Implemented', title: 'Emitted: the compiler reports this code today.' },
  reserved: { label: 'Specified', title: 'Reserved: a spec defines this code; no check reports it yet.' },
  retired: { label: null, title: 'Retired: withdrawn, and never given to anything else.' },
};

function parseArgs(argv) {
  const args = {
    out: 'target/site',
    mn: process.env.MN || null,
    mdbook: process.env.MDBOOK || 'mdbook',
    tag: process.env.SITE_TAG || null,
    ref: process.env.SITE_REF || null,
    runMdbook: true,
  };
  for (let i = 0; i < argv.length; i += 1) {
    const a = argv[i];
    const value = () => {
      const v = argv[i + 1];
      if (v === undefined) throw new Error(`${a} needs a value`);
      i += 1;
      return v;
    };
    if (a === '--out') args.out = value();
    else if (a === '--mn') args.mn = value();
    else if (a === '--mdbook') args.mdbook = value();
    else if (a === '--tag') args.tag = value() || null;
    else if (a === '--ref') args.ref = value() || null;
    else if (a === '--no-mdbook') args.runMdbook = false;
    else throw new Error(`unknown option ${a}`);
  }
  return args;
}

function read(rel) {
  return fs.readFileSync(path.join(ROOT, rel), 'utf8').replace(/\r\n/g, '\n');
}

function kindOf(rel) {
  try {
    const stat = fs.statSync(path.join(ROOT, rel));
    return stat.isDirectory() ? 'dir' : 'file';
  } catch {
    return null;
  }
}

function git(...args) {
  return execFileSync('git', args, { cwd: ROOT, encoding: 'utf8' }).trim();
}

/** Fails unless a test of that name exists in the compiler, so the site never cites one that is gone. */
function requireTest(name) {
  const stack = [path.join(ROOT, 'compiler')];
  while (stack.length > 0) {
    const dir = stack.pop();
    for (const entry of fs.readdirSync(dir, { withFileTypes: true })) {
      const full = path.join(dir, entry.name);
      if (entry.isDirectory()) {
        if (entry.name !== 'target') stack.push(full);
      } else if (entry.name.endsWith('.rs') && fs.readFileSync(full, 'utf8').includes(`fn ${name}(`)) {
        return;
      }
    }
  }
  throw new Error(`the site cites the test ${name}, which no longer exists in compiler/`);
}

/** The `mn` binary: the one given, or the workspace's debug build. */
function mnBinary(given) {
  if (given) return path.resolve(given);
  execFileSync('cargo', ['build', '--quiet', '--locked', '--bin', 'mn'], { cwd: ROOT, stdio: 'inherit' });
  const targetDir = process.env.CARGO_TARGET_DIR || path.join(ROOT, 'target');
  return path.join(targetDir, 'debug', process.platform === 'win32' ? 'mn.exe' : 'mn');
}

/** The spec files in the order of the authority map in spec/README.md, then any it does not list. */
function specOrder(specReadme) {
  const listed = [];
  for (const line of specReadme.split('\n')) {
    const m = /^\|\s*\[[^\]]+\]\(([a-z0-9-]+\.md)\)\s*\|/.exec(line);
    if (m && !listed.includes(m[1])) listed.push(m[1]);
  }
  const all = fs
    .readdirSync(path.join(ROOT, 'spec'))
    .filter((f) => f.endsWith('.md') && f !== 'README.md')
    .sort();
  const missing = listed.filter((f) => !all.includes(f));
  if (missing.length > 0) throw new Error(`spec/README.md lists files that do not exist: ${missing.join(', ')}`);
  return [...listed, ...all.filter((f) => !listed.includes(f))].map((f) => `spec/${f}`);
}

function summaryLink(title, target) {
  return `[${title.replace(/[[\]]/g, '\\$&')}](${target})`;
}

function countsLine(counts, badge) {
  return Object.entries(counts)
    .filter(([, n]) => n > 0)
    .map(([key, n]) => `${badge(key)} ${n}`)
    .join(' · ');
}

function main() {
  const args = parseArgs(process.argv.slice(2));
  const out = path.resolve(ROOT, args.out);
  const src = path.join(out, 'src');
  const bookDir = path.join(out, 'book');
  fs.rmSync(src, { recursive: true, force: true });
  fs.rmSync(bookDir, { recursive: true, force: true });
  fs.mkdirSync(src, { recursive: true });

  const repository = process.env.GITHUB_REPOSITORY || 'Menteion/Mente';
  const repoUrl = `https://github.com/${repository}`;
  const siteUrl = `/${repository.split('/')[1]}/`;
  const ref = args.tag || args.ref || git('rev-parse', 'HEAD');
  const shortRef = /^[0-9a-f]{40}$/.test(ref) ? ref.slice(0, 10) : ref;
  const edition = args.tag
    ? `milestone tag <code>${escapeHtml(args.tag)}</code>`
    : `an unreleased build of commit <code>${escapeHtml(shortRef)}</code>, not published`;

  requireTest(CARD_SNIPPET_TEST);
  requireTest(CATALOGUE_SNIPPET_TEST);

  const specReadme = read('spec/README.md');
  const meanings = labelMeanings(specReadme);
  const specFiles = specOrder(specReadme);

  // The error-code catalogue, as `mn explain` prints it.
  const mn = mnBinary(args.mn);
  const explain = (...a) => execFileSync(mn, ['explain', ...a], { cwd: ROOT, encoding: 'utf8' });
  const codes = parseExplainList(explain()).map((listed) => {
    const entry = parseExplain(explain(listed.code));
    if (entry.code !== listed.code || entry.title !== listed.title || entry.status !== listed.status) {
      throw new Error(`mn explain ${listed.code} disagrees with the list mn explain prints`);
    }
    return entry;
  });

  // Every page of the book, at its repository path so that relative links keep working.
  const sourcePages = ['README.md', 'spec/README.md', ...specFiles, 'docs/language-card.md',
    'docs/STATUS.md', 'docs/design-review.md', 'ROADMAP.md'];
  const generatedPages = ['errors/README.md', ...codes.map((e) => `errors/${e.code}.md`)];
  const book = new Set([...sourcePages, ...generatedPages]);
  const broken = [];
  const specCounts = new Map();
  const titles = new Map();

  const prerelease = (prefix) =>
    `<div class="site-banner" role="note">\n\n` +
    `<strong>Pre-release, unstable.</strong> This is the documentation of Mente at ${edition}. ` +
    `The language and these pages change without notice, and nothing here promises compatibility. ` +
    `The [implementation status](${prefix}docs/STATUS.md) says what the compiler does today.\n\n</div>`;

  const write = (rel, body) => {
    const file = path.join(src, rel);
    fs.mkdirSync(path.dirname(file), { recursive: true });
    fs.writeFileSync(file, body.endsWith('\n') ? body : `${body}\n`);
  };

  // First pass: transform each source and record what the overview pages need.
  const transformed = new Map();
  for (const rel of sourcePages) {
    const { meta, body: raw } = splitFrontMatter(read(rel));
    let body = raw;
    titles.set(rel, meta.title || firstTitle(raw) || rel);
    let summary = '';
    if (rel.startsWith('spec/')) {
      const marked = markClaims(body, meanings);
      body = marked.body;
      specCounts.set(rel, marked.counts);
      const total = LABELS.reduce((n, l) => n + marked.counts[l], 0);
      if (total > 0) summary = `Claims on this page: ${countsLine(marked.counts, (l) => maturityBadge(l))}.`;
    } else if (rel === 'docs/design-review.md') {
      const annotated = annotateRegister(body);
      body = annotated.body;
      summary = `Entries by status: ${countsLine(annotated.counts, (s) => statusBadge(s))}.`;
    } else if (rel === 'ROADMAP.md') {
      const tasks = countTasks(body);
      summary = `A checked item is done: ${tasks.done} checked, ${tasks.open} unchecked.`;
    } else if (rel === 'docs/language-card.md') {
      summary = `Every \`mente\` block on this page passes \`mn check\`, held to it by the test \`${CARD_SNIPPET_TEST}\`.`;
    } else if (rel === 'docs/STATUS.md') {
      summary = 'This page is the ledger of what the compiler does today.';
    } else if (rel === 'README.md') {
      summary = 'Most of the design this page describes is not implemented yet.';
    }
    transformed.set(rel, { meta, body, summary });
  }

  for (const rel of sourcePages) {
    const { meta, body: transformedBody, summary: pageSummary } = transformed.get(rel);
    const prefix = '../'.repeat(rel.split('/').length - 1);
    let summary = pageSummary;
    if (rel === 'spec/README.md') {
      const rows = specFiles.map((f) => {
        const c = specCounts.get(f);
        return `| [${titles.get(f).replace(/\|/g, '\\|')}](${path.posix.basename(f)}) | ${LABELS.map((l) => c[l] || '').join(' | ')} |`;
      });
      const totals = LABELS.map((l) => specFiles.reduce((n, f) => n + specCounts.get(f)[l], 0));
      summary =
        `Claims by file and maturity. Each label's tooltip on a page says what it means.\n\n` +
        `| File | ${LABELS.map((l) => maturityBadge(l)).join(' | ')} |\n` +
        `| :--- | ${LABELS.map(() => '---:').join(' | ')} |\n` +
        `${rows.join('\n')}\n| **All files** | ${totals.join(' | ')} |`;
    } else if (rel.startsWith('spec/') && summary) {
      summary += ` [What the labels mean](${prefix}spec/README.md#how-to-read-this-specification).`;
    }
    const control = [
      meta.version && `version ${escapeHtml(meta.version)}`,
      meta.status && `status <em>${escapeHtml(meta.status)}</em>`,
      meta.date && escapeHtml(meta.date),
      `source <a href="${repoUrl}/blob/${ref}/${rel}"><code>${rel}</code></a>`,
    ].filter(Boolean);
    const header = [
      prerelease(prefix),
      `<p class="doc-meta">${control.join(' · ')}</p>`,
      summary && `<div class="maturity-summary">\n\n${summary}\n\n</div>`,
    ].filter(Boolean).join('\n\n');
    const withHeader = insertAfterTitle(transformedBody, header);
    const rewritten = rewriteLinks(withHeader, {
      page: rel, inBook: (p) => book.has(p), kindOf, repoUrl, ref,
    });
    broken.push(...rewritten.broken.map((u) => `${rel}: ${u}`));
    write(rel, rewritten.body);
  }

  // The error-code catalogue: an index and one page per code.
  const tally = { emitted: 0, reserved: 0, retired: 0 };
  for (const e of codes) tally[e.status] += 1;
  const codeBadge = (status) => {
    const s = CODE_STATUS[status];
    return s.label ? maturityBadge(s.label, s.title).replace(`>${s.label}<`, `>${status}<`) : statusBadge(status, s.title);
  };
  const indexRows = codes.map((e) =>
    `| [${e.code}](${e.code}.md) | ${e.title.replace(/\|/g, '\\|')} | ${codeBadge(e.status)} | ${e.failureClass} |`);
  write('errors/README.md', [
    '# Error codes',
    '',
    prerelease('../'),
    '',
    `<div class="maturity-summary">\n\n${countsLine(tally, codeBadge)}. An emitted code is reported by the compiler today ` +
      `(${maturityBadge('Implemented')}); a reserved one is defined by a spec and not reported yet ` +
      `(${maturityBadge('Specified')}).\n\n</div>`,
    '',
    `Every code the compiler reports or reserves, one page each, generated from what \`mn explain\` prints. ` +
      `The catalogue lives in [\`compiler/mente-cli/src/catalogue.rs\`](${repoUrl}/blob/${ref}/compiler/mente-cli/src/catalogue.rs), ` +
      `and [diagnostics](../spec/diagnostics.md) specifies it. A code is never reused.`,
    '',
    '| Code | Title | Status | Failure class |',
    '| :--- | :--- | :--- | :--- |',
    ...indexRows,
  ].join('\n'));
  titles.set('errors/README.md', 'Error codes');

  for (const e of codes) {
    if (!book.has(e.spec)) throw new Error(`${e.code} is governed by ${e.spec}, which is not a page of the site`);
    const lines = [
      `# ${e.code}: ${e.title}`,
      '',
      prerelease('../'),
      '',
      `<p class="doc-meta">${codeBadge(e.status)} · failure class <em>${escapeHtml(e.failureClass)}</em> · governed by ` +
        `<a href="../${e.spec.replace(/\.md$/, '.html')}"><code>${e.spec}</code></a></p>`,
      '',
      e.explanation,
    ];
    if (e.fails) {
      lines.push('', `## This fails with ${e.code}`, '', '```mente', e.fails, '```',
        '', '## This passes', '', '```mente', e.passes, '```',
        '', `Both programs are checked by the test \`${CATALOGUE_SNIPPET_TEST}\`.`);
    }
    lines.push('', `<p class="doc-meta">Generated from <code>mn explain ${e.code}</code>.</p>`);
    write(`errors/${e.code}.md`, lines.join('\n'));
  }

  if (broken.length > 0) {
    throw new Error(`links to nothing in the repository:\n  ${broken.join('\n  ')}`);
  }

  // The table of contents and the book's configuration.
  const summaryLines = [
    '# Summary',
    '',
    summaryLink('Mente', 'README.md'),
    '',
    `- ${summaryLink('Specification', 'spec/README.md')}`,
    ...specFiles.map((f) => `    - ${summaryLink(titles.get(f), f)}`),
    `- ${summaryLink('Language card', 'docs/language-card.md')}`,
    `- ${summaryLink('Error codes', 'errors/README.md')}`,
    ...codes.map((e) => `    - ${summaryLink(`${e.code}: ${e.title}`, `errors/${e.code}.md`)}`),
    `- ${summaryLink('Implementation status', 'docs/STATUS.md')}`,
    `- ${summaryLink('Design register', 'docs/design-review.md')}`,
    `- ${summaryLink('Roadmap', 'ROADMAP.md')}`,
  ];
  write('SUMMARY.md', summaryLines.join('\n'));

  const title = args.tag ? `Mente ${args.tag} (pre-release, unstable)` : 'Mente (unreleased build)';
  const toml = (s) => JSON.stringify(s);
  fs.writeFileSync(path.join(out, 'book.toml'), [
    '# Generated by site/build.mjs; edit that script, not this file.',
    '[book]',
    `title = ${toml(title)}`,
    `description = ${toml('The specification, language card, error codes and status of Mente, a systems language for AI agents.')}`,
    'language = "en"',
    'src = "src"',
    '',
    '[build]',
    'build-dir = "book"',
    'create-missing = false',
    // No `{{#include}}` or README-to-index rewriting: the sources are shown as written.
    'use-default-preprocessors = false',
    '',
    '[output.html]',
    `site-url = ${toml(siteUrl)}`,
    `git-repository-url = ${toml(repoUrl)}`,
    'additional-css = ["maturity.css"]',
    'no-section-label = true',
    '',
    '[output.html.playground]',
    'runnable = false',
    '',
    '[output.html.fold]',
    'enable = true',
    'level = 0',
    '',
  ].join('\n'));
  fs.copyFileSync(path.join(ROOT, 'site', 'maturity.css'), path.join(out, 'maturity.css'));

  const claimTotals = LABELS.map((l) => `${specFiles.reduce((n, f) => n + specCounts.get(f)[l], 0)} ${l}`);
  console.log(`site sources: ${book.size} pages in ${path.relative(ROOT, src)}`);
  console.log(`  claims: ${claimTotals.join(', ')}`);
  console.log(`  error codes: ${tally.emitted} emitted, ${tally.reserved} reserved, ${tally.retired} retired`);
  console.log(`  edition: ${args.tag ? `tag ${args.tag}` : `unreleased, ${shortRef}`}`);

  if (!args.runMdbook) return;
  execFileSync(args.mdbook, ['build', out], { cwd: out, stdio: 'inherit' });
  const { pages, errors } = checkBook(bookDir, siteUrl);
  for (const e of errors) console.error(e);
  if (errors.length > 0) throw new Error(`the built site has ${errors.length} problems`);
  console.log(`site: ${pages} HTML pages in ${path.relative(ROOT, bookDir)}, every link resolves`);
}

try {
  main();
} catch (e) {
  console.error(`site/build.mjs: ${e.message}`);
  process.exit(1);
}
