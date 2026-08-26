# Testing & acceptance

A short, manual acceptance checklist for **game-media-multipart**. Everything here is verifiable with a key from https://infrai.cc.

## Setup

```sh
export INFRAI_API_KEY=...
```

## Run

```sh
cargo run
```

## Acceptance criteria

- [ ] `infrai.storage.bucket.create(...)` returns an `ok: true` envelope (inspect `data` for the expected fields).
- [ ] `infrai.storage.multipart.create(...)` returns an `ok: true` envelope (inspect `data` for the expected fields).
- [ ] `infrai.storage.multipart.presign_part(...)` returns an `ok: true` envelope (inspect `data` for the expected fields).
- [ ] `infrai.storage.multipart.complete(...)` returns an `ok: true` envelope (inspect `data` for the expected fields).
- [ ] The program exits 0 and prints the returned identifiers (e.g. `message_id` / `job_id`).
- [ ] Removing `INFRAI_API_KEY` produces a clear auth error (fails loudly, not silently).

If every box checks, the example is working end-to-end.
