//! `plan-render` — turns a `plans/*.md` document into a styled, self-contained HTML page.
//!
//! The design goal is that the *Markdown stays portable*. Everything this tool adds beyond
//! CommonMark is expressed as HTML comments, which every other Markdown renderer (GitHub,
//! editors, `less`) drops silently:
//!
//! ```text
//! <!-- eyebrow: Galactic Repoman / Step 3 / Detailed plan -->
//! <!-- fact: Backends = winit 0.30 · gilrs 0.11 -->
//! <!-- class: corrective -->     (applies to the next table / list / blockquote)
//! ```
//!
//! Everything else is inferred from ordinary Markdown: the `#` heading becomes the masthead,
//! the paragraph after it becomes the standfirst, `##` headings build the contents rail and
//! get tick-rule dividers, blockquotes whose first line is a bold `**Label:**` become labelled
//! notes, and tables are wrapped so they scroll inside their own box.
//!
//! Usage: `plan-render <input.md> [-o <output.html>]`
//! Default output is `target/plans/<stem>.html`, which is already gitignored.

use std::collections::HashSet;
use std::ops::Range;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use pulldown_cmark::{Event, HeadingLevel, Options, Parser, Tag, html};

const STYLE: &str = include_str!("../assets/style.css");
const SHELL: &str = include_str!("../assets/shell.html");

fn main() -> Result<()> {
    let args = Args::parse(std::env::args().skip(1))?;

    let markdown = std::fs::read_to_string(&args.input)
        .with_context(|| format!("reading {}", args.input.display()))?;

    let page = render(&markdown, &args.input);

    if let Some(parent) = args.output.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("creating {}", parent.display()))?;
    }
    std::fs::write(&args.output, page)
        .with_context(|| format!("writing {}", args.output.display()))?;

    println!("{}", args.output.display());
    Ok(())
}

struct Args {
    input: PathBuf,
    output: PathBuf,
}

impl Args {
    fn parse(args: impl Iterator<Item = String>) -> Result<Self> {
        let (mut input, mut output) = (None, None);
        let mut args = args.peekable();

        while let Some(arg) = args.next() {
            match arg.as_str() {
                "-o" | "--output" => {
                    output = Some(PathBuf::from(
                        args.next().context("-o needs a path")?,
                    ));
                }
                "-h" | "--help" => {
                    println!("usage: plan-render <input.md> [-o <output.html>]");
                    std::process::exit(0);
                }
                other if other.starts_with('-') => bail!("unknown flag: {other}"),
                other => input = Some(PathBuf::from(other)),
            }
        }

        let input = input.context("usage: plan-render <input.md> [-o <output.html>]")?;
        let output = output.unwrap_or_else(|| {
            let stem = input.file_stem().unwrap_or_default();
            Path::new("target/plans").join(stem).with_extension("html")
        });

        Ok(Self { input, output })
    }
}

// ── Rendering ───────────────────────────────────────────────────────────────

fn render(markdown: &str, source: &Path) -> String {
    let options = Options::ENABLE_TABLES
        | Options::ENABLE_STRIKETHROUGH
        | Options::ENABLE_FOOTNOTES
        | Options::ENABLE_TASKLISTS
        | Options::ENABLE_SMART_PUNCTUATION
        | Options::ENABLE_GFM;

    let events: Vec<Event> = Parser::new_ext(markdown, options).collect();
    let doc = Document::build(&events);

    let heading = doc.heading.unwrap_or_else(|| "Untitled".to_string());
    let title = doc.title.unwrap_or_else(|| strip_tags(&heading));

    let eyebrow = doc
        .eyebrow
        .map(|crumbs| {
            let joined = crumbs
                .iter()
                .map(|c| escape(c))
                .collect::<Vec<_>>()
                .join("<span class=\"sep\">/</span>");
            format!("<p class=\"eyebrow\">{joined}</p>\n")
        })
        .unwrap_or_default();

    let thesis = doc
        .thesis
        .map(|t| format!("<p class=\"thesis\">{t}</p>\n"))
        .unwrap_or_default();

    let facts = if doc.facts.is_empty() {
        String::new()
    } else {
        let items = doc
            .facts
            .iter()
            .map(|(k, v)| format!("  <div><dt>{}</dt><dd>{v}</dd></div>", escape(k)))
            .collect::<Vec<_>>()
            .join("\n");
        format!("<dl class=\"facts\">\n{items}\n</dl>\n")
    };

    let rail = if doc.toc.is_empty() {
        String::new()
    } else {
        let items = doc
            .toc
            .iter()
            .map(|(slug, text)| {
                format!("    <li><a href=\"#{slug}\">{}</a></li>", escape(text))
            })
            .collect::<Vec<_>>()
            .join("\n");
        format!(
            "<nav class=\"rail\" aria-label=\"Contents\">\n  <h2>Contents</h2>\n  <ol>\n{items}\n  </ol>\n</nav>\n"
        )
    };

    let footer = format!(
        "{}<br>\nrendered by tools/plan-render",
        escape(&source.display().to_string())
    );

    fill(
        SHELL,
        &[
            ("STYLE", STYLE),
            ("TITLE", &escape(&title)),
            ("EYEBROW", &eyebrow),
            ("HEADING", &heading),
            ("THESIS", &thesis),
            ("FACTS", &facts),
            ("RAIL", &rail),
            ("BODY", doc.body.trim_end()),
            ("FOOTER", &footer),
        ],
    )
}

