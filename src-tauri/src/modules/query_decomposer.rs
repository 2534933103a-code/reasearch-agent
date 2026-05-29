use crate::backends::llm::LlmBackend;
use crate::types::{QueryConstraints, QueryPlan, SubQuery};
use anyhow::Context;
use serde_json::Value;

pub struct QueryDecomposer;

impl QueryDecomposer {
    pub async fn decompose(
        llm: &LlmBackend,
        query: &str,
    ) -> Result<(QueryPlan, u32), anyhow::Error> {
        let system = r#"你是一个学术文献检索专家。你的任务是将用户的复杂研究查询分解为多个可独立检索的子查询。

输出必须是严格的JSON格式，包含以下字段：
- sub_queries: 子查询数组，每个包含 query(英文关键词)、dimension(methodology/application/theory/dataset/survey之一)、weight(0-1重要性)
- year_range: 建议的年份范围 [起始年, 终止年]，如果查询中没有明确年份，默认不限制
- venues: 建议的发表 venue 筛选，如无则空数组
- methodology_required: 必须涉及的方法名，如无则空数组

规则：
1. 生成3-5个子查询，覆盖不同维度
2. 所有 query 必须是英文、适合学术搜索引擎的关键词
3. 不要添加超出用户原始查询范围的约束
4. 年份默认不限制，除非用户明确要求"#;

        let resp = llm.chat(system, query).await?;
        let tokens = resp.tokens;
        let json: Value = serde_json::from_str(&resp.content)
            .context("Failed to parse LLM JSON response")?;

        let sub_queries: Vec<SubQuery> = json["sub_queries"]
            .as_array()
            .map(|arr| {
                arr.iter()
                    .map(|sq| SubQuery {
                        query: sq["query"].as_str().unwrap_or("").to_string(),
                        dimension: sq["dimension"].as_str().unwrap_or("methodology").to_string(),
                        weight: sq["weight"].as_f64().unwrap_or(0.8),
                    })
                    .collect()
            })
            .unwrap_or_default();

        let year_range = json["year_range"].as_array().and_then(|arr| {
            if arr.len() == 2 {
                Some((
                    arr[0].as_u64().unwrap_or(0) as u32,
                    arr[1].as_u64().unwrap_or(0) as u32,
                ))
            } else {
                None
            }
        });

        let venues: Vec<String> = json["venues"]
            .as_array()
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(|s| s.to_string()))
                    .collect()
            })
            .unwrap_or_default();

        let methodology_required: Vec<String> = json["methodology_required"]
            .as_array()
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(|s| s.to_string()))
                    .collect()
            })
            .unwrap_or_default();

        Ok((QueryPlan {
            original: query.to_string(),
            sub_queries,
            constraints: QueryConstraints { year_range, venues, methodology_required },
        }, tokens))
    }
}
