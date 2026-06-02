use super::SearchBackend;
use crate::types::Paper;
use async_trait::async_trait;
use reqwest::Client;
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Mutex;
use std::sync::OnceLock;
use std::time::{Duration, Instant};

fn cache() -> &'static Mutex<HashMap<String, (Vec<Paper>, Instant)>> {
    static CACHE: OnceLock<Mutex<HashMap<String, (Vec<Paper>, Instant)>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

const CACHE_TTL: Duration = Duration::from_secs(600); // 10 minutes

pub struct OpenAlexBackend {
    client: Client,
}

impl OpenAlexBackend {
    pub fn new() -> Self {
        Self {
            client: Client::builder()
                .user_agent("PaperSearch/1.0 (mailto:research@example.com)")
                .build()
                .unwrap(),
        }
    }

    fn parse_work(&self, w: &Value) -> Option<Paper> {
        let id = w["id"].as_str()?.to_string();
        let doi = w["doi"].as_str().unwrap_or("N/A").to_string();
        let title = w["title"].as_str()?.to_string();
        let year = w["publication_year"].as_u64().unwrap_or(0) as u32;
        let citation_count = w["cited_by_count"].as_u64().unwrap_or(0) as u32;

        let authors: Vec<String> = w["authorships"]
            .as_array()?
            .iter()
            .filter_map(|a| {
                a["author"]["display_name"]
                    .as_str()
                    .map(|s| s.to_string())
            })
            .collect();

        let venue = w["primary_location"]["source"]["display_name"]
            .as_str()
            .unwrap_or("Unknown")
            .to_string();

        let abstract_text = Self::rebuild_abstract(&w["abstract_inverted_index"]);
        let url = format!("https://doi.org/{}", doi);

        Some(Paper {
            id, title, authors, year, venue, doi, abstract_text, citation_count, url,
        })
    }

    fn rebuild_abstract(inverted: &Value) -> String {
        let obj = match inverted.as_object() {
            Some(o) => o,
            None => return "No abstract available.".into(),
        };
        let mut pairs: Vec<(usize, &str)> = Vec::new();
        for (word, positions) in obj {
            if let Some(pos_list) = positions.as_array() {
                for pos in pos_list {
                    if let Some(idx) = pos.as_u64() {
                        pairs.push((idx as usize, word.as_str()));
                    }
                }
            }
        }
        pairs.sort_by_key(|(idx, _)| *idx);
        let text: String = pairs.into_iter().map(|(_, w)| w).collect::<Vec<_>>().join(" ");
        if text.len() > 500 { format!("{}...", &text[..500]) } else { text }
    }

    /// Fetch with caching — checks global cache before making HTTP request
    async fn cached_fetch(&self, url: &str) -> Result<Vec<Paper>, anyhow::Error> {
        // Check cache
        {
            let c = cache().lock().unwrap();
            if let Some((papers, ts)) = c.get(url) {
                if ts.elapsed() < CACHE_TTL {
                    return Ok(papers.clone());
                }
            }
        }

        // Do HTTP request
        let resp = self.client.get(url).send().await?;
        let json: Value = resp.json().await?;
        let papers: Vec<Paper> = json["results"]
            .as_array()
            .map(|arr| arr.iter().filter_map(|w| self.parse_work(w)).collect())
            .unwrap_or_default();

        // Store in cache
        {
            let mut c = cache().lock().unwrap();
            c.insert(url.to_string(), (papers.clone(), Instant::now()));
        }

        Ok(papers)
    }
}

#[async_trait]
impl SearchBackend for OpenAlexBackend {
    fn name(&self) -> &str { "openalex" }

    async fn search(&self, query: &str, max_results: usize) -> Result<Vec<Paper>, anyhow::Error> {
        let url = format!(
            "https://api.openalex.org/works?search={}&per_page={}&sort=relevance_score:desc",
            urlencoding::encode(query), max_results
        );
        self.cached_fetch(&url).await
    }

    async fn get_cited_papers(&self, paper_id: &str, max_results: usize) -> Result<Vec<Paper>, anyhow::Error> {
        let url = format!(
            "https://api.openalex.org/works?filter=cites:{}&per_page={}&sort=cited_by_count:desc",
            paper_id, max_results
        );
        self.cached_fetch(&url).await
    }

    async fn get_references(&self, paper_id: &str, max_results: usize) -> Result<Vec<Paper>, anyhow::Error> {
        let url = format!(
            "https://api.openalex.org/works?filter=cited_by:{}&per_page={}&sort=cited_by_count:desc",
            paper_id, max_results
        );
        self.cached_fetch(&url).await
    }
}