/// Expand `{{KEY}}` placeholders in one pass over the template.
///
/// A sequential chain of `str::replace` would substitute into text that earlier
/// substitutions had already inserted — and the plans really do contain `{{ }}` (GitHub
/// Actions expressions such as `${{ vars.STEAM_APP_ID }}`), so a document could otherwise
/// rewrite the shell around it. Unknown placeholders are passed through untouched.
fn fill(template: &str, values: &[(&str, &str)]) -> String {
    let mut out = String::with_capacity(template.len() + STYLE.len());
    let mut rest = template;

    while let Some(open) = rest.find("{{") {
        let Some(close) = rest[open..].find("}}").map(|c| open + c) else {
            break;
        };

        match values.iter().find(|(key, _)| *key == &rest[open + 2..close]) {
            Some((_, value)) => {
                out.push_str(&rest[..open]);
                out.push_str(value);
            }
            None => out.push_str(&rest[..close + 2]),
        }

        rest = &rest[close + 2..];
    }

    out.push_str(rest);
    out
}

#[derive(Default)]
struct Document {
    title: Option<String>,
    heading: Option<String>,
    thesis: Option<String>,
    eyebrow: Option<Vec<String>>,
    facts: Vec<(String, String)>,
    toc: Vec<(String, String)>,
    body: String,
}

impl Document {
    fn build(events: &[Event]) -> Self {
        let mut doc = Self::default();
        let mut slugs = HashSet::new();
        let mut pending_class: Option<String> = None;
        let mut want_thesis = false;
        let mut seen_h2 = false;
        let mut i = 0;

        while i < events.len() {
            match &events[i] {
                // Comment directives. `class:` is positional; the rest are document-level.
                Event::Html(raw) => match directive(raw) {
                    Some(("class", value)) => {
                        pending_class = Some(value);
                        i += 1;
                    }
                    Some(("eyebrow", value)) => {
                        doc.eyebrow =
                            Some(value.split('/').map(|c| c.trim().to_string()).collect());
                        i += 1;
                    }
                    Some(("title", value)) => {
                        doc.title = Some(value);
                        i += 1;
                    }
                    Some(("fact", value)) => {
                        if let Some((k, v)) = value.split_once('=') {
                            doc.facts.push((k.trim().into(), v.trim().into()));
                        }
                        i += 1;
                    }
                    // Not ours — pass raw HTML through untouched.
                    _ => {
                        doc.body.push_str(raw);
                        i += 1;
                    }
                },

                Event::Start(Tag::Heading { level, .. }) => {
                    let level = *level;
                    let (inner, next) = span(events, i);

                    if level == HeadingLevel::H1 && doc.heading.is_none() {
                        doc.heading = Some(render_events(&events[inner]));
                        want_thesis = true;
                        i = next;
                        continue;
                    }

                    want_thesis = false;
                    let text = plain_text(&events[inner.clone()]);
                    let slug = unique_slug(&text, &mut slugs);

                    if level == HeadingLevel::H2 {
                        if seen_h2 {
                            doc.body.push_str("<hr class=\"tick-rule\">\n");
                        }
                        seen_h2 = true;
                        doc.toc.push((slug.clone(), text));
                    }

                    let n = level as u8;
                    doc.body.push_str(&format!(
                        "<h{n} id=\"{slug}\">{}</h{n}>\n",
                        render_events(&events[inner])
                    ));
                    i = next;
                }

                // The paragraph immediately after the `#` heading is the standfirst.
                Event::Start(Tag::Paragraph) if want_thesis => {
                    let (inner, next) = span(events, i);
                    doc.thesis = Some(render_events(&events[inner]));
                    want_thesis = false;
                    i = next;
                }

                Event::Start(Tag::BlockQuote(_)) => {
                    let (inner, next) = span(events, i);
                    doc.body
                        .push_str(&note(&events[inner], pending_class.take()));
                    i = next;
                }

                // Wrapped so a wide table scrolls inside its own box rather than
                // forcing the page body sideways.
                Event::Start(Tag::Table(_)) => {
                    let (_, next) = span(events, i);
                    let rendered = with_class(
                        render_events(&events[i..next]),
                        "<table>",
                        pending_class.take(),
                    );
                    doc.body
                        .push_str(&format!("<div class=\"scroller\">\n{rendered}</div>\n"));
                    i = next;
                }

                Event::Start(Tag::List(kind)) => {
                    let open = if kind.is_some() { "<ol>" } else { "<ul>" };
                    let (_, next) = span(events, i);
                    doc.body.push_str(&with_class(
                        render_events(&events[i..next]),
                        open,
                        pending_class.take(),
                    ));
                    i = next;
                }

                // Everything else streams through the standard renderer one event at a
                // time — `push_html` is a streaming renderer, so open and close tags
                // still pair up correctly across calls.
                event => {
                    if matches!(event, Event::Start(_)) {
                        want_thesis = false;
                    }
                    html::push_html(&mut doc.body, std::iter::once(event.clone()));
                    i += 1;
                }
            }
        }

        doc
    }
}

