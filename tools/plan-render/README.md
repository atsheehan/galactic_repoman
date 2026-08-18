# plan-render

Renders the `plans/**/*.md` documents to styled, self-contained HTML.

```sh
scripts/render-plans.sh                          # all of them → target/plans/
scripts/render-plans.sh plans/foo.md             # just this one
OPEN=1 scripts/render-plans.sh plans/foo.md      # ...and open it
```

Output is one HTML file per document with the CSS inlined, so it opens straight from disk
with no server and no network (bar the Google Fonts link, which falls back cleanly offline).

This is a **standalone crate**, deliberately not a workspace member of `galactic_repoman` —
`cargo build` at the repo root never pulls `pulldown-cmark` into the game's dependency tree.

## Keeping the Markdown portable

Everything this tool adds beyond CommonMark is written as an HTML comment, which every
other Markdown renderer — GitHub, editors, `less` — drops silently. The `.md` files stay
readable everywhere; they just render richer here.

| Directive | Scope | Effect |
| --- | --- | --- |
| `<!-- title: Some Name -->` | document | Overrides the `<title>`. Defaults to the `#` heading. |
| `<!-- eyebrow: Project / Step 3 / Plan -->` | document | Breadcrumb above the title; split on `/`. |
| `<!-- fact: Label = Value -->` | document | One cell in the masthead facts grid. Repeatable; `<br>` allowed in the value. |
| `<!-- class: NAME -->` | next block | Puts `NAME` on the next table, list, or blockquote. |

Classes worth knowing:

- `corrective` — a three-column table whose middle column is a failure and whose last column
  is the fix. Colour carries the contrast so the argument reads before it is parsed.
- `phases` — a bullet list rendered as stage cards, with the leading `**bold**` as the name.
- `checks` — an ordered list rendered as a zero-padded checklist.

## Inferred from ordinary Markdown

No directive needed for any of this:

- The `#` heading becomes the masthead; the paragraph right after it becomes the standfirst.
- `##` headings build the sticky contents rail, get stable `id` anchors, and are separated by
  a tick-rule divider (a nod to the event streams these plans keep describing).
- A blockquote opening with a bold run that reads as a label — `> **Exit criteria:** …` —
  lifts that run into a label slot. The label also picks the variant: *exit criteria*,
  *payoff*, *decision* render as accented; *alternative*, *risk*, *caveat*, *trade-off* as
  cautionary; anything else neutral. A long bold opener is treated as emphasis, not a label.
- Tables are wrapped so wide ones scroll in their own box rather than pushing the page
  sideways.
- Unrecognized HTML comments pass through untouched.

## Theming

Light and dark are both defined, covering all three viewer states (explicit light, explicit
dark, and the un-stamped system default). The accent is the magenta from the codebase's own
`CLEAR_COLOR = [1.0, 0.0, 1.0, 1.0]` black-screen diagnostic; neutrals carry a violet bias so
they sit with it. Typography is the IBM Plex superfamily — Condensed for headings, Sans for
body, Mono for code and labels.

Palette and type live in `assets/style.css`; the page skeleton is `assets/shell.html`. Both
are `include_str!`-ed into the binary, so a rebuild is all that is needed after editing them.

## Tests

`cargo test --manifest-path tools/plan-render/Cargo.toml` — covers heading slugs and
collisions, label lifting (including labels containing inline code), variant selection,
directive scoping, and the single-pass template fill. That last one matters: the plans
contain GitHub Actions expressions like `${{ vars.STEAM_APP_ID }}`, and a naive chain of
`str::replace` would let a document rewrite the shell around it.
