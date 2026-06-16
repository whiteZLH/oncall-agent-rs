# oncall-agent-rs

Minimal Rust service scaffold with `axum`.

## Run

```bash
cargo run
```

Override bind address if needed:

```bash
APP_HOST=0.0.0.0 APP_PORT=3000 cargo run
```

## Endpoints

- `GET /health`
- `POST /api/chat`
- `GET /api/incidents`

## Examples

```bash
curl http://127.0.0.1:3000/health
curl -X POST http://127.0.0.1:3000/api/chat -H "Content-Type: application/json" -d "{\"message\":\"hello\"}"
curl http://127.0.0.1:3000/api/incidents
```
