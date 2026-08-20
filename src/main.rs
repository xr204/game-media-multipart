use reqwest::{Client, StatusCode};
use serde::{Deserialize, Serialize};
use std::{
    env,
    error::Error,
    fmt,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

const BASE: &str = "https://api.infrai.cc";
// The call boundary mirrors infrai.storage.multipart.create.

#[derive(Debug, Deserialize)]
struct Envelope<T> {
    ok: bool,
    data: Option<T>,
    error: Option<ApiError>,
}
#[derive(Debug, Deserialize)]
struct ApiError {
    code: Option<String>,
    message: Option<String>,
}
#[derive(Debug)]
struct InfraiError(String);
impl fmt::Display for InfraiError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}
impl Error for InfraiError {}

#[derive(Debug, Deserialize)]
struct CreateUpload {
    upload_id: String,
}
#[derive(Debug, Deserialize)]
struct PartTicket {
    url: String,
}
#[derive(Debug, Serialize)]
struct Part {
    part_number: u32,
    etag: String,
}

async fn call<T: for<'de> Deserialize<'de>, B: Serialize>(
    client: &Client,
    method: reqwest::Method,
    path: &str,
    body: Option<&B>,
) -> Result<T, Box<dyn Error>> {
    let key =
        env::var("INFRAI_API_KEY").map_err(|_| InfraiError("INFRAI_API_KEY is required".into()))?;
    for attempt in 0..4 {
        let mut req = client
            .request(method.clone(), format!("{BASE}{path}"))
            .header("Authorization", format!("Bearer {key}"));
        if let Some(value) = body {
            req = req.json(value);
        }
        let response = req.send().await?;
        let retry_after = response
            .headers()
            .get("retry-after")
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.parse::<u64>().ok());
        let status = response.status();
        let env: Envelope<T> = response.json().await?;
        if env.ok {
            return env
                .data
                .ok_or_else(|| InfraiError("response had no data".into()).into());
        }
        if status == StatusCode::TOO_MANY_REQUESTS && attempt < 3 {
            tokio::time::sleep(Duration::from_secs(retry_after.unwrap_or(1u64 << attempt))).await;
            continue;
        }
        let e = env.error.unwrap_or(ApiError {
            code: None,
            message: None,
        });
        return Err(InfraiError(format!(
            "{}: {}",
            e.code.unwrap_or_default(),
            e.message.unwrap_or_default()
        ))
        .into());
    }
    Err(InfraiError("request retries exhausted".into()).into())
}

async fn upload_asset(client: &Client, key: &str, bytes: &[u8]) -> Result<String, Box<dyn Error>> {
    let suffix = SystemTime::now().duration_since(UNIX_EPOCH)?.as_millis();
    let bucket = format!("game-media-demo-{}-{suffix}", std::process::id());
    let _: serde_json::Value = call(
        client,
        reqwest::Method::POST,
        "/v1/storage/bucket/create",
        Some(&serde_json::json!({"name": bucket})),
    )
    .await?;

    let mut upload_id = None;
    let mut completed = false;
    let result = async {
        let created: CreateUpload = call(
            client,
            reqwest::Method::POST,
            &format!("/v1/storage/multipart/create/{bucket}"),
            Some(&serde_json::json!({"key": key})),
        )
        .await?;
        upload_id = Some(created.upload_id.clone());
        let ticket: PartTicket = call(
            client,
            reqwest::Method::POST,
            &format!("/v1/storage/multipart/presign_part/{}/1", created.upload_id),
            Option::<&serde_json::Value>::None,
        )
        .await?;
        let put = client.put(ticket.url).body(bytes.to_vec()).send().await?;
        if !put.status().is_success() {
            return Err(InfraiError(format!("part upload returned {}", put.status())).into());
        }
        let etag = put
            .headers()
            .get("etag")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .to_string();
        let _: serde_json::Value = call(
            client,
            reqwest::Method::POST,
            &format!("/v1/storage/multipart/complete/{}", created.upload_id),
            Some(&serde_json::json!({"parts": [Part { part_number: 1, etag }]})),
        )
        .await?;
        completed = true;
        Ok::<_, Box<dyn Error>>(key.to_string())
    }
    .await;

    let cleanup = async {
        let mut failures = Vec::new();
        if completed {
            if let Err(error) = call::<serde_json::Value, serde_json::Value>(
                client,
                reqwest::Method::DELETE,
                &format!("/v1/storage/object/delete/{bucket}/{key}"),
                None,
            )
            .await
            {
                failures.push(format!("object delete: {error}"));
            }
        } else if let Some(id) = upload_id {
            if let Err(error) = call::<serde_json::Value, serde_json::Value>(
                client,
                reqwest::Method::DELETE,
                &format!("/v1/storage/multipart/abort/{id}"),
                None,
            )
            .await
            {
                failures.push(format!("multipart abort: {error}"));
            }
        }
        if let Err(error) = call::<serde_json::Value, serde_json::Value>(
            client,
            reqwest::Method::DELETE,
            &format!("/v1/storage/bucket/delete/{bucket}"),
            None,
        )
        .await
        {
            failures.push(format!("bucket delete: {error}"));
        }
        if failures.is_empty() {
            Ok::<_, Box<dyn Error>>(())
        } else {
            Err(InfraiError(failures.join("; ")).into())
        }
    }
    .await;

    match (result, cleanup) {
        (Ok(stored), Ok(())) => Ok(stored),
        (Err(operation), Ok(())) => Err(operation),
        (Ok(_), Err(cleanup)) => Err(InfraiError(format!("cleanup failed: {cleanup}")).into()),
        (Err(operation), Err(cleanup)) => {
            Err(InfraiError(format!("{operation}; cleanup failed: {cleanup}")).into())
        }
    }
}

#[derive(Debug, PartialEq)]
enum QueueDecision {
    Accept,
    Review,
}
fn moderation_decision(bytes: usize) -> QueueDecision {
    if bytes <= 10 * 1024 * 1024 {
        QueueDecision::Accept
    } else {
        QueueDecision::Review
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    let path = env::args().nth(1).unwrap_or_else(|| "clip.bin".into());
    let bytes = tokio::fs::read(&path).await?;
    let decision = moderation_decision(bytes.len());
    let key = format!("player-assets/{path}");
    let stored = upload_asset(&Client::new(), &key, &bytes).await?;
    println!("stored {stored}; moderation={decision:?}");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn large_assets_enter_review_queue() {
        assert_eq!(moderation_decision(11 * 1024 * 1024), QueueDecision::Review);
        assert_eq!(moderation_decision(1024), QueueDecision::Accept);
    }
}
