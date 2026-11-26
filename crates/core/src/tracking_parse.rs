//! Answer reading: brand detection, the judge prompt, the judge verdict, and the rule that
//! chooses between the small and the big judge model.
//!
//! Pure logic. Calls to a model live in the worker.

use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

/// Answers the judge checks with both models before it picks `small_first` or `big_only`.
pub const AGREEMENT_SAMPLE: i32 = 200;
/// Share of the sample, in percent, in which the small model must agree with the big one.
pub const AGREEMENT_PERCENT: i32 = 90;

/// A brand to find in an answer. The owned brand and each competitor use the same shape.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct BrandSpec {
    pub name: String,
    pub domain: Option<String>,
    pub aliases: Vec<String>,
}

/// The host of a URL in lower case, without `www.`. `None` when no host can be read.
pub fn domain_of(url: &str) -> Option<String> {
    let rest = url.trim().split_once("://").map_or(url.trim(), |(_, r)| r);
    let authority = rest.split(['/', '?', '#']).next()?;
    let host = authority
        .rsplit('@')
        .next()?
        .split(':')
        .next()?
        .to_ascii_lowercase();
    let host = host.strip_prefix("www.").unwrap_or(&host);
    (host.contains('.') && !host.contains(' ')).then(|| host.to_owned())
}

/// True when `host` is `domain` or a subdomain of it.
pub fn same_site(host: &str, domain: &str) -> bool {
    let domain = domain.trim().to_ascii_lowercase();
    let domain = domain.strip_prefix("www.").unwrap_or(&domain);
    host == domain || host.ends_with(&format!(".{domain}"))
}

/// Lower case, every run of non-alphanumeric characters as one space, with a space at both ends,
/// so a name matches only as whole words.
pub fn normalize(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 2);
    out.push(' ');
    for c in text.chars().flat_map(char::to_lowercase) {
        if c.is_alphanumeric() {
            out.push(c);
        } else if !out.ends_with(' ') {
            out.push(' ');
        }
    }
    if !out.ends_with(' ') {
        out.push(' ');
    }
    out
}

impl BrandSpec {
    fn needles(&self) -> Vec<String> {
        let mut all = vec![normalize(&self.name)];
        all.extend(self.aliases.iter().map(|a| normalize(a)));
        if let Some(d) = &self.domain {
            all.push(normalize(d));
        }
        all.retain(|n| n.trim().len() > 1);
        all
    }

    /// Does `text` name this brand: by name, alias or domain, as whole words?
    pub fn is_named_in(&self, text: &str) -> bool {
        let hay = normalize(text);
        self.needles().iter().any(|n| hay.contains(n.as_str()))
    }

    /// Is `name` this brand? Compares the normalized name with the brand name, an alias or the domain.
    pub fn is_called(&self, name: &str) -> bool {
        let wanted = normalize(name);
        self.needles().contains(&wanted)
    }
}

/// Did the answer cite a page on the brand's own domain?
pub fn cites_domain(urls: &[&str], own: &BrandSpec) -> bool {
    let Some(domain) = own.domain.as_deref() else {
        return false;
    };
    urls.iter()
        .filter_map(|u| domain_of(u))
        .any(|h| same_site(&h, domain))
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BrandVerdict {
    pub name: String,
    #[serde(default)]
    pub recommended: bool,
    /// Place in a ranked list in the answer, from 1. `None` when the answer has no ranked list.
    #[serde(default)]
    pub rank: Option<i32>,
    /// `positive`, `neutral` or `negative`.
    #[serde(default)]
    pub sentiment: Option<String>,
}

/// What the judge says about one answer.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Verdict {
    pub brands: Vec<BrandVerdict>,
}

const SENTIMENTS: [&str; 3] = ["positive", "neutral", "negative"];

