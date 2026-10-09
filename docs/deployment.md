# 本地测试与线上部署

## 架构与前提

LIYU-MINI → 标准 `net.http_request` / WebReader → HTTPS Caddy → LIYU 服务端 → PostgreSQL。无需 LIYU 专用宿主，也无需克隆原生 LIYU。AI 使用设备提供的标准模型能力；本地演示无需任何后端。

需要 Docker Engine/Desktop 和 Docker Compose v2 或更新版本。执行下列命令的位置均为本仓库根目录。本地 Compose 包含数据库、服务端、Caddy；正式 compose.deploy.yaml 只包含数据库和服务端，镜像包含编译好的管理后台、迁移与商品示例图片。数据库和上传媒体分别保存到命名卷，服务端以非 root 用户运行，数据库及服务端端口不对外发布。

**当前支付、物流与折现是测试业务，余额不是可提现资金。部署到公网不会自动获得真实支付、短信或物流能力。**真实邮箱/手机号验证必须配置发送服务，协议见[联系方式发送说明](contact-delivery.md)。不要将本地测试环境暴露到公网。

## 1. 本地 Compose：一条命令启动

首次配置：

```sh
cp deploy/local.env.example deploy/local.env
```

以后启动、更新代码后的构建：

```sh
docker compose --env-file deploy/local.env -f compose.yaml -f compose.local.yaml up -d --build --wait
```

本地配置使用独立数据库用户，开发模式允许测试验证码，Caddy 仅监听回环地址：HTTPS `https://liyu.localhost:8443`，HTTP 8080。`liyu.localhost` 必须解析到 `127.0.0.1`；若系统不支持 `.localhost`，在 hosts 文件添加此记录。端口已被原来的本地服务占用时，先停止原服务，或修改 `LIYU_HTTPS_BIND` 和 `LIYU_PUBLIC_URL`，同时更新小程序的服务地址。

查看状态与日志：

```sh
docker compose --env-file deploy/local.env -f compose.yaml -f compose.local.yaml ps
docker compose --env-file deploy/local.env -f compose.yaml -f compose.local.yaml logs --tail=100 server caddy
```

### 信任开发证书

Caddy 使用本地 CA，不会自动安装到宿主系统。导出根证书：

```sh
mkdir -p build
docker compose --env-file deploy/local.env -f compose.yaml -f compose.local.yaml cp caddy:/data/caddy/pki/authorities/local/root.crt build/liyu-local-root.crt
```

macOS 在确认这是自己启动的 Caddy 后，将根证书加入**当前用户**登录钥匙串：

```sh
security add-trusted-cert -r trustRoot -k "$HOME/Library/Keychains/login.keychain-db" build/liyu-local-root.crt
```

其他系统使用其证书信任管理器；浏览器若使用独立证书库，还需要导入该证书。不要用 `-k` 或关闭 TLS 验证作为应用运行方案。无须修改系统钥匙串。重建 Caddy 数据卷会换 CA，需要重新导入。

```sh
curl --cacert build/liyu-local-root.crt https://liyu.localhost:8443/health
```

健康检查返回成功后，浏览器打开 `https://liyu.localhost:8443/admin`。本地管理员账号见 `deploy/local.env` 的 `LIYU_ADMIN_USERNAME`、`LIYU_ADMIN_PASSWORD`。小程序使用后端网页登录，不输入密码到 Splash。

### 开发账号、验证码与测试

仅本地模式允许迁移中的 `demo@liyu.test`、`linzhou@liyu.test`、`chenxiao@liyu.test`，密码 `123456`。注册先申请验证码，测试响应才包含 `test_code`。`LIYU_ENV` 只接受 `development` 或 `production`，拼写错误会在连接数据库前拒绝启动。生产模式要求安全管理 Cookie，禁止测试验证码，且禁用仍使用迁移测试密码哈希的账号并撤销其会话。

可执行下面的集成测试；执行环境需要 Rust、Python 3 与 `psql`，数据库连接用户需要创建测试数据库的权限。运行器为每组测试创建独立数据库并在结束后删除，另起回环 HTTP 测试后端，不修改原数据库中的业务数据。将连接字符串替换为自己的本地数据库：

```sh
cargo build --locked
DATABASE_URL='postgres://root:root@127.0.0.1:5432/liyu_dev' python3 tests/run_ci.py
```

