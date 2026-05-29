use crate::types::Paper;
use async_trait::async_trait;

#[async_trait]
pub trait SearchBackend: Send + Sync {
    fn name(&self) -> &str;

    async fn search(
        &self,
        query: &str,
        max_results: usize,
    ) -> Result<Vec<Paper>, anyhow::Error>;

    async fn get_cited_papers(
        &self,
        paper_id: &str,
        max_results: usize,
    ) -> Result<Vec<Paper>, anyhow::Error>;

    async fn get_references(
        &self,
        paper_id: &str,
        max_results: usize,
    ) -> Result<Vec<Paper>, anyhow::Error>;
}

pub mod openalex;