/// Read the judge reply. It may hold the JSON object inside text or a code fence. `None`: invalid.
pub fn parse_verdict(reply: &str) -> Option<Verdict> {
    let start = reply.find('{')?;
    let end = reply.rfind('}')?;
    let mut verdict: Verdict = serde_json::from_str(reply.get(start..=end)?).ok()?;
    let mut seen = BTreeSet::new();
    verdict
        .brands
        .retain(|b| !b.name.trim().is_empty() && seen.insert(normalize(&b.name)));
    for b in &mut verdict.brands {
        b.name = b.name.trim().to_owned();
        b.rank = b.rank.filter(|r| *r >= 1);
        b.sentiment = b
            .sentiment
            .take()
            .map(|s| s.to_lowercase())
            .filter(|s| SENTIMENTS.contains(&s.as_str()));
    }
    Some(verdict)
}

/// The two parts of the judge call: the system text and the user text.
pub fn judge_prompt(
    question: &str,
    answer: &str,
    own: &BrandSpec,
    competitors: &[BrandSpec],
) -> (String, String) {
    let system = "You read an answer from an AI assistant and report which brands it names. \
Reply with one JSON object only, in this form: \
{\"brands\":[{\"name\":\"string\",\"recommended\":true,\"rank\":1,\"sentiment\":\"positive\"}]}. \
List every brand, product or company the answer names, once each. \
\"recommended\": the answer suggests the reader use or choose it. \
\"rank\": its place in a ranked or numbered list in the answer, from 1, or null when there is no such list. \
\"sentiment\": positive, neutral or negative, as the answer treats it."
        .to_owned();
    let names = |b: &BrandSpec| match &b.domain {
        Some(d) => format!("{} ({d})", b.name),
        None => b.name.clone(),
    };
    let known: Vec<String> = std::iter::once(names(own))
        .chain(competitors.iter().map(names))
        .collect();
    let user = format!(
        "Brands to look for first: {}.\n\nQuestion asked:\n{question}\n\nAnswer to read:\n{answer}",
        known.join("; ")
    );
    (system, user)
}

impl Verdict {
    /// The entry for `own`, when the judge listed it.
    pub fn own<'a>(&'a self, own: &BrandSpec) -> Option<&'a BrandVerdict> {
        self.brands.iter().find(|b| own.is_called(&b.name))
    }

    /// Two verdicts agree when they name the same brands and give the owned brand the same
    /// recommendation, rank and sentiment.
    pub fn agrees_with(&self, other: &Verdict, own: &BrandSpec) -> bool {
        // The owned brand counts under its own name, whichever alias the judge used.
        let key = |name: &str| normalize(if own.is_called(name) { &own.name } else { name });
        let names = |v: &Verdict| {
            v.brands
                .iter()
                .map(|b| key(&b.name))
                .collect::<BTreeSet<_>>()
        };
        let facts = |v: &Verdict| {
            v.own(own)
                .map(|b| (b.recommended, b.rank, b.sentiment.clone()))
        };
        names(self) == names(other) && facts(self) == facts(other)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum JudgeMode {
    Unchecked,
    SmallFirst,
    BigOnly,
}

impl JudgeMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Unchecked => "unchecked",
            Self::SmallFirst => "small_first",
            Self::BigOnly => "big_only",
        }
    }

    pub fn parse(s: &str) -> Self {
        match s {
            "small_first" => Self::SmallFirst,
            "big_only" => Self::BigOnly,
            _ => Self::Unchecked,
        }
    }
}

