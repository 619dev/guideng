# Guideng Local Deployment Guide

This guide explains how to deploy Guideng on your own server: run the backend with Docker Compose, install Nginx on the host as a reverse proxy, and issue an HTTPS certificate for your domain.

Recommended layout:

- `server`: started by Docker Compose and exposed on host port `8080`.
- `client`: not started by Docker Compose for this local deployment flow. You can deploy it separately as a static site, package it into a mobile app shell, or host it through another web service.
- `nginx`: installed on the host and reverse proxies `/api/` and `/health` to `127.0.0.1:8080`.
- `HTTPS`: issued with Certbot.

[中文部署指南](DEPLOY_LOCAL.md)

## 1. Prepare the Server

Install:

- Docker
- Docker Compose
- Nginx
- Certbot

On Debian/Ubuntu:

```bash
sudo apt update
sudo apt install -y nginx certbot python3-certbot-nginx
```

Install Docker from the official Docker documentation. After installation, verify the commands:

```bash
docker --version
docker compose version
nginx -v
certbot --version
```

## 2. Prepare the Project Directory

Place the project on the server, for example:

```bash
cd /opt
sudo git clone <your-repo-url> guideng
cd /opt/guideng
```

If you are not using Git, upload the whole project directory to the server.

## 3. Edit Docker Compose

For this local deployment flow, delete the `client` section from `docker-compose.yml` and keep only the backend service.

Remove a section similar to this:

```yaml
  client:
    image: facilisvelox/guideng-client:latest
    pull_policy: always
    ports:
      - "3000:80"
    depends_on:
      - server
```

After removal, `docker-compose.yml` should look similar to this:

```yaml
services:
  server:
    image: facilisvelox/guideng-server:latest
    pull_policy: always
    environment:
      GUIDENG_TOKEN: ${GUIDENG_TOKEN:-}
      GUIDENG_BIND: 0.0.0.0:8080
      GUIDENG_DATABASE_URL: /data/guideng.sqlite3
      GUIDENG_LOG_PATH: /data/guideng.log
      GUIDENG_CORS_ORIGINS: ${GUIDENG_CORS_ORIGINS:-*}
    volumes:
      - guideng-data:/data
    ports:
      - "8080:8080"

volumes:
  guideng-data:
```

## 4. Set the Token

It is recommended to set a fixed token manually:

```bash
export GUIDENG_TOKEN='replace-with-a-long-random-token'
```

If `GUIDENG_TOKEN` is not set, the server generates a 128-character random token on startup and writes it to the log. In Docker, the log path defaults to `/data/guideng.log` inside the data volume.

View startup logs:

```bash
docker compose logs server
```

You can also enter the container and inspect `/data/guideng.log`.

## 5. Start the Backend

```bash
docker compose up -d
```

Check container status:

```bash
docker compose ps
```

Check the backend health endpoint:

```bash
curl http://127.0.0.1:8080/health
```

Expected response:

```json
{"name":"guideng","ok":true}
```

## 6. Configure Host Nginx Reverse Proxy

The project root includes `nginx.conf`. If you want to use it as the main Nginx configuration:

```bash
sudo cp nginx.conf /etc/nginx/nginx.conf
sudo nginx -t
sudo systemctl reload nginx
```

The recommended approach is to create a site file instead, so you do not overwrite the system Nginx main config.

Create a site file:

```bash
sudo nano /etc/nginx/sites-available/guideng.conf
```

Paste the following config and replace `example.com` with your domain:

```nginx
server {
  listen 80;
  server_name example.com;

  client_max_body_size 2m;

  location /health {
    proxy_pass http://127.0.0.1:8080;
    proxy_http_version 1.1;
    proxy_set_header Host $host;
    proxy_set_header X-Real-IP $remote_addr;
    proxy_set_header X-Forwarded-For $proxy_add_x_forwarded_for;
    proxy_set_header X-Forwarded-Proto $scheme;
  }

  location /api/ {
    proxy_pass http://127.0.0.1:8080;
    proxy_http_version 1.1;
    proxy_set_header Host $host;
    proxy_set_header X-Real-IP $remote_addr;
    proxy_set_header X-Forwarded-For $proxy_add_x_forwarded_for;
    proxy_set_header X-Forwarded-Proto $scheme;
  }
}
```

Enable the site:

```bash
sudo ln -s /etc/nginx/sites-available/guideng.conf /etc/nginx/sites-enabled/guideng.conf
sudo nginx -t
sudo systemctl reload nginx
```

Test the reverse proxy:

```bash
curl http://example.com/health
```

## 7. Configure DNS

In your domain provider dashboard, add a DNS record:

- Type: `A`
- Host: for example `guideng` or `@`
- Value: your server public IPv4 address

If your server has IPv6, you can also add:

- Type: `AAAA`
- Value: your server public IPv6 address

After DNS propagation, verify:

```bash
ping example.com
```

Or:

```bash
dig example.com
```

## 8. Issue an HTTPS Certificate

Mobile browsers usually require HTTPS to grant location permission, so HTTPS is recommended for production use.

Use Certbot to issue a certificate and update Nginx automatically:

```bash
sudo certbot --nginx -d example.com
```

For a subdomain:

```bash
sudo certbot --nginx -d guideng.example.com
```

Follow the prompts, enter your email, accept the terms, and choose whether to redirect HTTP to HTTPS. Enabling redirect is recommended.

After issuance, test:

```bash
curl https://example.com/health
```

Check automatic renewal:

```bash
sudo certbot renew --dry-run
```

## 9. Configure the Client

When logging in from the client, use:

- Server URL: `https://example.com`
- Token: your `GUIDENG_TOKEN`, or the token generated by the server and written to the log

The login page only asks for the server URL and token, and requires accepting the privacy rules and license agreement.

If the web client is deployed on another domain, set `GUIDENG_CORS_ORIGINS` to the client origin:

```bash
GUIDENG_CORS_ORIGINS=https://app.example.com docker compose up -d
```

For multiple origins, separate them with commas:

```bash
GUIDENG_CORS_ORIGINS=https://app.example.com,https://www.example.com docker compose up -d
```

## 10. Common Maintenance Commands

View logs:

```bash
docker compose logs -f server
```

Restart the backend:

```bash
docker compose restart server
```

Pull and update:

```bash
docker compose pull server
docker compose up -d
```

Back up data:

```bash
docker run --rm -v guideng_guideng-data:/data -v "$PWD":/backup alpine tar czf /backup/guideng-data.tar.gz /data
```

Stop the service before restoring data:

```bash
docker compose down
```

## 11. Security Notes

- Do not use an empty token as a long-term setup. If you leave it empty, copy the generated token from the server log.
- Do not share the token with anyone who should not access location data.
- Expose only Nginx ports `80` and `443` publicly. Avoid exposing `8080` directly.
- Back up the SQLite database regularly.
- If a device cannot get location permission, check whether you are using HTTPS first.
