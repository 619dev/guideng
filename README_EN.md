# Guideng

Guideng is a self-hosted family location sharing app. It has a Rust server in `server/` and a React + Vite + TypeScript web client in `client/`.

## Features

- One server URL and one shared token for login.
- Built-in privacy rules and license agreement on the login page; users must accept them before login.
- Mobile browser location sharing through the Geolocation API.
- Custom device names.
- Chinese and English UI.
- AMap support.
- Automatic coordinate conversion for China map providers: AMap uses GCJ-02, while the database keeps raw GPS coordinates.
- SQLite database storage with one week of location history per device.
- Docker, Docker Compose, and Zeabur deployment templates.

## Quick Start

```bash
docker compose up -d
```

Then open `http://localhost:3000`.

Default server URL in local Docker is `http://localhost:8080`.

If no token is provided, the server generates a 128-character random token on startup and writes it to the log. You can also set a fixed token manually:

```bash
GUIDENG_TOKEN=replace-with-a-long-random-token docker compose up -d
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

## Client Assets

The app logo is stored at `client/public/assets/guideng-logo.png`. In the client, reference it as `/assets/guideng-logo.png`; Android/iOS wrappers can reuse the same asset.

## Build Docker Images

```bash
./build-and-push.sh
TAG=v0.1.0 ./build-and-push.sh
PUSH=0 PLATFORM=linux/amd64 ./build-and-push.sh
IMAGES=server ./build-and-push.sh
IMAGES=client VITE_DEFAULT_SERVER_URL=https://guideng.example.com ./build-and-push.sh
```

The script builds `guideng-server` and `guideng-client` images. It automatically prefers Depot when available and falls back to Docker Buildx.

## Server Environment

- `GUIDENG_TOKEN`: shared API token. When unset or empty, the server generates a 128-character random token and writes it to the log.
- `GUIDENG_BIND`: bind address, default `0.0.0.0:8080`.
- `GUIDENG_DATABASE_URL`: SQLite database file, default `/data/guideng.sqlite3`.
- `GUIDENG_LOG_PATH`: log file path. By default it writes to `server/guideng.log`; Docker Compose sets it to `/data/guideng.log`.
- `GUIDENG_CORS_ORIGINS`: comma-separated allowed origins, default `*`.
- `GUIDENG_AMAP_WEB_JS_API_KEY`: AMap Web JavaScript API key.
- `GUIDENG_AMAP_WEB_JS_SECURITY_CODE`: AMap Web JavaScript API security code.
- `GUIDENG_AMAP_ANDROID_KEY`: AMap Android SDK key for future Android app builds.
- `GUIDENG_AMAP_IOS_KEY`: AMap iOS SDK key for future iOS app builds.

## API

All `/api/*` endpoints require one of:

- `Authorization: Bearer <token>`
- `X-Guideng-Token: <token>`

Endpoints:

- `GET /health`
- `GET /api/config`: get map provider and AMap key configuration.
- `GET /api/devices`
- `POST /api/devices`
- `PATCH /api/devices/:id`
- `POST /api/devices/:id/location`
- `GET /api/devices/:id/tracks?days=7`

The server stores every location report and keeps the most recent 7 days of history. Older points are pruned when new locations are written.

## Notes

Mobile browsers usually require HTTPS for high-accuracy geolocation outside `localhost`. When deploying, put both client and server behind HTTPS.

For detailed local deployment, see [Local Deployment Guide](DEPLOY_LOCAL_EN.md) / [中文部署指南](DEPLOY_LOCAL.md).

## Local Nginx Reverse Proxy

The root `nginx.conf` is intended for running Docker Compose locally first, then using host Nginx to reverse proxy the server port.

```bash
docker compose up -d
sudo cp nginx.conf /etc/nginx/nginx.conf
sudo nginx -t
sudo systemctl reload nginx
```

It proxies `/health` and `/api/` to `127.0.0.1:8080`.

## License

MIT