/// The mode to set once the check has `samples` answers, of which `agreed` agreed. `None`: wait.
pub fn decide_mode(samples: i32, agreed: i32) -> Option<JudgeMode> {
    (samples >= AGREEMENT_SAMPLE).then(|| {
        if agreed * 100 >= samples * AGREEMENT_PERCENT {
            JudgeMode::SmallFirst
        } else {
            JudgeMode::BigOnly
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn acme() -> BrandSpec {
        BrandSpec {
            name: "Acme CRM".into(),
            domain: Some("acme.example".into()),
            aliases: vec!["Acme".into()],
        }
    }

    #[test]
    fn a_brand_matches_as_whole_words_by_name_alias_or_domain() {
        let b = acme();
        assert!(b.is_named_in("For small teams, **Acme CRM** is a fair pick."));
        assert!(b.is_named_in("Try acme, then compare."));
        assert!(b.is_named_in("See https://www.acme.example/pricing"));
        assert!(
            !b.is_named_in("The acmeville store is open."),
            "no match inside a longer word"
        );
        assert!(!b.is_named_in("Nothing here."));
    }

    #[test]
    fn citations_match_the_own_domain_and_its_subdomains_only() {
        let b = acme();
        assert!(cites_domain(
            &["https://docs.acme.example/x", "https://other.example"],
            &b
        ));
        assert!(!cites_domain(
            &[
                "https://notacme.example/x",
                "https://acme.example.evil.test"
            ],
            &b
        ));
        assert_eq!(
            domain_of("HTTPS://User@WWW.Acme.Example:8080/a?b#c").as_deref(),
            Some("acme.example")
        );
        assert_eq!(domain_of("not a url"), None);
    }

    #[test]
    fn the_verdict_is_read_from_text_around_the_json_and_cleaned() {
        let reply = "Here you go:\n```json\n{\"brands\":[{\"name\":\"Acme\",\"recommended\":true,\"rank\":1,\"sentiment\":\"Positive\"},\
            {\"name\":\" acme \",\"rank\":9},{\"name\":\"Rival\",\"rank\":0,\"sentiment\":\"great\"},{\"name\":\"\"}]}\n```";
        let v = parse_verdict(reply).unwrap();
        assert_eq!(v.brands.len(), 2, "duplicate and empty names are dropped");
        assert_eq!(
            (v.brands[0].rank, v.brands[0].sentiment.as_deref()),
            (Some(1), Some("positive"))
        );
        assert_eq!(
            (v.brands[1].rank, v.brands[1].sentiment.clone()),
            (None, None),
            "bad rank and sentiment are dropped"
        );
        assert!(v.own(&acme()).is_some());
        assert_eq!(parse_verdict("no json"), None);
        assert_eq!(parse_verdict("{\"brands\": 3}"), None);
    }

    #[test]
    fn verdicts_agree_on_names_and_on_what_they_say_about_the_owned_brand() {
        let own = acme();
        let a = parse_verdict(r#"{"brands":[{"name":"Acme","recommended":true,"rank":1,"sentiment":"positive"},{"name":"Rival"}]}"#).unwrap();
        let same = parse_verdict(r#"{"brands":[{"name":"rival"},{"name":"Acme CRM","recommended":true,"rank":1,"sentiment":"positive"}]}"#).unwrap();
        let other_rank = parse_verdict(r#"{"brands":[{"name":"Acme","recommended":true,"rank":2,"sentiment":"positive"},{"name":"Rival"}]}"#).unwrap();
        let missing = parse_verdict(
            r#"{"brands":[{"name":"Acme","recommended":true,"rank":1,"sentiment":"positive"}]}"#,
        )
        .unwrap();
        assert!(a.agrees_with(&same, &own));
        assert!(!a.agrees_with(&other_rank, &own));
        assert!(!a.agrees_with(&missing, &own));
    }

    #[test]
    fn the_mode_is_chosen_only_after_the_sample_and_by_the_agreement_share() {
        assert_eq!(decide_mode(199, 199), None);
        assert_eq!(decide_mode(200, 180), Some(JudgeMode::SmallFirst));
        assert_eq!(decide_mode(200, 179), Some(JudgeMode::BigOnly));
        assert_eq!(JudgeMode::parse("big_only"), JudgeMode::BigOnly);
        assert_eq!(JudgeMode::parse("x"), JudgeMode::Unchecked);
    }
}