/// A blockquote becomes a labelled note. A leading bold run that reads as a label
/// (`> **Exit criteria:** ...`) is lifted out into the note's label slot, and picks the
/// note's variant when no explicit `<!-- class: -->` was given.
fn note(inner: &[Event], class: Option<String>) -> String {
    let label = label_of(inner);
    let class = class.unwrap_or_else(|| variant_for(label.as_ref().map(|(l, _)| l.as_str())).to_string());

    // Keep the paragraph open, drop the lifted bold run, then continue from the rest.
    let body = match &label {
        Some((_, rest)) => {
            let kept: Vec<Event> = std::iter::once(inner[0].clone())
                .chain(inner[*rest..].iter().cloned().map(trim_leading_space))
                .collect();
            render_events(&kept)
        }
        None => render_events(inner),
    };

    let label_html = label
        .map(|(l, _)| format!("<span class=\"label\">{}</span>\n", escape(&l)))
        .unwrap_or_default();

    format!("<div class=\"{class}\">\n{label_html}{body}</div>\n")
}

/// `> **Some label:** rest` — a bold run opening the quote that ends in a colon, or is
/// short enough to read as a heading rather than as emphasis. Returns the label text and
/// the index the remaining body starts at. The run may contain inline markup (a label
/// like `**Note on `app.rs`.**` spans several events), so it is scanned depth-aware
/// rather than assumed to be a single `Text`.
fn label_of(inner: &[Event]) -> Option<(String, usize)> {
    matches!(inner.first()?, Event::Start(Tag::Paragraph)).then_some(())?;
    matches!(inner.get(1)?, Event::Start(Tag::Strong)).then_some(())?;

    let (span, rest) = span(inner, 1);
    let text = plain_text(&inner[span]);
    let trimmed = text.trim().trim_end_matches([':', '.', '—', '-']).trim();

    (text.trim_end().ends_with(':') || trimmed.split_whitespace().count() <= 8)
        .then(|| (trimmed.to_string(), rest))
}

fn variant_for(label: Option<&str>) -> &'static str {
    let Some(label) = label.map(str::to_lowercase) else {
        return "note";
    };

    const KEY: [&str; 5] = ["exit criteria", "payoff", "decision", "locked", "the point"];
    const ALT: [&str; 6] = [
        "alternative",
        "risk",
        "caveat",
        "warning",
        "trade-off",
        "gotcha",
    ];

    match () {
        _ if KEY.iter().any(|k| label.contains(k)) => "note key",
        _ if ALT.iter().any(|k| label.contains(k)) => "note alt",
        _ => "note",
    }
}

