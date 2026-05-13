# Network: Protecting Your Node Behind an Nginx Proxy

This guide shows how to sit an **nginx** reverse proxy in front of your
sparkl-solo node to rate-limit critical HTTP paths, smooth out bursty
traffic, and keep the axum server from getting swamped under load.

---

## Why you need a proxy

The sparkl-solo axum server listens on a single TCP port
(`inference_port`, default `8080`). Without a proxy every HTTP client
talks directly to the Rust process. Under heavy load this means:

- Slow clients hold open connections and block the event loop.
- A sudden burst of `/v1/chat/completions` POSTs can pin CPU for
  several seconds per request.
- Health-checks (`/health`) without burst protection can get 429'd
  unexpectedly during a rate-limit spike.

An nginx proxy absorbs connection churn, applies token-bucket rate
limits per IP, and bails out excess traffic before it reaches your
node.

---

## Endpoint map

The following routes are served by the axum router:

| Path                              | Method | Purpose                                    |
|-----------------------------------|--------|--------------------------------------------|
| `/health`                          | GET    | Lightweight liveness check                 |
| `/status`                          | GET    | Node status summary                         |
| `/status/detail`                   | GET    | Extended status (session counts, receipts) |
| `/attestation/challenge`           | POST   | TPM/NRAS attestation challenge             |
| `/receipts/verify`                 | POST   | Receipt verification                       |
| `/receipts/proof/{sid}/{seq}`     | GET    | Per-receipt proof fetch                    |
| `/v1/models`                       | GET    | List available models                      |
| `/v1/chat/completions`             | POST   | Chat completion (the hot path)             |

---

## Nginx configuration

### Prerequisites

```bash
sudo apt install nginx          # or: brew install nginx
```

### Full config example

```nginx
# ── Rate limit zones (shared memory, keyed by client IP) ──

# Critical paths (tightest): attestation + receipts
limit_req_zone $binary_remote_addr zone=strict:10m       rate=5r/s;

# v1/ endpoints (moderate): chat completions and model list
limit_req_zone $binary_remote_addr zone=inference:10m    rate=10r/s;

# Health-checks (generous)
limit_req_zone $binary_remote_addr zone=health:10m       rate=30r/s;
```

### http block

```nginx
http {
    # ── Rate limit zones ──
    limit_req_zone $binary_remote_addr zone=strict:10m       rate=5r/s;
    limit_req_zone $binary_remote_addr zone=inference:10m   rate=10r/s;
    limit_req_zone $binary_remote_addr zone=health:10m      rate=30r/s;

    server {
        listen 80;
        server_name sparkl.example.com;

        # ── Upstream ──
        upstream sparkl_node {
            server 127.0.0.1:8080;
            # Add keepalive to the axum server
            keepalive 16;
        }

        # ── Default response when burst is exceeded ──
        limit_req_status 429;

        # ── /v1/ endpoints ──
        # Moderately rate-limited; burst=20 with nodelay handles
        # concurrent clients hammering the chat endpoint.
        location /v1/ {
            limit_req zone=inference burst=20 nodelay;
            proxy_pass http://sparkl_node;
            proxy_http_version 1.1;
            proxy_set_header Host $host;
            proxy_set_header X-Real-IP $remote_addr;
        }

        # ── /attestation and /receipts (strict) ──
        # These are the identity-critical paths. A sudden burst
        # of POST /attestation/challenge can eat CPU, and
        # /receipts/verify is consensus-critical.
        location /attestation/ {
            limit_req zone=strict burst=10 nodelay;
            proxy_pass http://sparkl_node;
            proxy_http_version 1.1;
            proxy_set_header Host $host;
            proxy_set_header X-Real-IP $remote_addr;
        }

        location /receipts/ {
            limit_req zone=strict burst=10 nodelay;
            proxy_pass http://sparkl_node;
            proxy_http_version 1.1;
            proxy_set_header Host $host;
            proxy_set_header X-Real-IP $remote_addr;
        }

        # ── Health/status (generous) ──
        location /health {
            limit_req zone=health burst=50 nodelay;
            proxy_pass http://sparkl_node;
            proxy_http_version 1.1;
            proxy_set_header Host $host;
            proxy_set_header X-Real-IP $remote_addr;
        }

        location /status {
            limit_req zone=health burst=30 nodelay;
            proxy_pass http://sparkl_node;
            proxy_http_version 1.1;
            proxy_set_header Host $host;
            proxy_set_header X-Real-IP $remote_addr;
        }

        # ── Fallback: catch-all ──
        location / {
            limit_req zone=inference burst=15 nodelay;
            proxy_pass http://sparkl_node;
            proxy_http_version 1.1;
            proxy_set_header Host $host;
            proxy_set_header X-Real-IP $remote_addr;
        }
    }
}
```

---

## Configuration explained

### limit_req_zone

```
limit_req_zone $binary_remote_addr zone=NAME:SIZE rate=RATES;
```

| Parameter | Meaning |
|-----------|---------|
| `$binary_remote_addr` | Groups requests by the client's IPv4/IPv6 address |
| `zone=NAME:SIZE` | Shared memory bucket (e.g. `inference:10m` holds ~160k IPs at 10MB) |
| `rate=Xr/s` | Steady-state throughput per IP (1 req every `1000/X` ms) |

### limit_req

```
limit_req zone=ZONE burst=N nodelay;
```

