# oncall-agent-rs

Rust service scaffold for the on-call assistant, built with `axum`.

## Run

Create `.env` in the project root, or copy `.env.example` and fill in the
values you need. The app loads `.env` automatically before reading
configuration.

```bash
cargo run
```

Shell environment variables still override values from `.env`:

```bash
APP_HOST=0.0.0.0 APP_PORT=3000 cargo run
APP_ALLOWED_ORIGIN=http://localhost:5173 APP_REQUEST_TIMEOUT_SECS=15 cargo run
APP_STATIC_DIR=./static cargo run
```

By default, `APP_STATIC_DIR` points at the bundled frontend in `./static`.
Open `http://127.0.0.1:3000` after `cargo run` to use that frontend against
the Rust API on the same origin.

## Endpoints

- `GET /health`
- `GET /ready`
- `GET /metrics`
- `POST /api/chat`
- `POST /api/chat_stream`
- `GET /api/incidents`

Every response includes an `x-request-id` header for trace correlation.

## Examples

```bash
curl http://127.0.0.1:3000/health
curl http://127.0.0.1:3000/ready
curl http://127.0.0.1:3000/metrics
curl -X POST http://127.0.0.1:3000/api/chat -H "Content-Type: application/json" -d "{\"Id\":\"session-1\",\"Question\":\"hello\"}"
curl -N -X POST http://127.0.0.1:3000/api/chat_stream -H "Content-Type: application/json" -d "{\"Id\":\"session-1\",\"Question\":\"hello\"}"
curl http://127.0.0.1:3000/api/incidents
```

## Validation

```bash
cargo test
```