// ── Event-stream helpers ────────────────────────────────────────────────────

/// The range of events *inside* the container starting at `start`, plus the index just
/// past its close. Depth-counted, so nested containers of any kind are handled.
fn span(events: &[Event], start: usize) -> (Range<usize>, usize) {
    let mut depth = 0usize;

    for (offset, event) in events[start..].iter().enumerate() {
        match event {
            Event::Start(_) => depth += 1,
            Event::End(_) => {
                depth -= 1;
                if depth == 0 {
                    let close = start + offset;
                    return (start + 1..close, close + 1);
                }
            }
            _ => {}
        }
    }

    (start + 1..events.len(), events.len())
}

fn render_events(events: &[Event]) -> String {
    let mut out = String::new();
    html::push_html(&mut out, events.iter().cloned());
    out
}

fn plain_text(events: &[Event]) -> String {
    events
        .iter()
        .filter_map(|event| match event {
            Event::Text(t) | Event::Code(t) => Some(t.as_ref()),
            _ => None,
        })
        .collect()
}

/// Add a class to the first occurrence of an opening tag in already-rendered HTML.
/// `push_html` gives no hook for attributes, and the input here is our own output.
fn with_class(html: String, open_tag: &str, class: Option<String>) -> String {
    match class {
        Some(class) => {
            let replacement = format!("{} class=\"{class}\">", open_tag.trim_end_matches('>'));
            html.replacen(open_tag, &replacement, 1)
        }
        None => html,
    }
}

fn trim_leading_space(event: Event) -> Event {
    match event {
        Event::Text(text) if text.starts_with(char::is_whitespace) => {
            Event::Text(text.trim_start().to_string().into())
        }
        other => other,
    }
}

/// `<!-- key: value -->` → `("key", "value")`. Anything else is left alone.
fn directive(raw: &str) -> Option<(&'static str, String)> {
    const KEYS: [&str; 4] = ["class", "eyebrow", "title", "fact"];

    let inner = raw.trim().strip_prefix("<!--")?.strip_suffix("-->")?.trim();
    let (key, value) = inner.split_once(':')?;
    let key = key.trim();

    KEYS.iter()
        .find(|k| **k == key)
        .map(|k| (*k, value.trim().to_string()))
}

fn unique_slug(text: &str, taken: &mut HashSet<String>) -> String {
    let base: String = text
        .to_lowercase()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect::<String>()
        .split('-')
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join("-");

    let base = if base.is_empty() {
        "section".to_string()
    } else {
        base
    };

    (0..)
        .map(|n| match n {
            0 => base.clone(),
            n => format!("{base}-{n}"),
        })
        .find(|candidate| taken.insert(candidate.clone()))
        .unwrap_or(base)
}

