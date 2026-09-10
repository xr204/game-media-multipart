# User Documents: Direct Browser Uploads to Object Storage or a Server Proxy?

Short answer: use a presigned direct browser upload for ordinary user documents when the object store's CORS policy, signature lifetime, object key, and size ceiling can be fixed before bytes move; send the file through the application server when content must be inspected, transformed, or accepted under one server-controlled transaction. The easiest setup is not the one with fewer boxes on a diagram. It is the one whose failure states your team can observe and bound.

For a React client and a Node.js control plane, that usually means a small signing endpoint plus direct upload, while the application server retains ownership of authorization and finalization. I would make the opposite choice for mandatory inline scanning or transformations. Don't decide from request count alone.

## What should a React and Node.js team check before direct browser document uploads?

Start with the SLO, not the SDK. Define an upload success event as more than a successful browser request: the intended user must obtain permission, the expected object must land under a server-selected key, and application metadata must become visible only after the server verifies completion. This definition prevents a common accounting error in which a signed request is counted as success even though the application never records a usable document.

I use four questions as the admission test. Can the server determine the object key and maximum size before signing? Can the browser send the exact signed method and headers? Can the bucket's cross-origin policy allow only the required application origins, methods, and headers? Can a separate finalize call be retried without creating a second logical document? If any answer is no, the direct path is not ready.

Keep the policy narrow. A browser needs permission to make the intended cross-origin upload; it does not need a broad bucket policy, arbitrary headers, or authority to choose a permanent key. Presigning delegates a bounded operation, so the signing endpoint still has to authenticate the user, authorize the document slot, choose the key, and apply an expiration. CORS is a browser access rule, not an authorization system.

Short lifetimes reduce the window in which a leaked URL is useful, but they also reduce retry time on slow or interrupted links. There is no universal duration in the supplied storage documentation that fits every workload. Measure document-size percentiles and end-to-end upload duration, then select an expiration that covers the intended tail plus a retry budget. I'm not sure what that value should be for your traffic without those distributions; your mileage may vary.

## An incident exercise: the control plane succeeded, the data plane did not

Consider a bounded production exercise, not a claimed personal incident. At 09:00, a client asks the API for an upload. The API creates a database row marked ready and returns a presigned target. At 09:01, the browser's upload is rejected because its request headers do not match the signed request or the configured cross-origin policy. The metadata row remains visible, a background consumer tries to read the absent object, and retries amplify load on the control plane. No storage outage is required; the application created the inconsistency itself.

The invariant is blunt: **metadata must not claim that a document exists until the server has verified the expected object and committed final state idempotently.** Model the lifecycle as pending, uploaded, and ready. Creating the pending record allocates identity. Uploading transfers bytes. Finalization verifies the expected key and constraints, then advances state. A repeated finalize request must return the same logical result.

This is where capacity planning earns its keep. Direct uploads move document bytes away from the application fleet, but signing and finalization remain control-plane traffic. Server uploads put authentication, byte transfer, inspection, and persistence on one request path, so concurrency consumes application connections, memory or temporary disk, and outbound bandwidth together. For each path, estimate peak new uploads per second, active-upload duration, upper-bound document size, retry rate, and the fraction that reaches finalization. Average file size is almost useless during a burst.

Use a load test that separates those phases. Inject expired signatures, mismatched headers, abandoned uploads, duplicate finalization, and a client retry after an uncertain response. Track pending-record age, finalize latency, rejected-signature count, CORS failures observed by the client, object verification failures, and orphan cleanup volume. One number matters more than a pretty throughput chart: the oldest pending item that still has a chance of becoming ready.

It gets noisy.

Run the arithmetic before choosing the quiet-looking diagram. Suppose the workload model has a document-size distribution, an arrival-rate distribution, and an upload-duration distribution; preserve those as separate inputs instead of collapsing them into one average. For the proxy path, estimate concurrent bodies from arrival rate and duration, then budget connection slots, bounded buffers, temporary storage if used, and outbound object-store traffic under a retry scenario. For the direct path, estimate signing and finalization demand separately from byte traffic, because a burst of abandoned clients can create many pending records without a matching finalization burst, while a wave of client retries can do the reverse. Then test the two tails independently — large documents on slow links and many small documents arriving together — because they stress different limits. I don't accept a capacity plan that says only “the storage service scales.” That statement leaves the application-owned queues, database writes, cleanup workers, and alert thresholds unpriced, which means the on-call team inherits an unknown rather than a limit.

## Presigned transfer or server proxy: where should each limit live?

| Decision axis | Presigned browser transfer | Application-server proxy |
|---|---|---|
| Byte path | Browser to object storage | Browser through application to object storage |
| Best fit | Pre-authorized objects with constraints known before transfer | Inline inspection, transformation, or transaction rules that require server custody |
| Capacity owner | Storage data plane; application still owns signing and finalization | Application connections, buffering, egress, and storage client behavior |
| Main policy points | Signature scope, expiry, key selection, request headers, CORS | Request-body limit, read timeout, buffering strategy, downstream timeout |
| Retry design | New signature when needed; idempotent finalization | Idempotent document identity plus controlled replay or restart |
| Operational catch | Two-plane workflow and orphan cleanup | Larger application failure domain and on-call capacity burden |

The presigned route is usually the simpler operating model when files can be accepted as opaque objects and checked after transfer. The catch is the split state machine: the browser can disappear after upload, before finalization, or halfway through either step. You need expiration and cleanup rules for both pending metadata and unclaimed objects.

