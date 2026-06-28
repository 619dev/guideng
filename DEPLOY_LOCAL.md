# 归灯本地部署指南

这份文档说明如何在一台自己的服务器上部署归灯：后端用 Docker Compose 启动，宿主机安装 Nginx 反代服务端接口，并为域名申请 HTTPS 证书。

推荐结构：

- `server`：由 Docker Compose 启动，监听宿主机 `8080` 端口。
- `client`：本地部署时不通过 Docker Compose 启动；可以后续单独部署为静态站点、移动端壳应用，或放到其他 Web 服务中。
- `nginx`：安装在宿主机上，负责反代 `/api/` 和 `/health` 到 `127.0.0.1:8080`。
- `HTTPS`：使用 Certbot 申请证书。

[English deployment guide](DEPLOY_LOCAL_EN.md)

## 1. 准备服务器

服务器需要安装：

- Docker
- Docker Compose
- Nginx
- Certbot

以 Debian/Ubuntu 为例：

```bash
sudo apt update
sudo apt install -y nginx certbot python3-certbot-nginx
```

Docker 可以参考 Docker 官方文档安装。安装完成后确认命令可用：

```bash
docker --version
docker compose version
nginx -v
certbot --version
```

## 2. 准备项目目录

把项目放到服务器上，例如：

```bash
cd /opt
sudo git clone <your-repo-url> guideng
cd /opt/guideng
```

如果不是通过 Git 部署，也可以把整个项目目录上传到服务器。

## 3. 修改 Docker Compose

本地部署时，请先删除 `docker-compose.yml` 中的 `client` 部分，只保留服务端。

需要删除的部分大致如下：

```yaml
  client:
    image: facilisvelox/guideng-client:latest
    pull_policy: always
    ports:
      - "3000:80"
    depends_on:
      - server
```

删除后，`docker-compose.yml` 应类似：

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

## 4. 设置 Token

推荐手动设置一个固定 Token：

```bash
export GUIDENG_TOKEN='replace-with-a-long-random-token'
```

如果不设置 `GUIDENG_TOKEN`，服务端会在启动时自动生成一个 128 字符随机 Token，并写入日志。日志默认位于 Docker 数据卷中的 `/data/guideng.log`。

启动后可以查看日志：

```bash
docker compose logs server
```

也可以进入容器查看 `/data/guideng.log`。

## 5. 启动服务端

```bash
docker compose up -d
```

检查容器状态：

```bash
docker compose ps
```

检查服务端健康状态：

```bash
curl http://127.0.0.1:8080/health
```

正常情况下会返回：

```json
{"name":"guideng","ok":true}
```

## 6. 配置本地 Nginx 反代

项目根目录已经提供了 `nginx.conf`。如果你希望直接使用它作为主配置：

```bash
sudo cp nginx.conf /etc/nginx/nginx.conf
sudo nginx -t
sudo systemctl reload nginx
```

更推荐的方式是只新增一个站点配置，避免覆盖系统原有 Nginx 主配置。

创建站点文件：

```bash
sudo nano /etc/nginx/sites-available/guideng.conf
```

填入下面内容，把 `example.com` 替换成你的域名：

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

启用站点：

```bash
sudo ln -s /etc/nginx/sites-available/guideng.conf /etc/nginx/sites-enabled/guideng.conf
sudo nginx -t
sudo systemctl reload nginx
```

测试反代：

```bash
curl http://example.com/health
```

## 7. 解析域名

到你的域名服务商后台添加 DNS 解析：

- 记录类型：`A`
- 主机记录：例如 `guideng` 或 `@`
- 记录值：你的服务器公网 IPv4 地址

如果服务器有 IPv6，也可以添加：

- 记录类型：`AAAA`
- 记录值：你的服务器公网 IPv6 地址

等待 DNS 生效后检查：

```bash
ping example.com
```

或：

```bash
dig example.com
```

## 8. 申请 HTTPS 证书

移动端浏览器通常要求 HTTPS 才能获取定位权限，所以正式使用建议配置证书。

使用 Certbot 自动申请并修改 Nginx 配置：

```bash
sudo certbot --nginx -d example.com
```

如果你使用的是子域名：

```bash
sudo certbot --nginx -d guideng.example.com
```

按提示输入邮箱、同意服务条款，并选择是否将 HTTP 自动重定向到 HTTPS。推荐开启重定向。

申请完成后测试：

```bash
curl https://example.com/health
```

检查证书自动续期：

```bash
sudo certbot renew --dry-run
```

## 9. 配置客户端连接

客户端登录时填写：

- 服务器网址：`https://example.com`
- Token：你设置的 `GUIDENG_TOKEN`，或服务端自动生成并写入日志的 Token

登录页只需要填写服务器网址和 Token，并勾选隐私规则与使用许可协议。

如果 Web 客户端部署在其他域名，建议把 `GUIDENG_CORS_ORIGINS` 设置为客户端域名，例如：

```bash
GUIDENG_CORS_ORIGINS=https://app.example.com docker compose up -d
```

如果有多个客户端来源，用英文逗号分隔：

```bash
GUIDENG_CORS_ORIGINS=https://app.example.com,https://www.example.com docker compose up -d
```

## 10. 常用维护命令

查看日志：

```bash
docker compose logs -f server
```

重启服务端：

```bash
docker compose restart server
```

更新镜像：

```bash
docker compose pull server
docker compose up -d
```

备份数据：

```bash
docker run --rm -v guideng_guideng-data:/data -v "$PWD":/backup alpine tar czf /backup/guideng-data.tar.gz /data
```

恢复数据前请先停止服务：

```bash
docker compose down
```

## 11. 安全提示

- 不要使用空 Token 作为长期配置；如果留空，请从日志中复制自动生成的 Token。
- 不要把 Token 发给不需要访问位置数据的人。
- 建议只开放 Nginx 的 `80` 和 `443` 端口，对外不要直接开放 `8080`。
- 建议定期备份 SQLite 数据库。
- 如果设备无法获取定位权限，优先检查是否使用 HTTPS。
