//! The user guide as a page of its own: `docs/user-guide.md` rendered to one HTML
//! file, its style inside it and nothing to fetch, which the packages carry for
//! Help ▸ Kerf User Guide to open without a connection
//! (`scripts/package-kerf-windows.ps1`, `package-kerf-macos.sh`;
//! `apps/kerf-qt/user_guide.cpp` finds it).

use std::collections::HashMap;

use pulldown_cmark::{html, CowStr, Event, HeadingLevel, Options, Parser, Tag, TagEnd};

/// The repository the guide lives in.
pub const REPOSITORY: &str = "https://github.com/imjunseong958-droid/industrial-kerf";

/// What to render, and what to say about it.
pub struct Page<'a> {
    /// The guide, as Markdown.
    pub markdown: &'a str,
    /// The Kerf it comes with, named in its title and at its foot.
    pub version: &'a str,
    /// What a link relative to the guide is relative to: the guide's folder on
    /// GitHub, at the commit the package is made from, ending in a slash. Only the
    /// guide is in the package, so a link to another file of the repository goes
    /// there.
    pub link_base: &'a str,
}

impl Page<'_> {
    /// The guide's folder on GitHub, on `main`.
    pub fn main_link_base() -> String {
        format!("{REPOSITORY}/blob/main/docs/")
    }
}

/// The page, as one HTML document.
pub fn render(page: &Page<'_>) -> Result<String, String> {
    let options = Options::ENABLE_TABLES | Options::ENABLE_STRIKETHROUGH;
    let mut events: Vec<Event<'_>> = Parser::new_ext(page.markdown, options).collect();
    let mut title = None;
    let mut anchors = HashMap::new();
    for index in 0..events.len() {
        let heading = match &events[index] {
            Event::Start(Tag::Heading { .. }) => Some(heading_text(&events[index + 1..])),
            _ => None,
        };
        match &mut events[index] {
            Event::Start(Tag::Heading { level, id, .. }) => {
                let text = heading.unwrap_or_default();
                if *level == HeadingLevel::H1 && title.is_none() {
                    title = Some(text.clone());
                }
                // As GitHub names them, so a link to a section of the guide on
                // GitHub finds the same section here.
                if id.is_none() {
                    *id = Some(CowStr::from(anchor(&text, &mut anchors)));
                }
            }
            Event::Start(Tag::Link { dest_url, .. }) => {
                if let Some(resolved) = resolve(dest_url, page.link_base) {
                    *dest_url = CowStr::from(resolved);
                }
            }
            // The page fetches nothing, and the package carries no picture beside it.
            Event::Start(Tag::Image { dest_url, .. }) => {
                return Err(format!(
                    "The guide shows a picture ({dest_url}), and the page has no way to carry one yet: \
                     give kerf-guide one (as a data: URL) before adding it"
                ));
            }
            _ => {}
        }
    }
    let title = title.ok_or("The guide has no heading of the first level to title the page")?;

    let mut body = String::new();
    html::push_html(&mut body, events.into_iter());
    let version = escape(page.version);
    Ok(format!(
        "<!doctype html>\n\
         <html lang=\"en\">\n\
         <head>\n\
         <meta charset=\"utf-8\">\n\
         <meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\n\
         <meta name=\"generator\" content=\"kerf-guide\">\n\
         <title>{title} · Industrial Kerf {version}</title>\n\
         <style>{STYLE}</style>\n\
         </head>\n\
         <body>\n\
         <main>\n{body}</main>\n\
         <footer>\n<p>This guide came with Industrial Kerf {version}. \
         The newest is on <a href=\"{REPOSITORY}/blob/main/docs/user-guide.md\">GitHub</a>.</p>\n\
         </footer>\n\
         </body>\n\
         </html>\n",
        title = escape(&title),
    ))
}

/// The words of a heading, from the events after its start.
fn heading_text(events: &[Event<'_>]) -> String {
    events
        .iter()
        .take_while(|event| !matches!(event, Event::End(TagEnd::Heading(_))))
        .filter_map(|event| match event {
            Event::Text(text) | Event::Code(text) => Some(text.as_ref()),
            _ => None,
        })
        .collect()
}

/// A heading's anchor as GitHub makes it: lower case, its spaces hyphens, its
/// punctuation but `-` and `_` dropped, and `-1`, `-2` after a repeat.
fn anchor(text: &str, used: &mut HashMap<String, usize>) -> String {
    let base: String = text
        .trim()
        .chars()
        .flat_map(char::to_lowercase)
        .filter_map(|c| match c {
            ' ' => Some('-'),
            c if c.is_alphanumeric() || c == '-' || c == '_' => Some(c),
            _ => None,
        })
        .collect();
    let seen = used.entry(base.clone()).or_insert(0);
    let anchor = if *seen == 0 {
        base
    } else {
        format!("{base}-{seen}")
    };
    *seen += 1;
    anchor
}

/// Where a link relative to the guide goes; `None` for one that already says
/// (a scheme, or a section of the guide).
fn resolve(link: &str, base: &str) -> Option<String> {
    let has_scheme = link.split_once(':').is_some_and(|(scheme, _)| {
        !scheme.is_empty()
            && scheme
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || "+-.".contains(c))
    });
    if link.is_empty() || link.starts_with('#') || link.starts_with('/') || has_scheme {
        return None;
    }
    // base is scheme://host/path/; collapse the link's . and .. into the path.
    let (origin, path) = match base.split_once("://") {
        Some((scheme, rest)) => {
            let (host, path) = rest.split_once('/').unwrap_or((rest, ""));
            (format!("{scheme}://{host}"), path)
        }
        None => (String::new(), base),
    };
    let mut segments: Vec<&str> = path
        .split('/')
        .filter(|segment| !segment.is_empty())
        .collect();
    let (link_path, fragment) = match link.find(['#', '?']) {
        Some(at) => link.split_at(at),
        None => (link, ""),
    };
    for segment in link_path.split('/') {
        match segment {
            "" | "." => {}
            ".." => {
                segments.pop();
            }
            segment => segments.push(segment),
        }
    }
    let trailing = if link_path.ends_with('/') { "/" } else { "" };
    Some(format!(
        "{origin}/{}{trailing}{fragment}",
        segments.join("/")
    ))
}

