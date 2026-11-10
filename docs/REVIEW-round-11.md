# Review round 11 (final)

Date: 2026-11-10. Reviewed tree: commit `0614d49` (round-10 fixes), live site
on the same build. Go/free models, same rubric.

- Product / features (`opencode-go/mimo-v2.6-flash`): **8/10**.
- UX / design (`opencode-go/mimo-v2.6-flash`): **8/10**.
- Engineering / security (`opencode-go/deepseek-v4.1-flash`): **7/10**.

## What the critics still saw

Product:
- `development` is selectable in the UI/API but no deploy path accepts it.
- Certificate status outside `--edge` is a hard-coded "external proxy" label.
- Metrics are a 1-hour window over a 5000-row log plus an in-memory ring.
- Preview certificates are per-host (no DNS-01 wildcard); custom domains always
  bind production; no GitLab/Gitea/PR previews.
- Installer/README drift around `DOK_BIN_URL`, compression default and docs.

UX:
- The sign-in page had a stray backslash and a dead Docs link (fixed after the
  review); validation errors on env/proxy forms still lose typed input.
- Type scale is flat (15 px h2, 14 px KPI numbers).
- Minor table/sidebar/action-header conventions and 500 request-id polish.

Engineering:
- Production-branch default was unsafe when no branch was configured (fixed
  after the review: `main` is persisted; unset branches only produce previews).
- The two-phase TLS listener started a certificate-less listener (fixed after
  the review: wait for ACME issuance, then swap; keep the old listener on
  failure).
- Certificate status could not match SANs (fixed after the review with X.509
  SAN parsing).
- Remaining: instance-global ACME renewal/order set, DB access log on the hot
  path, build/container isolation limits, backup coverage for builds/ACME, and
  CI clippy (clippy now passes locally with crate-level allow attributes).

## Fixes landed after the review

Round-10/11 fixes include: blank env values keep the stored value, passkey
listing/removal, build output capped at 2 MB, container capability/pids
hardening, git command timeouts and non-interactive SSH, `Authorization`
stripped after dok basic auth, tag pushes ignored, X-Forwarded-* rebuilt from
trusted values, temp containers not restarting before rename, server-side
memory/CPU clamps, production branch defaults, ACME readiness gating, SAN-based
certificate expiry, proxy settings failing closed, traffic metrics, access-log
pagination, mobile table/empty-state fixes, sign-in/noscript/error-copy fixes,
and a green `cargo clippy -D warnings`.

## Final state

`cargo test`: 73 green. Smoke, auth, git, rollback, preview, UX, Docker smokes
and `e2e-auth` pass. `cargo clippy -- -D warnings` passes. Live:
`https://dok.solta.tech` 200; apps now use `https://<app>.dok.solta.tech`
(the base domain was changed from `apps.solta.tech` in a concurrent packaging
commit; `hello.apps.solta.tech` therefore returns 404).

Scores remain below the 9 target. Remaining gaps are architectural rather than
small nits: per-host ACME orders/wildcards, durable analytics, a real
development environment, front-proxy certificate introspection, and complete
backup/restore of build artifacts.
