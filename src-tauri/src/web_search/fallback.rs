use super::{public_url, Duration, HashSet, Read, Res, Value};
use serde_json::json;
use std::time::Instant;

fn link(raw: &str) -> Option<String> {
    let base = url::Url::parse("https://duckduckgo.com/").ok()?;
    let url = base.join(raw).ok()?;
    if matches!(
        url.host_str(),
        Some("duckduckgo.com" | "www.duckduckgo.com" | "html.duckduckgo.com")
    ) {
        let (_, target) = url.query_pairs().find(|(k, _)| k == "uddg")?;
        return public_url(&target).then(|| target.into_owned());
    }
    public_url(url.as_str()).then(|| url.to_string())
}

fn brave(body: &str) -> Vec<Value> {
    let json: Value = serde_json::from_str(body).unwrap_or(Value::Null);
    let mut seen = HashSet::new();
    json.pointer("/web/results").and_then(Value::as_array).into_iter().flatten().filter_map(|row| {
        let url=row["url"].as_str()?;
        if !public_url(url) || !seen.insert(url) {return None}
        let plain=|v:&str| dom_query::Document::from(v).root().text().trim().to_owned();
        let title=plain(row["title"].as_str()?);
        let excerpt=plain(row["description"].as_str()?);
        (!title.is_empty() && !excerpt.is_empty()).then(||json!({"title":title.chars().take(180).collect::<String>(),"url":url,"excerpt":excerpt.chars().take(2600).collect::<String>(),"searchProvider":"Brave"}))
    }).take(8).collect()
}

fn duck(body: &str) -> Vec<Value> {
    let doc = dom_query::Document::from(body);
    let mut seen = HashSet::new();
    doc.select(".result:not(.result--ad)").iter().filter_map(|row| {
        let anchor=row.select("a.result__a");
        let url=link(&anchor.attr("href")?)?;
        let title=anchor.text().trim().to_owned();
        let excerpt=row.select(".result__snippet").text().trim().to_owned();
        if title.is_empty() || excerpt.is_empty() || !seen.insert(url.clone()) {return None}
        Some(json!({"title":title.chars().take(180).collect::<String>(),"url":url,"excerpt":excerpt.chars().take(2600).collect::<String>(),"searchProvider":"DuckDuckGo"}))
    }).take(8).collect()
}

fn body(response: ureq::Response) -> Option<String> {
    let mut text = String::new();
    response
        .into_reader()
        .take(512001)
        .read_to_string(&mut text)
        .ok()?;
    (text.len() <= 512000).then_some(text)
}

pub(super) fn search(query: &str, limit: usize) -> Res<Vec<Value>> {
    // All providers receive the very same outbound query. Credentials never follow redirects.
    let agent = ureq::AgentBuilder::new()
        .redirects(0)
        .user_agent("Beside/1.0 (public web search)")
        .build();
    let deadline = Instant::now() + Duration::from_secs(12);
    let key = std::env::var("BRAVE_API_KEY").ok();
    providers(
        query,
        limit,
        key.as_deref(),
        |q, key, n| {
            agent
                .get("https://api.search.brave.com/res/v1/web/search")
                .query("q", q)
                .query("count", &n.to_string())
                .set("X-Subscription-Token", key)
                .timeout(Duration::from_secs(5))
                .call()
                .ok()
                .and_then(body)
        },
        |q| duck_request(&agent, q, deadline),
    )
}

fn duck_request(agent: &ureq::Agent, query: &str, deadline: Instant) -> Res<String> {
    let response = agent
        .get("https://html.duckduckgo.com/html/")
        .query("q", query)
        .timeout(
            deadline
                .saturating_duration_since(Instant::now())
                .max(Duration::from_millis(1)),
        )
        .call()
        .map_err(|_| "Public search is temporarily unavailable.")?;
    body(response).ok_or("The public search response was too large.".into())
}

fn providers(
    query: &str,
    limit: usize,
    key: Option<&str>,
    brave_request: impl FnOnce(&str, &str, usize) -> Option<String>,
    duck_request: impl FnOnce(&str) -> Res<String>,
) -> Res<Vec<Value>> {
    if let Some(key) = key.map(str::trim).filter(|v| !v.is_empty()) {
        let mut rows = brave_request(query, key, limit)
            .map(|body| brave(&body))
            .unwrap_or_default();
        if !rows.is_empty() {
            rows.truncate(limit);
            return Ok(rows);
        }
    }
    let mut rows = duck(&duck_request(query)?);
    rows.truncate(limit);
    if rows.is_empty() {
        return Err("No readable public results were available from the fallback search.".into());
    }
    Ok(rows)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn provider_outage_replays_only_the_public_query_at_each_step() {
        let q = "HTTP/3 status";
        let html = r#"<div class="result"><a class="result__a" href="https://example.org/protocol">Protocol</a><span class="result__snippet">The published standard.</span></div>"#;
        let rows = super::super::search_using(
            q,
            false,
            |tool, args| {
                assert_eq!(tool, "web_search_exa");
                assert_eq!(args["query"], q);
                Err("unavailable".into())
            },
            |query, n| {
                providers(
                    query,
                    n,
                    Some("fixture"),
                    |typed, key, _| {
                        assert_eq!(typed, q);
                        assert_eq!(key, "fixture");
                        None
                    },
                    |typed| {
                        assert_eq!(typed, q);
                        Ok(html.to_owned())
                    },
                )
            },
        )
        .unwrap();
        assert_eq!(rows[0]["searchProvider"], "DuckDuckGo");
        let rows = providers(
            q,
            5,
            None,
            |_, _, _| panic!("unexpected Brave request"),
            |_| Ok(html.into()),
        )
        .unwrap();
        assert_eq!(rows[0]["url"], "https://example.org/protocol");
    }
    #[test]
    #[ignore = "Runs a keyless public search for an RFC, with no notebook"]
    fn keyless_public_search_returns_publisher_passages() {
        let agent = ureq::AgentBuilder::new()
            .redirects(0)
            .user_agent("Beside/1.0 (public web search)")
            .build();
        let rows = providers(
            "RFC 9114 HTTP 3 IETF",
            5,
            None,
            |_, _, _| None,
            |q| duck_request(&agent, q, Instant::now() + Duration::from_secs(12)),
        )
        .unwrap();
        assert!(rows
            .iter()
            .any(|row| row["excerpt"].as_str().is_some_and(|s| !s.is_empty())));
    }
    #[test]
    fn fallbacks_share_link_and_excerpt_rules() {
        assert!(super::super::public_typed_lookup("C# 13 release date"));
        assert!(super::super::public_typed_lookup("HTTP/3 status"));
        assert!(!super::super::public_typed_lookup(
            "HTTP/3 /Users/person/private"
        ));
        assert_eq!(
            link("//duckduckgo.com/l/?uddg=https%3A%2F%2Fexample.org%2Fguide").as_deref(),
            Some("https://example.org/guide")
        );
        for raw in [
            "http://127.0.0.1/x",
            "https://10.0.0.1",
            "http://localhost",
            "https://duckduckgo.com/l/?uddg=http%3A%2F%2Fhost.local",
            "https://name:pw@example.com",
        ] {
            assert!(link(raw).is_none(), "{raw}");
        }
        let rows = brave(
            r#"{"web":{"results":[{"title":"A <b>guide</b>","description":"Some text","url":"https://example.org/"},{"title":"Private","description":"No","url":"http://localhost/"}]}}"#,
        );
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0]["title"], "A guide");
        assert!(duck("<html>Verification required</html>").is_empty());
    }
}
