# Documentation maintenance and GitHub Pages

[简体中文](../zh-CN/Documentation.md) · [Home / 首页](Home.md)

## Choice of documentation system

“Get book” likely refers to **GitBook**. Its current product offers hosted documentation with GitHub/GitLab sync. That sync is not the same as building a static artifact in this repository for GitHub Pages.

| Approach | Example / deployment | Fit here |
| --- | --- | --- |
| mdBook | The Rust Programming Language and mdBook's own guide; Markdown → static HTML → Pages | Chosen: Rust ecosystem, navigation/search, no frontend package tree |
| Docusaurus | Docusaurus documentation; Node build → static output → Pages | Good for richer React documentation; more configuration/dependencies |
| GitBook | Hosted docs with repository sync | Useful hosted editing, but not needed for a same-repository Pages artifact |

Sources: [mdBook introduction](https://rust-lang.github.io/mdBook/), [mdBook CI](https://rust-lang.github.io/mdBook/continuous-integration.html), [Docusaurus deployment](https://docusaurus.io/docs/deployment), [GitBook Git Sync](https://www.gitbook.com/features/git-sync), [GitHub custom Pages workflows](https://docs.github.com/en/pages/getting-started-with-github-pages/using-custom-workflows-with-github-pages). Reviewed 2026-10-11; this project pins mdBook **0.4.52** for reproducibility, not as a claim that it is the newest release.

## Source and output

```text
wiki/README.md          repository documentation entry
wiki/en/               English pages, SUMMARY.md, book.toml
wiki/zh-CN/            matching Chinese pages, SUMMARY.md, book.toml
wiki/archive/          original Wiki snapshot + SHA-256 manifest; not published
doc/                   existing website and assets
scripts/build-docs.py   validate, build both books, merge website, check output
_site/                 generated Pages artifact, ignored by Git
```

Every current page has an explicit link to the same topic in the other language. Keep filenames and chapter order paired; Markdown links work in the repository and mdBook converts them to HTML links. Archive records remain in their original language as historical evidence.

## Local build and preview

Install Python 3.9+ and mdBook 0.4.52 from its [official release](https://github.com/rust-lang/mdBook/releases/tag/v0.4.52), or install the documentation tool using Cargo:

```sh
cargo install mdbook --version 0.4.52 --locked
python3 scripts/build-docs.py
python3 -m http.server 8000 --directory _site
```

Open `http://localhost:8000/wiki/`, then choose English or 简体中文. On Windows use `py -3` if `python3` is unavailable. `MDBOOK` may specify the executable path. For source-only validation without mdBook, run `python3 scripts/build-docs.py --check`.

This builds documentation only. It does not build, test or start the desktop/server application. Output is regenerated under `_site/`; keep source changes in `wiki/` and `doc/`.

## Deploy to the existing Pages site

In repository **Settings → Pages → Build and deployment**, select **GitHub Actions**. The `github-pages` environment must allow main. Merge/push changes to main, or manually run **Deploy GitHub Pages**. The workflow:

1. Checks out the main repository (no Wiki clone).
2. Installs the pinned mdBook Linux binary and checks its SHA-256.
3. Runs the source checks, builds both languages and copies the website into one `_site/` artifact.
4. Uploads that single artifact and deploys it using the existing Pages environment.

Pull requests touching documentation also build/check the site, without deployment permissions. Push/manual deployment requires `pages: write` and `id-token: write`; no personal access token or separate `gh-pages` branch is required.

Expected URLs after deployment:

- Website: `https://zhaiyanqi.github.io/vclogg2/`
- Language chooser: `https://zhaiyanqi.github.io/vclogg2/wiki/`
- English: `https://zhaiyanqi.github.io/vclogg2/wiki/en/`
- Chinese: `https://zhaiyanqi.github.io/vclogg2/wiki/zh-CN/`

The landing page and documentation must be deployed together: a second Pages deployment would replace the first artifact. The workflow triggers on `doc/**`, `wiki/**`, the docs build script and the workflow itself. Do not add credentials or runtime files to these public source directories.

GitHub's old Wiki URLs cannot be redirected by Pages. See [migration](Archive.md) for the retained source snapshot and optional old-Wiki notice. A local successful build does not prove the remote deployment has run; check the Actions deployment result after pushing.

## Updating content

Use current code as the source for behavior, dependencies and module ownership. Change both language pages and SUMMARY entries together; preserve UI IDs. Keep historical test results in the archive and identify newly executed checks accurately. Build scripts and release documentation must agree. Run source validation and a full documentation build before publishing.

The repository Pages API was checked on 2026-10-11: its build type is already `workflow` (GitHub Actions). No settings change was made.
