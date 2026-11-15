//! Pipe a JSON object containing `tracks`, `queries` and optional `iterations`
//! into this example to measure the same deterministic DTOs as the JS baseline.
//! No App session, file probing or audio writes are involved.
use serde::Deserialize;
use std::io::Read;
use std::time::Instant;
use tagrex_commands::query::{QueryIndex, QueryModel, QueryRequest, SuggestRequest};
use tagrex_commands::TrackDto;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Input {
    tracks: Vec<TrackDto>,
    queries: Vec<QueryModel>,
    #[serde(default = "iterations")]
    iterations: usize,
    #[serde(default)]
    suggest: Option<SuggestionInput>,
}
#[derive(Deserialize)]
struct SuggestionInput {
    text: String,
    field: Option<String>,
    #[serde(default = "suggest_strategy")]
    strategy: String,
}
fn suggest_strategy() -> String {
    "values".into()
}
fn iterations() -> usize {
    20
}
fn checksum(indices: &[usize]) -> u64 {
    indices.iter().fold(0u64, |sum, &i| {
        sum.wrapping_add((i as u64).wrapping_mul(31).wrapping_add(7))
    })
}
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut text = String::new();
    std::io::stdin().read_to_string(&mut text)?;
    let input: Input = serde_json::from_str(&text)?;
    let started = Instant::now();
    let index = QueryIndex::from_tracks(&input.tracks);
    let index_ms = started.elapsed().as_secs_f64() * 1000.0;
    let mut output = Vec::new();
    for (i, query) in input.queries.iter().enumerate() {
        let request = QueryRequest {
            request_id: i as u64,
            snapshot_id: Some(index.id),
            query: query.clone(),
        };
        let first = index.query(&request, || false)?;
        let mut times = Vec::new();
        let mut last = first.clone();
        for _ in 0..input.iterations.clamp(1, 1000) {
            let started = Instant::now();
            last = index.query(&request, || false)?;
            times.push(started.elapsed().as_secs_f64() * 1000.0);
        }
        times.sort_by(f64::total_cmp);
        let median = times[times.len() / 2];
        let p95 = times[((times.len() - 1) as f64 * 0.95).ceil() as usize];
        output.push(serde_json::json!({"query":i,"coldMs":first.elapsed_ms,"medianMs":median,"p95Ms":p95,"matched":last.matched,"checksum":checksum(&last.indices)}));
    }
    let suggestion = if let (Some(suggest), Some(query)) = (input.suggest, input.queries.first()) {
        let request = SuggestRequest {
            request_id: 1,
            snapshot_id: Some(index.id),
            query: query.clone(),
            text: suggest.text,
            field: suggest.field,
            limit: 12,
            strategy: suggest.strategy,
        };
        let first = index.suggest(&request, || false)?;
        let cold_ms = first.elapsed_ms;
        let mut last = first;
        let mut times = Vec::new();
        for _ in 0..input.iterations.clamp(1, 1000) {
            let started = Instant::now();
            last = index.suggest(&request, || false)?;
            times.push(started.elapsed().as_secs_f64() * 1000.0);
        }
        times.sort_by(f64::total_cmp);
        let median = times[times.len() / 2];
        let p95 = times[((times.len() - 1) as f64 * 0.95).ceil() as usize];
        Some(
            serde_json::json!({"elapsedMs":cold_ms,"coldMs":cold_ms,"medianMs":median,"p95Ms":p95,"returned":last.suggestions.len(),"truncated":last.truncated,"suggestions":last.suggestions}),
        )
    } else {
        None
    };
    println!(
        "{}",
        serde_json::json!({"rows":input.tracks.len(),"indexMs":index_ms,"queries":output,"suggestion":suggestion,"profile":if cfg!(debug_assertions){"debug"}else{"release"}})
    );
    Ok(())
}