| Parameter | Meaning |
|-----------|---------|
| `burst=N` | Allow up to N excess requests before returning 429 |
| `nodelay` | Serve the burst immediately instead of spacing them out |
| (no `nodelay`) | Hold excess requests in a queue and drip-feed them |

### Recommended rate tiers

| Path              | Rate       | Burst      | Use case                                      |
|-------------------|------------|------------|-------------------------------------------------|
| `/v1/chat/*`      | 10 r/s / IP | 20         | Chat completions (main hot path)               |
| `/v1/models`      | 10 r/s / IP | 20         | Model list polling                           |
| `/attestation/*`  | 5 r/s / IP  | 10         | Attestation challenge/resolution             |
| `/receipts/*`     | 5 r/s / IP  | 10         | Receipt proof & verification                   |
| `/health`         | 30 r/s / IP | 50         | Kubernetes liveness probe (hammered by K8s)   |
| `/status`         | 30 r/s / IP | 30         | Dashboard polling                            |

---

## Deployment step-by-step

### 1. Place the config

```bash
sudo cp /path/to/nginx.conf /etc/nginx/nginx.conf
```

### 2. Point nginx at your node

If your node runs on the default port, the upstream is ready:

```
server 127.0.0.1:8080;
```

For a custom port, override `inference_port` in your `default.toml` or
via CLI:

```bash
cargo run --features mock-tpm -- --inference-port 8080
```

### 3. Restart the proxy

```bash
sudo nginx -t && sudo systemctl reload nginx
```

### 4. Bind your node to localhost

To prevent clients hitting the axum port directly, change this in your config or pass via CLI:

```bash
cargo run --features mock-tpm -- \
  --listen-addrs "127.0.0.1:8080" \
  --external-ip "$(curl -s ifconfig.me)"
```

---

## Tuning for real-world deployments

### Scenario A: High-traffic public node

When exposing your node on an AnyCPU VM, increase the zone memory and
add more burst:

```nginx
limit_req_zone $binary_remote_addr zone=inference:50m  rate=25r/s;

location /v1/ {
    limit_req zone=inference burst=50 nodelay;
    proxy_pass http://sparkl_node;
}
```

### Scenario B: Farm mode with 3-5 sibling nodes

If you run `node1`, `node2`, `node3` on the same machine, differentiate
them either by subdomain or by adding `X-Node-Name`:

```nginx
location = / {
    set $upstream_port 8081;   # node1
    proxy_pass http://127.0.0.1:$upstream_port;
}
```

Or use an upstream pool:

```nginx
upstream sparkl_farm {
    server 127.0.0.1:8081;
    server 127.0.0.1:8082;
    server 127.0.0.1:8083;
}
```

### Scenario C: NGINX + Let's Encrypt (HTTPS)

```nginx
server {
    listen 443 ssl http2;
    server_name sparkl.example.com;

    ssl_certificate     /etc/letsencrypt/live/sparkl.example.com/fullchain.pem;
    ssl_certificate_key /etc/letsencrypt/live/sparkl.example.com/privkey.pem;

    # --- same location blocks as above ---
    location /v1/ {
        limit_req zone=inference burst=20 nodelay;
        proxy_pass http://sparkl_node;
        proxy_set_header X-Real-IP      $remote_addr;
        proxy_set_header X-Forwarded-For $proxy_add_x_forwarded_for;
    }
}
```

---

## Using `limit_conn` alongside `limit_req`

For nodes that serve long-lived `/v1/chat/completions` sessions (e.g.
SSE or JSON-lines), connection limits prevent a single IP from
consuming all file-descriptors:

```nginx
# Limit: 5 concurrent connections per IP
limit_conn_zone $binary_remote_addr zone=conn_per_ip:10m;

server {
    limit_conn conn_per_ip 5;

    location /v1/ {
        limit_req zone=inference burst=20 nodelay;
        proxy_pass http://sparkl_node;
    }
}
```

---

## Troubleshooting

### Check current rate-limit state

```bash
# See 429 responses in the access log
tail -f /var/log/nginx/access.log | grep " 429 "

# Force a 429 (test with curl)
for i in $(seq 1 30); do
  curl -so /dev/null -w "%{http_code}\n" http://localhost/v1/models
  sleep 0.1
done
```

### Too many 503s instead of 429s?

Your burst is too small for the upstream's response time. Increase
`burst` or enable `delay` to spread requests:

```nginx
location /v1/ {
    limit_req zone=inference burst=30;       # nodelay removed
    proxy_pass http://sparkl_node;
}
```

### Rate limits not applying?

Make sure the key matches. For a single-IP public node, test with
`$binary_remote_addr`; if behind a load balancer, switch to
`$http_x_forwarded_for`:

```nginx
limit_req_zone $http_x_forwarded_for zone=inference:10m rate=10r/s;
```

---

## Summary

| Goal                      | Directive        | Where to put it    |
|---------------------------|-------------------|--------------------|
| Rate-limit /v1/          | `limit_req_zone` | `http {}`          |
| Rate-limit /attestation/ | `limit_req_zone` | `http {}`          |
| Rate-limit /receipts/    | `limit_req_zone` | `http {}`          |
| Apply per location       | `limit_req`      | `location {}`    |
| Concurrency cap            | `limit_conn`     | `server {}`      |

This setup protects your node from getting DDoS'd by slow clients
while keeping the inference backend responsive for consensus-critical
receipts and attestation.