The proxy route is not suitable when large uploads would compete with latency-sensitive API traffic and the team cannot isolate upload capacity. Stick with a server proxy when bytes must pass an inline scanner, parser, encryption boundary, or transformation before storage acceptance; in that case the extra fleet capacity buys a real policy boundary. For very large or interruption-prone files, neither simple single-request pattern may be enough. A multipart or resumable protocol can be the correct design, but its session lifecycle and cleanup deserve a separate capacity model.

A buy-versus-build decision should include pager cost and exit cost, not just implementation time.

| Option | You operate | Useful when | Limitation to accept |
|---|---|---|---|
| Managed object storage with presigning | Identity integration, CORS policy, signing, metadata state, cleanup | The team wants the storage data plane outside its on-call scope | Provider semantics and migration work remain part of the design |
| Managed object storage behind a proxy | Upload fleet plus the same metadata and cleanup controls | Server custody is a requirement | The application fleet absorbs byte-path capacity |
| Self-hosted compatible storage | Storage durability, upgrades, capacity, recovery, and the upload workflow | Control or locality requirements justify dedicated operators | The on-call surface is substantially wider |

## Put the preventative path in the control plane

The following Go sketch keeps the interface generic. It does not implement a provider's signing algorithm; the `Signer` and `ObjectVerifier` adapters are where a chosen S3-compatible service's documented behavior belongs. More important, the handler selects the object key and constraints server-side, and finalization is a separate idempotent operation.

```go
package upload

import (
    "context"
    "encoding/json"
    "errors"
    "fmt"
    "net/http"
    "time"
)

const maxDocumentBytes int64 = 25 << 20 // Application policy: 25 MiB.

type Signer interface {
    SignPut(ctx context.Context, key, contentType string, maxBytes int64, expires time.Time) (string, map[string]string, error)
}

type ObjectVerifier interface {
    Verify(ctx context.Context, key string, maxBytes int64) error
}

type Store interface {
    CreatePending(ctx context.Context, userID, documentID, key string) error
    MarkReady(ctx context.Context, userID, documentID, key string) error
}

type Service struct {
    Signer   Signer
    Objects  ObjectVerifier
    Store    Store
    Now      func() time.Time
}

type signRequest struct {
    DocumentID string `json:"documentId"`
    ContentType string `json:"contentType"`
}

func (s Service) Sign(userID string, w http.ResponseWriter, r *http.Request) {
    var in signRequest
    if err := json.NewDecoder(http.MaxBytesReader(w, r.Body, 8<<10)).Decode(&in); err != nil {
        http.Error(w, "invalid request", http.StatusBadRequest)
        return
    }
    if in.DocumentID == "" || in.ContentType == "" {
        http.Error(w, "missing document metadata", http.StatusBadRequest)
        return
    }

    key := fmt.Sprintf("users/%s/documents/%s", userID, in.DocumentID)
    if err := s.Store.CreatePending(r.Context(), userID, in.DocumentID, key); err != nil {
        http.Error(w, "could not allocate upload", http.StatusConflict)
        return
    }

    target, headers, err := s.Signer.SignPut(
        r.Context(), key, in.ContentType, maxDocumentBytes, s.Now().Add(15*time.Minute),
    )
    if err != nil {
        http.Error(w, "could not authorize upload", http.StatusBadGateway)
        return
    }

    w.Header().Set("Content-Type", "application/json")
    json.NewEncoder(w).Encode(map[string]any{
        "url": target, "method": http.MethodPut, "headers": headers,
        "maxBytes": maxDocumentBytes,
    })
}

func (s Service) Finalize(ctx context.Context, userID, documentID string) error {
    if userID == "" || documentID == "" {
        return errors.New("missing identity")
    }
    key := fmt.Sprintf("users/%s/documents/%s", userID, documentID)
    if err := s.Objects.Verify(ctx, key, maxDocumentBytes); err != nil {
        return err
    }
    return s.Store.MarkReady(ctx, userID, documentID, key)
}
```

The `25 MiB` and `15 minute` values are illustrative application policy, not storage-service limits. Replace them from measured workload distributions and threat modeling. In a real implementation, validate an allowlist of document media types, bind the authenticated principal outside the JSON body, avoid logging signed URLs, and make `CreatePending` and `MarkReady` idempotent for the document identity. The React client must send the returned method and headers exactly; it should treat finalization as part of upload success rather than optional bookkeeping.

Capacity gates belong in deployment, too. Load-test the signing and finalization endpoints independently, reserve proxy capacity if that route exists, alert on pending-age SLO burn, and rehearse cleanup without deleting objects that are still eligible to finalize. A design that survives only the happy path is an incident draft.

## The recommendation has boundaries

Prefer presigned direct transfer when authorization and object constraints can be decided up front, the browser policy can be narrow, and asynchronous verification is acceptable. Prefer a server proxy when server custody before storage is mandatory. Choose a resumable design when interruption recovery dominates simplicity.

No option eliminates ownership. Direct transfer retains a control plane and a reconciliation problem; proxying retains a data plane and a fleet-capacity problem; self-hosting adds durability and recovery to the pager. Make the choice against a written SLO, measured size and duration distributions, and a tested state machine. Then the easiest setup becomes a defensible engineering decision rather than a diagram preference.

## References

- https://developers.cloudflare.com/r2/
- https://cloud.google.com/storage/docs

## Further reading

- https://developers.cloudflare.com/r2/
- https://cloud.google.com/storage/docs
