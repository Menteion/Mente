# The documentation site

Mente's documentation site, as the register entry [`docs-site`](../docs/design-review.md#docs-site) decided it: an [mdBook](https://rust-lang.github.io/mdBook/) book built by CI from the repository's own sources, never written as a second copy. This folder holds only the generator; the pages come from:

| Page | Source |
| :--- | :--- |
| Introduction | [`README.md`](../README.md) |
| Specification | [`spec/README.md`](../spec/README.md), then every `spec/*.md` in the order of its authority map |
| Language card | [`docs/language-card.md`](../docs/language-card.md) |
| Error codes | `mn explain`, one page per code, from [`catalogue.rs`](../compiler/mente-cli/src/catalogue.rs) |
| Implementation status | [`docs/STATUS.md`](../docs/STATUS.md) |
| Design register | [`docs/design-review.md`](../docs/design-review.md) |
| Roadmap | [`ROADMAP.md`](../ROADMAP.md), which holds the measurement protocol every `Target` cites |

## What the generator adds

[`build.mjs`](build.mjs) copies each source into the book at its repository path, so relative links between pages keep working, and changes only what a reader of the site needs, using the functions of [`markdown.mjs`](markdown.mjs):

- **Maturity on every page.** Each claim's label becomes a badge whose tooltip says what the label means (read from the label table of `spec/README.md`) and, for an `Implemented` claim, names the tests its directives cite. Each spec page states how many claims it has under each label, and the specification's index tabulates them by file. The register shows each entry's status, an error code shows whether it is emitted or reserved, the roadmap counts its checked items, and the language card names the test that compiles its snippets.
- **A pre-release banner** on every page, naming the milestone tag the site was built from, or the commit of an unreleased build.
- **Front matter** becomes one line under the title: version, status, date and a link to the source on GitHub.
- **Links.** A link to a page of the book stays as it is; a link to any other file of the repository goes to that file on GitHub, at the tag or commit being built; a link to nothing fails the build. Register entries get an anchor named by their key, so `design-review.md#key` resolves.
- **Mente's logo** above the introduction's title, and the favicon, from [`assets/`](assets): [`logo.css`](logo.css) shows the logo's light version on mdBook's light themes and its dark version, the same path in a light ink, on the dark ones. mdBook takes the favicon from the book's `theme/` folder, which holds nothing else.
- **Directives stay hidden.** `<!-- @assert-* -->` and `<!-- @spec-item -->` comments pass through as HTML comments, which a browser does not show.

[`check.mjs`](check.mjs) then reads the built HTML and fails on a relative link to a missing page, a fragment the target page does not have, a link outside the site's address, or a directive rendered as text.

## Snippets

The only Mente code the site shows is the language card's `mente` blocks, which `test_language_card_snippets_pass_mn_check` compiles, and each error code's failing program and near miss, which `test_each_reported_code_fails_its_program_and_passes_its_near_miss` checks. The build fails if either test is renamed or removed. Code blocks inside `spec/*.md` are shown as written and are not compiled by a test yet.

## Building it locally

With [mdBook 0.5.4](https://github.com/rust-lang/mdBook/releases/tag/v0.5.4) on the `PATH` (or named with `--mdbook`):

```powershell
npm run site
npm run test:site
```

The book is written to `target/site/book`; open `index.html` in it. `--no-mdbook` writes only the generated sources, to `target/site/src`. `--tag m1` builds the edition a milestone tag publishes, and `--mn <path>` uses an `mn` binary that is already built.

## Publishing

[`.github/workflows/pages.yml`](../.github/workflows/pages.yml) builds and checks the site on every pull request that touches its sources, and keeps the result as the workflow artifact `docs-site`. On a milestone tag (`m*`) it publishes the site to GitHub Pages, marked pre-release and unstable. The repository's `github-pages` environment must allow the tag: a deployment tag rule `m*` beside the default branch.