默认测试监听 `127.0.0.1:18788`，已被占用时设置 `LIYU_CI_PORT`。不能使用不受自己控制的数据库。Compose 内的数据库不向宿主发布端口；这组测试使用源码开发的本机 PostgreSQL，GitHub CI 则使用一次性的 PostgreSQL 服务。Compose 的实际 HTTPS、后台及图片可通过上一节验证。

停止容器（保留数据）：

```sh
docker compose --env-file deploy/local.env -f compose.yaml -f compose.local.yaml down
```

`down -v` 会删除数据库、媒体和 Caddy CA，只应在确认清空本地测试数据时使用。

## 2. 本地源码开发

安装 Rust、PostgreSQL 客户端与数据库、Python 3、just。复制 `.env.example` 为 `.env`，设置自己的 `DATABASE_URL`；`just dev` 的自动管理仅支持本地 `liyu_dev` 数据库，不会初始化数据库集群。其他数据库应自行创建后执行 `cargo run --locked`。

管理后台需要 `wasm32-unknown-unknown` 与 Dioxus CLI 0.7.10：

```sh
rustup target add wasm32-unknown-unknown
cargo install dioxus-cli --version 0.7.10 --locked
just build-admin
just dev
```

服务端在 `127.0.0.1:8787` 监听。Caddy 最小配置：

```caddyfile
{
    skip_install_trust
}
https://liyu.localhost:8443 {
    tls internal
    reverse_proxy 127.0.0.1:8787
}
```

保存后 `caddy run --config <配置路径> --adapter caddyfile`。根证书位置由 Caddy 的数据目录决定，按上一节信任证书。小程序仓库还提供 `./start-local-services.sh`，可同时启动源码后端与 Caddy，要求两个仓库同级；它是开发便利脚本，线上使用 Compose。

## 3. GitHub CI 与镜像

[工作流](../.github/workflows/ci.yml) 在 PR、main 推送、`v*` 标签和手动运行时执行：Rust 格式、Clippy、编译、单元测试，以及真实数据库上的网页授权、心愿单和双方约定状态测试。测试通过后构建 amd64/arm64 容器；PR 只构建，main/标签使用 `GITHUB_TOKEN` 发布到 GHCR，无需配置仓库密码。需在仓库 Actions 设置允许执行工作流及写入 Packages。版本 Release 的创建还需 contents 写权限。

本仓库预期镜像地址是 `ghcr.io/chrislearn/liyu-server`：

- main：`ghcr.io/chrislearn/liyu-server:main`。
- `v1.2.3` 标签：`ghcr.io/chrislearn/liyu-server:v1.2.3`。
- 提交：`ghcr.io/chrislearn/liyu-server:sha-<完整提交 SHA>`。

**这些是发布规则，不表示镜像已经发布。**首次推送后先在 Actions 确认整个工作流成功，再在 Packages 查看实际标签与摘要。将该容器包设为 Public，其他人才能匿名拉取；私有包需用具备 `read:packages` 权限的个人令牌登录 GHCR。不要在 README 或 Compose 文件里保存令牌。仓库 fork 后地址随仓库所有者变化。

上线时优先固定已验证的标签或 `@sha256:...` 摘要。没有已发布镜像时可以本地构建：

```sh
docker build -t liyu-server:local .
```

然后将配置中的 `LIYU_IMAGE` 改成 `liyu-server:local`，跳过 `pull server`。Dockerfile 会编译后台，不依赖本地已有的 `web/dist` 或 `target`。

## 4. 公网部署：由已有 Caddy 反代

实际域名为 **liyu.taidge.com**，公网 HTTPS 和证书由运营者自己的 Caddy 配置。正式部署使用根目录的 **compose.deploy.yaml**，其中只有 PostgreSQL 和服务端，无 Caddy 容器、证书卷或 80/443 端口。服务端仅绑定宿主机 `127.0.0.1:8787`，数据库不发布端口。

```sh
cp deploy/production.env.example deploy/production.env
chmod 600 deploy/production.env
```

修改 POSTGRES_PASSWORD 和 LIYU_ADMIN_PASSWORD（建议 `openssl rand -hex 32` 生成），设置自己的管理员用户名；真实注册需要 LIYU_DELIVERY_WEBHOOK 与 LIYU_DELIVERY_TOKEN，协议见[联系方式发送说明](contact-delivery.md)。LIYU_IMAGE 默认 `ghcr.io/chrislearn/liyu-server:v0.1.2`，必须等该版本 CI 成功发布后拉取，也可替换为已验证的镜像固定摘要。默认 LIYU_PUBLIC_URL 为 `https://liyu.taidge.com`。

