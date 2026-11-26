//! The article a revision stores: checks, Markdown export and the model prompt for rewrites.

use chrono::NaiveDate;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

/// Fields every exported file carries in its frontmatter.
pub const REQUIRED_FRONTMATTER: &[&str] = &["title", "description", "slug", "keywords", "date"];

#[derive(Clone, Debug, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct Article {
    /// The article headline.
    pub title: String,
    /// One sentence of at most 160 characters.
    pub meta_description: String,
    /// Lowercase words joined by hyphens.
    pub slug: String,
    /// The terms the article targets.
    pub keywords: Vec<String>,
    /// The article body in Markdown with no YAML frontmatter, no top-level title heading, and
    /// sections as level-2 headings.
    pub body_markdown: String,
}

impl Article {
    /// The first problem found, worded for the caller.
    pub fn check(&self) -> Result<(), String> {
        if self.title.trim().is_empty() || self.title.chars().count() > 200 {
            return Err("article.title must be 1 to 200 characters".into());
        }
        let meta = self.meta_description.trim();
        if meta.is_empty() || meta.chars().count() > 160 {
            return Err("article.metaDescription must be 1 to 160 characters".into());
        }
        if !is_slug(&self.slug) {
            return Err(
                "article.slug must be lowercase words joined by hyphens, at most 100 characters"
                    .into(),
            );
        }
        if self.keywords.len() > 30 || self.keywords.iter().any(|k| k.trim().is_empty()) {
            return Err("article.keywords must be at most 30 non-empty terms".into());
        }
        let first = self.body_markdown.trim_start();
        if first.is_empty() {
            return Err("article.bodyMarkdown is empty".into());
        }
        if first.starts_with("---") {
            return Err("article.bodyMarkdown must not start with YAML frontmatter".into());
        }
        if first.starts_with("# ") {
            return Err(
                "article.bodyMarkdown must not start with a top-level title heading".into(),
            );
        }
        Ok(())
    }

    /// The file a site repository stores: YAML frontmatter, then the body.
    pub fn to_markdown(&self, date: NaiveDate) -> String {
        // A JSON string is a valid YAML double-quoted scalar, so it needs no second escaper.
        let q = |s: &str| serde_json::to_string(s).unwrap_or_default();
        let keywords: Vec<String> = self.keywords.iter().map(|k| q(k)).collect();
        format!(
            "---\ntitle: {}\ndescription: {}\nslug: {}\nkeywords: [{}]\ndate: {}\n---\n\n{}\n",
            q(&self.title),
            q(self.meta_description.trim()),
            q(&self.slug),
            keywords.join(", "),
            date,
            self.body_markdown.trim()
        )
    }
}

pub fn is_slug(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 100
        && s.split('-').all(|w| {
            !w.is_empty()
                && w.bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit())
        })
}

/// The system and user prompt that ask a model for a new version of an article.
pub fn rewrite_prompt(article: &Article, instruction: Option<&str>) -> (String, String) {
    let system = "You are an editor of web articles that AI assistants may cite. Reply with one JSON \
object only, no code fence, with the keys title, metaDescription, slug, keywords (array of strings) \
and bodyMarkdown. metaDescription is at most 160 characters. slug is lowercase words joined by \
hyphens. bodyMarkdown has no frontmatter and no level-1 heading; sections are level-2 headings. \
Keep every fact that is in the article. Add no fact you cannot support."
        .to_owned();
    let task = match instruction {
        Some(text) => format!("Change the article as follows: {text}"),
        None => "Write the article again from the start: keep its topic and slug, improve its \
structure and clarity."
            .to_owned(),
    };
    let user = format!(
        "{task}\n\nArticle:\n{}",
        serde_json::to_string(article).unwrap_or_default()
    );
    (system, user)
}

/// The article in a model reply. The reply may wrap the JSON in a code fence or add words around it.
pub fn parse_reply(text: &str) -> Option<Article> {
    let (start, end) = (text.find('{')?, text.rfind('}')?);
    let article: Article = serde_json::from_str(text.get(start..=end)?).ok()?;
    article.check().ok()?;
    Some(article)
}

/// The address an article is published at: `pattern` with `{slug}` filled in. A pattern that
/// starts with `/` is relative to the site URL.
pub fn public_url(pattern: &str, site_url: &str, slug: &str) -> String {
    let path = pattern.replace("{slug}", slug);
    if path.starts_with('/') {
        format!("{}{path}", site_url.trim_end_matches('/'))
    } else {
        path
    }
}

/// The repository path of an article: `<root>/<slug>.<ext>`.
pub fn file_path(content_root: &str, slug: &str, extension: &str) -> String {
    let root = content_root.trim_matches('/');
    if root.is_empty() {
        format!("{slug}.{extension}")
    } else {
        format!("{root}/{slug}.{extension}")
    }
}

/// A repository folder of plain path characters that stays inside the repository.
pub fn is_safe_folder(path: &str) -> bool {
    let t = path.trim_matches('/');
    path.chars()
        .all(|c| c.is_ascii_alphanumeric() || "._-/".contains(c))
        && (t.is_empty() || t.split('/').all(|p| !p.is_empty() && p != "." && p != ".."))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Article {
        Article {
            title: "How \"GEO\" works".into(),
            meta_description: "A short guide.".into(),
            slug: "how-geo-works".into(),
            keywords: vec!["geo".into(), "ai: search".into()],
            body_markdown: "## Intro\n\nText.".into(),
        }
    }

    #[test]
    fn markdown_has_quoted_frontmatter() {
        let md = sample().to_markdown(NaiveDate::from_ymd_opt(2026, 11, 26).unwrap());
        assert_eq!(
            md,
            "---\ntitle: \"How \\\"GEO\\\" works\"\ndescription: \"A short guide.\"\nslug: \"how-geo-works\"\nkeywords: [\"geo\", \"ai: search\"]\ndate: 2026-11-26\n---\n\n## Intro\n\nText.\n"
        );
    }

    #[test]
    fn check_rejects_what_the_contract_forbids() {
        let mut a = sample();
        assert!(a.check().is_ok());
        a.body_markdown = "---\ntitle: x\n---\nbody".into();
        assert!(a.check().unwrap_err().contains("frontmatter"));
        a.body_markdown = "# Title\nbody".into();
        assert!(a.check().unwrap_err().contains("heading"));
        a = sample();
        a.slug = "Bad_Slug".into();
        assert!(a.check().is_err());
        a = sample();
        a.meta_description = "x".repeat(161);
        assert!(a.check().is_err());
    }

    #[test]
    fn urls_and_paths() {
        assert_eq!(
            public_url("/blog/{slug}", "https://acme.com/", "a-b"),
            "https://acme.com/blog/a-b"
        );
        assert_eq!(
            public_url("https://x.dev/{slug}/", "https://acme.com", "a"),
            "https://x.dev/a/"
        );
        assert_eq!(file_path("/content/blog/", "a", "md"), "content/blog/a.md");
        assert_eq!(file_path("", "a", "md"), "a.md");
        assert!(is_safe_folder("content/blog") && is_safe_folder("/") && is_safe_folder("a/b/"));
        assert!(!is_safe_folder("../x") && !is_safe_folder("a/../b") && !is_safe_folder("a//b"));
    }

    #[test]
    fn reply_parses_through_a_fence_and_rejects_bad_articles() {
        let json = serde_json::to_string(&sample()).unwrap();
        assert!(parse_reply(&format!("```json\n{json}\n```")).is_some());
        assert!(parse_reply("no json here").is_none());
        assert!(parse_reply(&json.replace("how-geo-works", "Bad Slug")).is_none());
    }
}
