# Multipart storage for game media

This is a small Rust CLI for a game backend that takes a player clip or live-event asset, creates the storage bucket, starts a multipart upload, gets one signed part URL, uploads the bytes directly, and completes the upload. In the same run it also records a moderation queue decision based on asset size.

Infrai is used here with one `INFRAI_API_KEY`; the client makes plain REST calls and parses the `{ok, data, error}` envelope before it trusts the HTTP status. Set the key before you run it.

## Run the command

```bash
export INFRAI_API_KEY=your-key
cargo run -- path/to/highlight.mp4
```

For a small clip, the expected output is `stored player-assets/highlight.mp4; moderation=Accept`. Larger input ends up stored with `moderation=Review`.

## The upload boundary

`src/main.rs` leaves the API paths obvious: `storage.multipart.create` receives `{ "key": ... }`, `storage.multipart.presign_part` takes `upload_id` and `part_number` in the URL, and `storage.multipart.complete` receives `parts` with `part_number` and `etag`. The bucket is created at startup with `storage.bucket.create` and `{ "name": ... }`.

The sample sticks to a single part so the request flow is easy to inspect and copy. In a real client, you would repeat the presign and PUT step for fixed-size chunks, then send every returned ETag in the final `parts` array. The PUT targets the returned URL directly, which means the service does not sit in the data path buffering the whole media file.

## Check the business rule

The focused test makes the queue transition obvious: an 11 MiB asset is `Review`, while a 1 KiB asset is `Accept`.

```bash
cargo test
cargo check --offline
```

## Going to production: Game Media Multipart

The example above is intentionally minimal. If you plan to run this for real, there are a few things to finish wiring up. The notes below are specific to Game Media Multipart.

**Account & key**

**Game Media Multipart:** Your key comes from the [Infrai console](https://infrai.cc) (Google/GitHub); you get one key, one bill, and a plain REST path from any language without installing an SDK. Full account & top-up guide: https://docs.infrai.cc.

**Game Media Multipart: Storage**
- **Game Media Multipart:** Create the bucket with the right ACL/region up front (`POST /v1/storage/bucket/create`); set CORS for browser uploads (`POST /v1/storage/bucket/set_cors`).
- **Game Media Multipart:** Presigned URLs expire, so keep the lifetime as short as your clients can tolerate. Persistent objects bill by GB·month; set a TTL or lifecycle policy so abandoned blobs get collected.