```sh
docker compose --env-file deploy/production.env -f compose.deploy.yaml config --quiet
docker compose --env-file deploy/production.env -f compose.deploy.yaml pull
docker compose --env-file deploy/production.env -f compose.deploy.yaml up -d --wait
curl --fail http://127.0.0.1:8787/health
```

已有 Caddy 对 `liyu.taidge.com` 反代到 **127.0.0.1:8787**，包括 `/api/v1`、`/authorize`、`/admin` 和商品图片路径，不能只代理 API。配置完成后验证 `curl --fail https://liyu.taidge.com/health`，再打开 `/admin` 和小程序网页登录，核验真实验证码送达、注册与会话撤销。不要在反代访问日志中记录 Authorization 头、请求体、验证码或会话令牌。

上述回环地址适用于 **Caddy 运行在宿主机**。如果你的 Caddy 本身在容器中，容器的 127.0.0.1 不指向宿主机；应把 Caddy 接入同一 Docker 网络并使用 `server:8787`，按你的现有代理环境配置，不要将数据库或明文后端端口直接暴露公网。

Compose 强制 production 模式、关闭旧版 LIYU_TEST_DELIVERY 开发开关、启用安全管理 Cookie；新 LIYU_TEST_MODE 默认 false，固定验证码体验需显式开启。公网部署仍不包含真实支付或物流集成。更改已初始化数据库的 POSTGRES_PASSWORD 不会自动修改数据库用户密码，需协调数据库和服务端更新。环境文件、真实媒体、数据卷和备份不得提交 Git。

小程序 1.0.36 默认域名与本节一致，无需额外 localhost 配置。若更换域名，必须从小程序可编辑源码同步修改服务地址和 network.hosts，然后发布新版本；不能修改已封装的 GitHub 发布包。

## 5. 数据持久化、升级与排障

命名卷保存 PostgreSQL、上传媒体和 Caddy 证书。每次升级前备份数据库及媒体并验证恢复；不要把数据库密码写入命令参数或公开日志。

```sh
mkdir -p backups
docker compose --env-file deploy/production.env -f compose.deploy.yaml exec -T db pg_dump -U liyu -d liyu -Fc > backups/liyu.dump
docker compose --env-file deploy/production.env -f compose.deploy.yaml cp server:/data backups/media
```

恢复应先停写入业务，在独立恢复环境创建空数据库，使用 `pg_restore -U liyu -d liyu` 导入，将媒体恢复到 `/data` 并保持 UID 10001 可写，再启动服务验证。不要覆盖唯一的线上卷。备份包含个人资料，限制访问并安全保存。

升级：修改 `LIYU_IMAGE` 为新版本，`docker compose --env-file deploy/production.env -f compose.deploy.yaml pull server`，再 `docker compose --env-file deploy/production.env -f compose.deploy.yaml up -d --wait`。迁移可能使旧程序不兼容；回退程序前检查迁移，必要时使用升级前的数据库与媒体备份。无需删除卷。

- `server` 不健康：查看服务日志与 DB 健康状态，检查密码、迁移及生产保护设置。
- 管理后台 503：源码方式未构建后台；容器内应包含 `/app/web/dist`，确认拉取的是此 Dockerfile 构建的镜像。
- HTTPS 错误：本地核对根证书及域名；线上核对 DNS、80/443、Caddy 日志与持久化证书卷。
- 小程序请求被拒：核对服务地址、`network.hosts`、系统证书信任和服务器健康；网络失败不会切换成演示用户。
- 收不到验证码：确认关闭测试模式且发送桥正确，避免把测试验证码作为真实验证方式。

