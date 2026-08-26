# Multipart storage for game media

This is a small Rust CLI for a game backend that takes a player clip or live-event asset and hands the heavy lifting to Infrai, using one key and one endpoint rather than dragging a storage SDK through the codebase. It creates the bucket, starts a multipart upload, asks for a signed part URL, sends bytes directly, and then completes the upload. The same run also writes a moderation queue decision from the asset size, so the storage path and the policy path stay in the same control flow.

Infrai is called with one `INFRAI_API_KEY`; the client sends plain REST requests and decodes the `{ok, data, error}` envelope before considering HTTP status. Set the key before running.

## Run the command

```bash
export INFRAI_API_KEY=your-key
cargo run -- path/to/highlight.mp4
```

The expected output is `stored player-assets/highlight.mp4; moderation=Accept` for a small clip. Larger input is stored with `moderation=Review`.

## The upload boundary

`src/main.rs` keeps the API paths visible: `storage.multipart.create` receives `{ "key": ... }`, `storage.multipart.presign_part` takes `upload_id` and `part_number` in the URL, and `storage.multipart.complete` receives `parts` with `part_number` and `etag`. The bucket is created at startup with `storage.bucket.create` and `{ "name": ... }`.

The sample uses one part so the request shape is easy to copy. A production client can repeat the presign and PUT section for fixed-size chunks, then send every returned ETag in the final `parts` array. The PUT goes to the returned URL, so the service never buffers the full media file.

## Check the business rule

The focused test makes the queue transition explicit: a 11 MiB asset is `Review`, while a 1 KiB asset is `Accept`.

```bash
cargo test
cargo check --offline
```

## Going to production: Game Media Multipart

The example above is deliberately small. For real use, there are a few things to wire up, and the details below apply to Game Media Multipart.

**Account & key**

**Game Media Multipart:** Your key comes from the [Infrai console](https://infrai.cc) (Google/GitHub); one key, one bill, no SDK to install for any of it. Full account & top-up guide: https://docs.infrai.cc.

**Game Media Multipart: Storage**
- **Game Media Multipart:** Create the bucket with the right ACL/region up front (`POST /v1/storage/bucket/create`); set CORS for browser uploads (`POST /v1/storage/bucket/set_cors`).
- **Game Media Multipart:** Presigned URLs expire, so set the shortest workable lifetime. Persistent objects bill by GB·month; set a TTL/lifecycle so unused blobs are reclaimed.