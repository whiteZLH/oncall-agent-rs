# oncall-agent-rs

Rust service scaffold for the on-call assistant, built with `axum`.

## Run

```bash
cargo run
```

Override configuration if needed:

```bash
APP_HOST=0.0.0.0 APP_PORT=3000 cargo run
APP_ALLOWED_ORIGIN=http://localhost:5173 APP_REQUEST_TIMEOUT_SECS=15 cargo run
```

## Endpoints

- `GET /health`
- `GET /ready`
- `GET /metrics`
- `POST /api/chat`
- `GET /api/incidents`

Every response includes an `x-request-id` header for trace correlation.

## Examples

```bash
curl http://127.0.0.1:3000/health
curl http://127.0.0.1:3000/ready
curl http://127.0.0.1:3000/metrics
curl -X POST http://127.0.0.1:3000/api/chat -H "Content-Type: application/json" -d "{\"message\":\"hello\"}"
curl http://127.0.0.1:3000/api/incidents
```

## Validation

```bash
cargo test
```