参考：[GitHub 镜像发布](https://docs.github.com/en/actions/tutorials/publish-packages/publish-docker-images)、[GHCR 访问与可见性](https://docs.github.com/en/packages/working-with-a-github-packages-registry/working-with-the-container-registry)、[Caddy 自动 HTTPS](https://caddyserver.com/docs/automatic-https)。

## 发布与证据对应

API 测试完成后 CI 上传 `api-tests.json`，记录测试源码提交、工作目录是否干净和实际通过的测试组。容器发布使用同一提交的 `sha-<完整 SHA>` 标签；线上仍应固定 Packages 提供的镜像摘要，不能把浮动 `main` 标签作为长期复现标识。

GitHub Actions 已固定到完整提交 SHA，Cargo 依赖使用锁文件；基础容器标签仍会更新，因此这是可追溯构建，不是字节级可重复构建保证。数据库、Caddy及服务端镜像可进一步按已验证摘要固定。GHCR 首次包可能为私有，确认可见性后再承诺其他人能匿名拉取。

公开发行或平台收录、发布者签名和苹果公证是不同步骤。此仓库的 Docker CI 通过不证明小程序跨平台界面或真实支付通过。服务端隐私和删除边界见根目录 `PRIVACY.md`。

## 6. 版本发布与部署包

[GitHub Releases](https://github.com/chrislearn/liyu-server/releases) 提供对应版本的源码与部署配置 ZIP、测试报告、镜像标签及固定摘要。镜像仍从 GHCR 拉取，ZIP 不是容器镜像。

先更新 Cargo.toml 的 package.version 和 Cargo.lock，提交后推送一致的版本标签，如 `git tag v0.1.0 && git push origin v0.1.0`。标签 CI 校验版本、运行测试、发布双架构镜像，然后自动创建 Release；任一前置任务失败均不发布 Release。main 推送继续发布 main 镜像，不创建版本 Release。

下载 ZIP 后解压，按照本指南配置环境。线上将 LIYU_IMAGE 设置为发布页提供的版本标签或固定摘要，再执行 Compose pull 与 up。源码 ZIP 也包含本地构建所需文件。不要移动已发布的版本标签；升级应使用新的版本号。

## 无发送服务时的固定验证码体验

设置 `LIYU_TEST_MODE=true`：当 `LIYU_DELIVERY_WEBHOOK` 或 `LIYU_DELIVERY_TOKEN` 任一未设置、为空或只有空白时，验证码固定为 `123456`，不入邮件/短信验证码投递队列，投递 worker 暂停外部发送。授权网页明确显示测试验证码；仍需先申请 challenge，十分钟有效期、错误尝试次数及一次性使用规则不变。注册和邮箱/手机号修改使用同一规则。

默认 `LIYU_TEST_MODE=false`，不会因缺少供应商而自动接受固定码。开关为 true 但两个发送配置均完整时，继续使用随机验证码和真实发送。该开关可用于 production 部署下的体验，独立于只供开发环境使用的 LIYU_TEST_DELIVERY；HTTPS、安全 Cookie 和生产账号保护继续有效。固定码体验不能证明用户实际持有邮箱或手机号，体验期间不要使用真实业务数据；正式验证需关闭此开关并配置发送服务。

## 已上线地址与常见配置问题

当前线上入口为 **https://liyu.taidge.com**；业务 API 在 `/api/v1`，管理后台在 `/admin`。2026-10-09 从公网检查 `/health` 返回 HTTP 200 和 `{"status":"ok"}`，这是健康检查证据，不代表真实发送服务或全部业务验收。

环境文件可放在 `deploy/production.env`，也可像实际服务器一样放在根目录 `./production.env`。后者启动命令为：

```sh
docker compose --env-file ./production.env -f compose.deploy.yaml pull
docker compose --env-file ./production.env -f compose.deploy.yaml up -d --wait
```

使用固定验证码体验时，在所选环境文件中填写：

```dotenv
LIYU_TEST_MODE=true
LIYU_DELIVERY_WEBHOOK=
LIYU_DELIVERY_TOKEN=
```

**--env-file 不会自动将全部变量传入容器。** compose.deploy.yaml 的 server.environment 必须包含 `LIYU_TEST_MODE: ${LIYU_TEST_MODE:-false}`；旧版 Compose 文件需同步更新。修改后重建容器，单纯 restart 不更新环境变量：

```sh
docker compose --env-file ./production.env -f compose.deploy.yaml up -d --force-recreate --wait server
docker compose --env-file ./production.env -f compose.deploy.yaml exec server printenv LIYU_TEST_MODE
```

最后一条应输出 true。镜像 v0.1.2 已包含该功能；拉取新镜像仍需通过 up 更新运行中的容器。环境文件和 Compose 文件都使用显式路径，避免在没有默认 compose.yaml 的目录执行 docker compose pull 时出现找不到配置的错误。
