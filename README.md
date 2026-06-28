# 归灯 Guideng

归灯 is a self-hosted family location sharing app. It has a Rust server in `server/` and a React + Vite + TypeScript web client in `client/`.

## Features

- One server URL and one shared token for login.
- Mobile browser location sharing through the Geolocation API.
- Custom device names.
- Chinese and English UI.
- Map provider switcher for Baidu Maps, AMap, Google Maps, and Apple Maps.
- SQLite database storage with one week of location history per device.
- Docker, Docker Compose, and Zeabur deployment templates.

## Quick Start

```bash
docker compose up --build
```

Then open `http://localhost:3000`.

Default server URL in local Docker is `http://localhost:8080`.

Set a real shared token before using it:

```bash
GUIDENG_TOKEN=replace-with-a-long-random-token docker compose up --build
```

## Development

Server:

```bash
cd server
GUIDENG_TOKEN=dev-token cargo run
```

Client:

```bash
cd client
npm install
npm run dev
```

## Server Environment

- `GUIDENG_TOKEN`: required shared API token.
- `GUIDENG_BIND`: bind address, default `0.0.0.0:8080`.
- `GUIDENG_DATABASE_URL`: SQLite database file, default `/data/guideng.sqlite3`.
- `GUIDENG_CORS_ORIGINS`: comma-separated allowed origins, default `*`.

## API

All `/api/*` endpoints require one of:

- `Authorization: Bearer <token>`
- `X-Guideng-Token: <token>`

Endpoints:

- `GET /health`
- `GET /api/devices`
- `POST /api/devices`
- `PATCH /api/devices/:id`
- `POST /api/devices/:id/location`
- `GET /api/devices/:id/tracks?days=7`

The server stores every location report and keeps the most recent 7 days of history. Older points are pruned when new locations are written.

## Notes

Mobile browsers usually require HTTPS for high-accuracy geolocation outside `localhost`. When deploying, put both client and server behind HTTPS.

## License

MIT