fn escape(text: &str) -> String {
    let mut escaped = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => escaped.push_str("&amp;"),
            '<' => escaped.push_str("&lt;"),
            '>' => escaped.push_str("&gt;"),
            '"' => escaped.push_str("&quot;"),
            '\'' => escaped.push_str("&#39;"),
            c => escaped.push(c),
        }
    }
    escaped
}

/// The editor's own colours and type (`apps/kerf-qt/design_tokens.h`), with the
/// accent lightened to read as text on the dark ground. No web font: the page is
/// read offline, so it falls back to the system's.
const STYLE: &str = "
:root { color-scheme: dark; }
html { background: #121212; }
body {
  margin: 0; padding: 48px 24px 64px;
  color: #ededed; background: #121212;
  font: 16px/1.6 Inter, system-ui, -apple-system, \"Segoe UI\", Roboto, sans-serif;
}
main, footer { max-width: 44rem; margin: 0 auto; }
h1, h2, h3, h4 { font-weight: 500; line-height: 1.25; color: #ededed; }
h1 { font-size: 2rem; margin: 0 0 1.5rem; }
h2 { font-size: 1.4rem; margin: 2.5rem 0 0.75rem; padding-top: 1rem; border-top: 1px solid #262626; }
h3 { font-size: 1.1rem; margin: 1.75rem 0 0.5rem; }
p, ul, ol, table { margin: 0 0 1rem; }
li { margin: 0.25rem 0; }
strong { font-weight: 600; color: #ffffff; }
a { color: #7c9cf0; text-decoration: none; }
a:hover, a:focus-visible { text-decoration: underline; }
code, kbd, pre { font-family: \"JetBrains Mono\", ui-monospace, Menlo, Consolas, monospace; font-size: 0.9em; }
code { background: #1c1c1c; border-radius: 4px; padding: 0.1em 0.35em; }
pre { background: #1c1c1c; border-radius: 6px; padding: 12px 16px; overflow-x: auto; }
pre code { background: none; padding: 0; }
table { border-collapse: collapse; }
th, td { border: 1px solid #262626; padding: 6px 10px; text-align: left; }
footer { margin-top: 3rem; padding-top: 1rem; border-top: 1px solid #262626; color: #a1a1a1; font-size: 0.875rem; }
";

#[cfg(test)]
mod tests {
    use super::*;

    const BASE: &str = "https://github.com/imjunseong958-droid/industrial-kerf/blob/0123abc/docs/";

    fn page(markdown: &str) -> String {
        render(&Page {
            markdown,
            version: "9.8.7",
            link_base: BASE,
        })
        .expect("renders")
    }

    #[test]
    fn links_to_the_rest_of_the_repository_go_to_github_at_the_commit() {
        let html = page(
            "# Guide\n\n[cloud](cloud.md) [build](../apps/kerf-qt/README.md#build) [here](#start) \
             [there](./looks.md?plain=1) [web](https://example.org/a) [mail](mailto:a@b.c)\n",
        );
        for href in [
            "https://github.com/imjunseong958-droid/industrial-kerf/blob/0123abc/docs/cloud.md",
            "https://github.com/imjunseong958-droid/industrial-kerf/blob/0123abc/apps/kerf-qt/README.md#build",
            "#start",
            "https://github.com/imjunseong958-droid/industrial-kerf/blob/0123abc/docs/looks.md?plain=1",
            "https://example.org/a",
            "mailto:a@b.c",
        ] {
            assert!(html.contains(&format!("href=\"{href}\"")), "no link to {href}:\n{html}");
        }
    }

    #[test]
    fn sections_are_named_as_github_names_them() {
        let html = page("# Guide\n\n## Start a project\n\n## What's new?\n\n## Start a project\n\n## `kerf://` links\n");
        for id in [
            "guide",
            "start-a-project",
            "whats-new",
            "start-a-project-1",
            "kerf-links",
        ] {
            assert!(html.contains(&format!(" id=\"{id}\"")), "no #{id}:\n{html}");
        }
    }

    #[test]
    fn the_title_and_version_are_text() {
        let html = render(&Page {
            markdown: "# A < b & \"c\"\n",
            version: "<1>",
            link_base: BASE,
        })
        .unwrap();
        assert!(
            html.contains(
                "<title>A &lt; b &amp; &quot;c&quot; · Industrial Kerf &lt;1&gt;</title>"
            ),
            "{html}"
        );
    }

    #[test]
    fn a_picture_or_no_title_is_refused() {
        let picture = render(&Page {
            markdown: "# Guide\n\n![shot](shot.png)\n",
            version: "1",
            link_base: BASE,
        });
        assert!(picture.unwrap_err().contains("shot.png"));
        assert!(render(&Page {
            markdown: "## Only a section\n",
            version: "1",
            link_base: BASE
        })
        .is_err());
    }
}
