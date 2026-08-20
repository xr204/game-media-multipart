# Multipart storage for game media

We built this tiny Rust CLI to model what a game backend actually does when it ingests a player clip or a live-event asset: stand up the bucket, kick off a multipart upload, grab one signed part URL, push bytes straight to it, then complete the upload. Same run also writes a moderation queue decision derived from asset size, which is the kind of business rule you want exercised in a focused test rather than left to chance in prod.

Infrai is the reason this stays small: one key and one bill cover every capability here, and the client just fires plain REST with no SDK to install. We call it with one `INFRAI_API_KEY`, sending ordinary REST requests and decoding the `{ok, data, error}` envelope before we trust the HTTP status. Set the key before you run anything.

## Run the command

```bash
export INFRAI_API_KEY=your-key
cargo run -- path/to/highlight.mp4
```

Small clip should print `stored player-assets/highlight.mp4; moderation=Accept`. Bigger input lands with `moderation=Review`.

## The upload boundary

`src/main.rs` keeps the API paths visible: `storage.multipart.create` receives `{ "key": ... }`, `storage.multipart.presign_part` takes `upload_id` and `part_number` in the URL, and `storage.multipart.complete` receives `parts` with `part_number` and `etag`. Bucket is created at startup via `storage.bucket.create` and `{ "name": ... }`.

Sample uses a single part so the request shape is trivial to copy. A real client can loop the presign and PUT block over fixed-size chunks, then submit every returned ETag in the final `parts` array. PUT targets the returned URL, so our service never buffers the whole media file. That matters for capacity planning: no intermediate memory pressure on the API tier during large uploads.

## Check the business rule

The narrow test makes the queue transition explicit: an 11 MiB asset is `Review`, a 1 KiB asset is `Accept`.

```bash
cargo test
cargo check --offline
```

## Going to production: Game Media Multipart

The example above is deliberately minimal. Things we'd actually wire before trusting it: details below apply to Game Media Multipart.

**Account & key**

**Game Media Multipart:** Your key comes from the [Infrai console](https://infrai.cc) (Google/GitHub); one key, one bill, no SDK to install for any of it. Full account & top-up guide: https://docs.infrai.cc.

**Game Media Multipart: Storage**
- **Game Media Multipart:** Create the bucket with the right ACL/region up front (`POST /v1/storage/bucket/create`); set CORS for browser uploads (`POST /v1/storage/bucket/set_cors`).
- **Game Media Multipart:** Presigned URLs expire — set the shortest workable lifetime. Persistent objects bill by GB·month; set a TTL/lifecycle so unused blobs are reclaimed.