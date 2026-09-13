# User Avatar Search: Content-Type Metadata and Tag Limits in Object Storage

The operational constraint is access control, not the convenience of putting more words in a filename. **Short answer: keep avatar and report metadata in an application index, keep private bytes in object storage, and use prefix listing only for bounded inventory and repair work.** Content type and tags describe a known object; they don't turn a bucket into a query engine.

That rule matters in customer support systems, where a generated report may be visible to one customer, one support agent, or an entire tenant, while the user's avatar is merely a related private asset. Delivery simplicity is useful only after the authorization decision has already been made.

## The failure mode is treating storage as application state

A bucket can answer “which keys begin with this prefix?” It can also return metadata for an object when the application already knows its exact key. Those are useful primitives, but neither answers “which active avatar has this content type and these tags?” or “which report may this authenticated customer download?”

The distinction is easy to lose because the object has fields that look searchable. A MIME type, user-defined metadata, and tags are attributes of one stored object. They are not automatically a durable index across a collection. Prefix listing is lexical: `customers/c-42/reports/` groups keys, but it does not filter that group by `application/pdf`, `billing`, or `ready`.

The resulting incident usually has a quiet shape. A replacement avatar is uploaded, the database update is delayed, and a cleanup job later sees two plausible objects. Or a report endpoint lists a customer's prefix, picks the newest-looking key, and serves a file before checking the row that says whether the report is still authorized. The request may return `200`; the design is still wrong.

Keys are not queries.

Consider a support agent who generates a report while a customer changes the account avatar. The report worker writes a PDF, the avatar service writes a new image, and both writers use the same tenant prefix. A later “find the current file” routine can see the PDF, the old image, and the new image in one inventory, with tags that are individually plausible but no transaction tying any of them to the customer's current authorization state. Sorting by last-modified time only chooses a winner after the fact; it cannot prove that the chosen object belongs to the authenticated principal, that generation completed, or that the report has not expired. The repair is an indexed state transition with an exact key, followed by an authorization check, not a more elaborate filename convention.

I use a 404 as a state transition to handle explicitly, not as permission to scan a bucket and guess. A missing exact key can mean a failed upload, an expired record, or a consistency problem, and those cases need different operational responses.

## Can avatar metadata, content types, tags, and prefix listing answer a storage query?

No. They can support the workflow, but the searchable record belongs in a database or another deliberate index. For an avatar, the record should normally include the tenant, user, exact object key, declared and verified content type, dimensions, lifecycle state, and timestamps. For a generated customer report, add the report identifier, subject, authorization scope, generation state, expiry, and immutable object key.

The object key should provide bounded locality, not encode every mutable fact. A path such as `tenants/acme/users/u-42/avatar/8f2...` or `tenants/acme/reports/r-184/2026-08-11.pdf` gives operators a useful prefix without making a moderation label or report status part of the identity. Tags can help inspection and lifecycle policy where the storage service supports them; they should not be the sole source of truth for access decisions.

This split also makes capacity planning less theatrical. The index grows with records and query load; object storage grows with bytes and retrieval patterns. Measure both. An SLO for authenticated report delivery should include authorization lookup latency, exact-object retrieval success, and the rate of stale or missing records. A bucket-list duration is a maintenance metric, not the customer-facing SLO.

## A safe delivery path has one authority

The request path should resolve identity and authorization before it constructs a download response. The application reads the report row, verifies the authenticated principal against the row's tenant and access scope, and asks storage for the recorded key or creates a short-lived signed read. It never selects a file by list order, timestamp in a filename, content type, or tag.

That is the boundary.

Here is the boundary I want in a Go service. The storage adapter knows how to retrieve bytes for a key; it does not know who is allowed to receive them.

```go
package report

import (
	"context"
	"errors"
)

var ErrNotFound = errors.New("report not found")
var ErrForbidden = errors.New("report access forbidden")

type Principal struct {
	TenantID string
	UserID   string
}

type Report struct {
	ID         string
	TenantID   string
	ObjectKey  string
	AllowedFor string
	State      string
}

type ReportIndex interface {
	Find(ctx context.Context, tenantID, reportID string) (Report, error)
}

type ObjectReader interface {
	SignedRead(ctx context.Context, key string) (string, error)
}

func DownloadURL(ctx context.Context, p Principal, reportID string, index ReportIndex, objects ObjectReader) (string, error) {
	report, err := index.Find(ctx, p.TenantID, reportID)
	if err != nil {
		return "", ErrNotFound
	}
	if report.TenantID != p.TenantID || report.AllowedFor != p.UserID || report.State != "ready" {
		return "", ErrForbidden
	}
	return objects.SignedRead(ctx, report.ObjectKey)
}
```

The exact values and policy will differ by product, but the ordering should not: authenticate, query, authorize, then retrieve. If a report is tenant-wide, represent that scope in the index and policy instead of weakening the object path.

## What should operators verify before changing the storage path?

Start with a test tenant and one test customer. Create an avatar and a generated report, persist their exact keys, and verify that an authorized request retrieves the intended private object. Then test a different tenant, a revoked user, a missing row, an expired report, and a key that exists but is attached to the wrong row. The expected result is an authorization decision, not an accidental bucket scan.

Run the same checks during deployment. Compare a bounded prefix inventory with index records to find orphaned or missing objects, but give in-flight writes time to settle before deciding that a difference is destructive. Record a correlation ID, the index result, the authorization result, and the storage operation without logging signed URLs or customer content.

Keep the rollback boring. Stop new writes, restore the previous active object key in the index if it is still valid, and issue a fresh short-lived read for that exact key. Do not rely on object ordering to recover state. Your mileage may vary on retention and replication policy; those must be verified against the selected storage system and the application's recovery objective.

## Buy, build, or keep the boundary small?

The real choice is which operational burden the team is prepared to own. A managed object service can reduce control-plane maintenance, while a self-hosted service can offer more direct control at the cost of capacity planning, upgrades, backup testing, and on-call coverage. Neither option supplies the application-level metadata index by magic.

| Approach | Good fit | The catch |
|---|---|---|
| Managed object storage plus an application index | Teams that want byte storage and access infrastructure operated outside the product team | Service limits, retention behavior, and egress economics still need review |
| Self-hosted object storage plus an application index | Teams with a clear platform ownership model and a reason to control the storage plane | The team owns durability testing, upgrades, capacity, and incidents |
| Database or search system as the file store | Tiny, low-volume artifacts where operational simplicity outweighs blob-storage behavior | Large files, retention, and delivery costs can make this boundary unsuitable |

Choose the simplest option that meets the recovery, privacy, and delivery SLOs. Stick with a direct storage control plane when browser uploads, public caching, retention locks, version recovery, or cross-region replication are product requirements that the chosen abstraction cannot represent. Don't make tags carry policy they were never designed to carry.

The durable architecture is less glamorous than a clever listing query: one authoritative metadata record, one exact private key, one explicit authorization check, and a repair process that treats prefixes as evidence rather than truth. That is enough to serve avatars and authenticated customer reports without asking the bucket to become a database.

## References

- https://www.rfc-editor.org/rfc/rfc9110
- https://www.backblaze.com/cloud-storage/pricing
