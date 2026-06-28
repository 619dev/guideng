# 归灯

归灯是一个可自建的家人位置共享应用。服务端使用 Rust 编写，位于 `server/`；Web 客户端使用 React + Vite + TypeScript 编写，位于 `client/`。客户端只需要填写服务器地址和共享 Token 即可登录，后续可以把 Web 端封装为 Android 或 iOS 应用。

[English README](README_EN.md)

## 功能特性

- 使用一个服务器 URL 和一个共享 Token 登录。
- 登录页内置隐私规则和使用许可协议，勾选同意后才能登录。
- 通过浏览器 Geolocation API 获取移动设备位置。
- 支持自定义设备名称。
- 支持中文和英文界面。
- 支持百度地图、高德地图、Google Maps、Apple Maps。
- 国内地图展示时自动处理坐标偏移：高德/Apple 使用 GCJ-02，百度使用 BD-09，数据库保留原始 GPS 坐标。
- 使用 SQLite 数据库保存数据。
- 记录每个设备最近 7 天的行动轨迹。
- 提供 Dockerfile、Docker Compose 和 Zeabur 部署模板。
- 提供多架构 Docker 镜像构建和推送脚本。

## 快速开始

```bash
docker compose up -d
```

启动后打开：

```text
http://localhost:3000
```

本地 Docker 默认服务端地址是：

```text
http://localhost:8080
```

如果不设置 Token，服务端会在启动时自动生成一个 128 字符随机 Token，并输出到日志中。你也可以手动设置固定 Token：

```bash
GUIDENG_TOKEN=replace-with-a-long-random-token docker compose up -d
```

## 开发运行

服务端：

```bash
cd server
GUIDENG_TOKEN=dev-token cargo run
```

客户端：

```bash
cd client
npm install
npm run dev
```

## 客户端资源

App logo 位于 `client/public/assets/guideng-logo.png`。在客户端中可通过 `/assets/guideng-logo.png` 引用，后续封装 Android/iOS 时也可以从该路径取用。

## Docker 镜像构建

项目提供了 `build-and-push.sh`，用于构建并推送 `guideng-server` 和 `guideng-client` 镜像。脚本会优先使用 Depot；如果本机没有 Depot，则回退到 Docker Buildx。

```bash
./build-and-push.sh
TAG=v0.1.0 ./build-and-push.sh
PUSH=0 PLATFORM=linux/amd64 ./build-and-push.sh
IMAGES=server ./build-and-push.sh
IMAGES=client VITE_DEFAULT_SERVER_URL=https://guideng.example.com ./build-and-push.sh
```

常用环境变量：

- `REPO`：镜像仓库命名空间，默认 `facilisvelox`。
- `IMAGE_PREFIX`：镜像名前缀，默认 `guideng`。
- `TAG`：镜像标签，默认 `latest`。
- `PUSH`：是否推送镜像，`1` 表示推送，`0` 表示只构建。
- `BUILDER`：构建器，可选 `auto`、`depot`、`buildx`。
- `PLATFORM`：目标平台，默认 `linux/amd64,linux/arm64`。
- `IMAGES`：构建目标，可选 `all`、`server`、`client`。
- `VITE_DEFAULT_SERVER_URL`：客户端构建时内置的默认服务端地址。

## 服务端环境变量

- `GUIDENG_TOKEN`：共享 API Token。不设置或为空时，服务端会自动生成 128 字符随机 Token，并输出到日志中。
- `GUIDENG_BIND`：监听地址，默认 `0.0.0.0:8080`。
- `GUIDENG_DATABASE_URL`：SQLite 数据库文件路径，默认 `/data/guideng.sqlite3`。
- `GUIDENG_LOG_PATH`：日志文件路径。默认写入项目 `server/guideng.log`；Docker Compose 中默认写入 `/data/guideng.log`。
- `GUIDENG_CORS_ORIGINS`：允许跨域访问的来源，多个来源用英文逗号分隔，默认 `*`。

## API

所有 `/api/*` 接口都需要携带 Token，可以使用下面任意一种方式：

```http
Authorization: Bearer <token>
X-Guideng-Token: <token>
```

接口列表：

- `GET /health`：健康检查。
- `GET /api/devices`：获取设备列表和每个设备的最新位置。
- `POST /api/devices`：注册或更新当前设备。
- `PATCH /api/devices/:id`：修改设备名称或平台信息。
- `POST /api/devices/:id/location`：上报设备位置。
- `GET /api/devices/:id/tracks?days=7`：获取设备最近轨迹，最多 7 天。

服务端会保存每次位置上报，并保留最近 7 天的轨迹点。写入新位置时，会自动清理超过 7 天的旧轨迹。

## 部署说明

推荐使用 HTTPS 部署。移动端浏览器在非 `localhost` 环境下通常要求 HTTPS 才能使用高精度定位，否则客户端可能无法获取位置权限。

详细本地部署流程见：[归灯本地部署指南](DEPLOY_LOCAL.md) / [English deployment guide](DEPLOY_LOCAL_EN.md)。

使用 Docker Compose：

```bash
GUIDENG_TOKEN=replace-with-a-long-random-token docker compose up -d
```

默认会拉取 `facilisvelox/guideng-server:latest` 和 `facilisvelox/guideng-client:latest`。如需使用自定义镜像：

```bash
GUIDENG_SERVER_IMAGE=yourname/guideng-server:v0.1.0 \
GUIDENG_CLIENT_IMAGE=yourname/guideng-client:v0.1.0 \
docker compose up -d
```

项目也包含 `zeabur.yaml`，可以作为 Zeabur 部署模板使用。部署时可以手动设置 `GUIDENG_TOKEN`；如果留空，服务端会自动生成并写入日志。

## 本地 Nginx 反代

项目根目录提供了 `nginx.conf`，用于本地先通过 Docker Compose 启动服务端，再用宿主机 Nginx 反代服务端端口。

```bash
docker compose up -d
sudo cp nginx.conf /etc/nginx/nginx.conf
sudo nginx -t
sudo systemctl reload nginx
```

默认反代目标是：

```text
127.0.0.1:8080
```

配置中只反代服务端接口：

- `/health`
- `/api/`

## 数据存储

归灯默认使用 SQLite。Docker 部署时数据库位于容器内 `/data/guideng.sqlite3`，并通过 Compose volume 持久化。

如果你要备份数据，备份 `/data` 对应的 volume 或 SQLite 文件即可。

## 注意事项

- 这个项目使用一个共享 Token，适合家庭或小范围自建使用。
- 如果没有手动设置 `GUIDENG_TOKEN`，请从服务端日志中复制启动时生成的 Token。
- 如果客户端和服务端部署在不同域名下，请根据实际域名设置 `GUIDENG_CORS_ORIGINS`。
- 地图服务由各地图供应商提供，部分供应商可能在不同地区有访问限制或坐标系差异。

## 许可证

MIT