fn escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// Plain-text form of already-rendered inline HTML, for the `<title>` element.
fn strip_tags(html: &str) -> String {
    let mut out = String::with_capacity(html.len());
    let mut inside = false;

    for c in html.chars() {
        match c {
            '<' => inside = true,
            '>' => inside = false,
            c if !inside => out.push(c),
            _ => {}
        }
    }

    out.trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn body_of(markdown: &str) -> String {
        let events: Vec<Event> = Parser::new_ext(markdown, Options::ENABLE_TABLES).collect();
        Document::build(&events).body
    }

    #[test]
    fn placeholders_in_the_document_are_not_expanded() {
        // The plans contain GitHub Actions expressions; the body must not be able to
        // rewrite the shell, and unknown placeholders must survive verbatim.
        let filled = fill("<a>{{BODY}}</a>", &[("BODY", "${{ vars.STEAM_APP_ID }}{{FOOTER}}")]);
        assert_eq!(filled, "<a>${{ vars.STEAM_APP_ID }}{{FOOTER}}</a>");
    }

    #[test]
    fn fill_substitutes_every_known_key_once() {
        assert_eq!(
            fill("{{A}}/{{B}}/{{A}}", &[("A", "1"), ("B", "2")]),
            "1/2/1"
        );
    }

    #[test]
    fn h2_headings_get_ids_and_dividers() {
        let body = body_of("## First\n\ntext\n\n## Second\n");
        assert!(body.contains("<h2 id=\"first\">First</h2>"));
        assert!(body.contains("<h2 id=\"second\">Second</h2>"));
        // The divider separates sections, so it precedes every H2 but the first.
        assert_eq!(body.matches("tick-rule").count(), 1);
    }

    #[test]
    fn duplicate_headings_get_distinct_slugs() {
        let body = body_of("## Notes\n\n## Notes\n");
        assert!(body.contains("id=\"notes\""));
        assert!(body.contains("id=\"notes-1\""));
    }

    #[test]
    fn blockquote_label_becomes_a_note_label() {
        let body = body_of("> **Exit criteria:** the triangle spins.\n");
        assert!(body.contains("class=\"note key\""));
        assert!(body.contains("<span class=\"label\">Exit criteria</span>"));
        assert!(body.contains("the triangle spins."));
        // The lifted label must not also remain in the body.
        assert!(!body.contains("<strong>"));
    }

    #[test]
    fn blockquote_without_a_label_stays_a_plain_note() {
        let body = body_of("> just a remark\n");
        assert!(body.contains("class=\"note\""));
        assert!(!body.contains("class=\"label\""));
    }

    #[test]
    fn a_label_containing_inline_code_is_still_lifted() {
        let body = body_of("> **Note on `app.rs`.** the rest of it\n");
        assert!(body.contains("<span class=\"label\">Note on app.rs</span>"));
        assert!(body.contains("the rest of it"));
        assert!(!body.contains("<strong>"));
    }

    #[test]
    fn a_long_bold_opener_is_emphasis_not_a_label() {
        let body =
            body_of("> **This is a long bold sentence that is plainly emphasis, not a label** ok\n");
        assert!(!body.contains("class=\"label\""));
        assert!(body.contains("<strong>"));
    }

    #[test]
    fn alternatives_and_risks_pick_the_alt_variant() {
        assert!(body_of("> **Alternative considered:** x\n").contains("class=\"note alt\""));
        assert!(body_of("> **Risk:** y\n").contains("class=\"note alt\""));
    }

    #[test]
    fn tables_are_wrapped_so_they_scroll_independently() {
        let body = body_of("| a | b |\n| - | - |\n| 1 | 2 |\n");
        assert!(body.contains("<div class=\"scroller\">"));
        assert!(body.contains("<table>"));
    }

    #[test]
    fn class_directive_applies_to_the_next_block_only() {
        let body = body_of("<!-- class: corrective -->\n\n| a |\n| - |\n| 1 |\n\n| c |\n| - |\n| 2 |\n");
        assert!(body.contains("<table class=\"corrective\">"));
        assert_eq!(body.matches("corrective").count(), 1);
        // The directive itself must not survive into the output.
        assert!(!body.contains("<!--"));
    }

    #[test]
    fn class_directive_applies_to_lists() {
        let body = body_of("<!-- class: checks -->\n\n1. one\n2. two\n");
        assert!(body.contains("<ol class=\"checks\">"));
    }

    #[test]
    fn document_directives_are_lifted_out_of_the_body() {
        let events: Vec<Event> = Parser::new_ext(
            "<!-- eyebrow: A / B -->\n<!-- fact: Backends = winit -->\n\n# Title\n\nStandfirst.\n\nBody.\n",
            Options::empty(),
        )
        .collect();
        let doc = Document::build(&events);

        assert_eq!(doc.eyebrow.as_deref(), Some(&["A".to_string(), "B".to_string()][..]));
        assert_eq!(doc.facts, vec![("Backends".to_string(), "winit".to_string())]);
        assert_eq!(doc.heading.as_deref(), Some("Title"));
        assert_eq!(doc.thesis.as_deref(), Some("Standfirst."));
        assert!(doc.body.contains("Body."));
        assert!(!doc.body.contains("Standfirst."));
    }

    #[test]
    fn unknown_comments_pass_through_untouched() {
        assert!(body_of("<!-- TODO: revisit -->\n").contains("<!-- TODO: revisit -->"));
    }

    #[test]
    fn nested_containers_do_not_confuse_the_scanner() {
        let body = body_of("> **Note:** intro\n>\n> - one\n> - two\n");
        assert!(body.contains("<li>one</li>"));
        assert!(body.contains("<li>two</li>"));
        // One note div, properly closed around the nested list.
        assert_eq!(body.matches("<div class=").count(), 1);
        assert_eq!(body.matches("</div>").count(), 1);
    }
}
