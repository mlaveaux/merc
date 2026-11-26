---
name: documentation
description: Decide where documentation for a merc change belongs (rustdoc, crate README, the merc-website, or merc-verified) and how to link between them. Use when writing or editing doc comments, a crate README, or a design explanation, or when a code change alters behaviour that a website page describes.
---

# Where documentation goes

`~/merc-website` and `~/merc-verified` are separate repositories that may not be checked out. When they are absent, write the contract in rustdoc and the crate README, do not invent website links, and name in your summary the explanation that belongs on the website so the maintainer can move it.

Each kind of content has one home. Never write the same explanation in two places; link to its home instead. The full policy for human readers is the website page `docs/developer/documentation.md` in `~/merc-website`.

| Home | Content |
|---|---|
| rustdoc | The contract: preconditions, postconditions, `# Panics`, `# Safety`, invariants. A short definition only when the contract needs it. |
| crate `README.md` | Purpose, a minimal usage example, entry points, links to website pages and papers. Usually included as the crate docs via `#![doc = include_str!("../README.md")]`. |
| `~/merc-website` | Definitions, algorithms (LaTeX pseudocode), correctness arguments, merc-specific choices compared with mCRL2 or the literature, measurements, known gaps, architecture across crates. Write pages following `~/merc-website/.agents/skills/write-page/SKILL.md`. |
| `~/merc-verified` | Lean statements and proofs. |
| `docs/*.md` in merc | Working plans only. Once a design lands, move its explanation to the website and shrink or delete the plan. |

Signs that a doc comment is in the wrong place: it explains why an alternative was rejected, cites measurements, derives a property, or is longer than the item it documents. Move that text to the website and link to it.

# Linking to the website

Links go from the less stable side to the more stable side, and stay few:

- **Crate docs** (normally the README) link to the website pages about the crate.
- **An item that implements an algorithm** described on the website links to that section, e.g. `/// See [the one-point rule](https://mercorg.github.io/merc-website/developer/rewriting/enumeration/#the-one-point-rule).`
- Nothing else links to the website. Ordinary items document their contract and use intra-doc links to other items.

Rules:

- Base URL: `https://mercorg.github.io/merc-website/`. A page `docs/developer/x/y.md` is served at `developer/x/y/`, and the anchor of a heading is its lowercase text with spaces replaced by `-` and punctuation dropped.
- Before adding a link, check that the page and anchor exist: build the site (`.venv/bin/zensical build` in `~/merc-website`) and look at `site/<path>/index.html`.
- Module-level `//!` docs of a private `mod` that is flattened with `pub use` never render. Put the link on the crate docs or on the item itself.
- The website links back only to crates or modules, never to individual items, so renaming an item never breaks the website.

# Keeping code and website in sync

When a change alters behaviour described on a website page (a new choice, a removed optimisation, a gap that is now closed), update that page in the same piece of work and mention it in the summary to the user. Renaming or removing an item that the website names in text (not as a link) also needs a check of the page.
