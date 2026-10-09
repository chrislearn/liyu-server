# 旧版网页授权与联系方式验证

此协议保留给旧版客户端及联系方式验证。LIYU-MINI 1.0.38 登录已改为[宿主 PKCE](host-pkce.md)，不会轮询领取应用令牌。

1. 应用向 `POST /api/v1/browser-authorizations` 提交 `{"purpose":"login"}`。修改联系方式使用 `email` 或 `phone`，并携带当前应用的 Bearer 会话。
2. 返回 `id`、`poll_key`、`browser_path`、`interval: 3` 和 `expires_in_seconds: 300`。应用在 WebReader 打开后端 `browser_path`；网页密钥在 URL fragment 中，页面立即从地址栏移除它，不进入服务器访问日志。应用保留独立的 `poll_key`，网页不持有它。
3. 网页通过 `/info` 验证网页密钥，再通过 `/login` 输入账号密码。`?mode=register` 支持注册；验证码使用现有 `/api/v1/auth/challenges`。网页会话仅十分钟有效，每次授权最多尝试登录十次。
4. 登录后用户明确点击「确认授权」。联系方式修改必须使用创建授权时的同一个账号，调用现有联系方式验证接口后确认完成。`POST .../{id}/approve` 验证网页密钥和 Bearer 身份，事务中更新授权状态并立即撤销网页会话。
5. 应用每三秒 `POST .../{id}/poll`，正文为 `{"poll_key":"..."}`。未完成返回 `pending`；完成后事务中领取一次结果。登录返回 `complete` 和独立的应用令牌；联系方式修改只返回 `complete`。重复领取返回 `consumed`，并发领取也只有一个成功。
6. 返回时取消通过 `POST .../{id}/poll?cancel=true`；未领取的结果变为 `cancelled`，之后不能批准或领取。五分钟后返回 `expired`。领取结果丢失时应重新开始授权，不能重复领取旧结果。

应用会话沿用现有三十天有效期和 `/auth/logout` 撤销机制。服务端仅保存所有密钥和令牌的 SHA-256 摘要。创建授权按来源 IP 限制每小时六十次。网页禁止跨站嵌入、不缓存、不发送 Referer，也不向应用暴露脚本桥。

该授权页面只应部署到应用网络允许列表中的受信任 HTTPS 后端。它是礼遇自己的授权协议，并非通用 OAuth 提供方。

## 验证

针对独立迁移完成的测试数据库，以 `LIYU_TEST_DELIVERY=true` 启动服务器后运行：

```sh
python3 tests/browser_auth_e2e.py http://127.0.0.1:18788
```

设置测试数据库的 `DATABASE_URL` 时还会验证请求过期。测试会注册账号、修改测试用户联系方式；请勿对真实用户数据库执行。
