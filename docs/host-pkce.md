# LIYU 宿主 PKCE 后端契约

LIYU-MINI 1.0.38 使用 OctoSense 标准 `auth` 后端连接；需要支持 `backend-api-v1` 的宿主，macOS 验证宿主为官方 desktop-v0.1.0-rc.2。四个 OAuth 地址与业务操作由清单声明，均在 `https://liyu.taidge.com` 的 HTTPS 443 来源下。更换地址使用 `scripts/configure-backend.py` 并重新校验。

1. 小程序调用 `auth.connect`，只提交 provider=backend 和 app.session scope。宿主先展示来源与权限说明，再在其自有 WebView 中打开后端网页登录。应用不能读取此 WebView、密码或验证码。
2. 宿主生成随机 state 与 S256 PKCE verifier。后端 `/oauth/authorize` 仅接受注册 client_id、scope 和宿主回调；请求五分钟后过期。网页账号登录生成十分钟临时会话，用户确认后立即撤销。
3. 后端重定向带一次性 code 和原 state。宿主检查 state，携带 verifier 调用 `/oauth/token`。错误 verifier/回调不能领取；并发领取只成功一次。后端仅保存令牌摘要。
4. 后端签发一小时访问令牌和九十天刷新令牌。刷新时轮换两者，并撤销该连接的旧访问令牌；重放已使用刷新令牌会撤销整个令牌族。宿主在 macOS 的 Keychain 保管凭据，不交给小程序，不使用应用文件作为保险库。
5. 应用仅在内存中持有宿主返回的不透明连接 handle，用 `auth.backend.me` 核验账号，调用 `auth.backend.request` 执行清单中固定的业务操作。宿主注入 Bearer；应用无法指定任意来源、URL、请求头或凭据。后端适配器再限制可访问的礼遇业务路由。
6. 业务写操作必须在前台宿主审阅并由用户实际按下确认。取消不执行写入。订单继续使用报价金额核验和幂等键，未知结果先回读，AI 只提供待确认建议。

## 恢复、切换与退出

`auth.active` 恢复宿主连接，`auth.backend.me` 失败会清空当前账号的界面数据；网络失败不会降级到演示。切换成功才更换连接；取消保留宿主原连接。业务响应绑定账号代次，切换后的旧响应不应用到新账号。`storage.accounts: true` 使偏好和日历关联按账号隔离。退出调用 `auth.disconnect`，宿主负责本地凭据清除及后端 logout，后端撤销整个刷新令牌族。

旧版的 `session.json`、`revocations.json` 在可见的应用存储内启动时删除，不读取或迁移明文令牌，需要重新连接。其他旧数据目录和旧版服务端会话不会因此自动销毁；旧会话仍受原有效期约束。需要立即终止遗留会话时，由服务端管理员撤销对应 sessions。新版本不会再生成这两类令牌文件。

邮箱/手机号修改仍在后端拥有的标准 WebReader 页面中完成，确认时必须匹配原账号。此用途的状态查询只能访问当前账号的 email/phone 授权，不能通过旧版 login 轮询取得令牌。网页 nonce 是五分钟联系方式事务密钥，不是账号会话。

## 日历与平台边界

设备日历使用可选 `device_calendar` API，先发现宿主支持，再检查和请求授权。读取日历列表、选择可写日历，添加/更新事件都有明确入口；权限、选择和写入保留原生确认。只写入双方已确认日期，全天事件使用 UTC 午夜和次日排他结束；不设置联系人或邀请名单。写后回读核验，重试先查找已创建事件；日历中的外部修改不会被覆盖。改期需再次主动更新，清除日期不会自动删除日历事件。

日历关联文件仅保存 handle、事件 id、修订和日期，不包含登录令牌。只在选定日历和对应日期的有限窗口查找本应用标记；不把日历事件提交给 AI。无支持、拒绝授权或超出原生 1970–2100 年范围时保留 .ics 导出。

macOS RC2 支持本实现；Windows/Linux 后端保护写入及 iOS 后端登录仍受上游限制。card-host 仅用于源码、布局及演示检查，不能证明宿主登录或日历能力。实际个人日历写入及真实模型建议需要用户授权/设备模型配置，自动测试不模拟用户实际确认。

## 服务端入口

| 方法 | 地址 | 约束 |
| --- | --- | --- |
| GET | /oauth/authorize | 注册客户端、app.session、S256、宿主回调 |
| POST（form） | /oauth/token | 一次性授权码或轮换刷新令牌；no-store |
| GET | /oauth/me | Bearer 身份；sub、label |
| POST | /oauth/logout | 撤销整个令牌族 |
| GET | /api/v1/host/read | 限制 path 到礼遇业务 API |
| POST | /api/v1/host/{post,put,patch,delete} | 固定适配器；保留报价与幂等参数 |
| GET | /api/v1/host/contact-status | 仅当前账号 email/phone 事务 |

新增表在启动时幂等升级，不重建数据库。刷新令牌族的九十天期限以首次签发为准，轮换不延长期限。数据库只保存令牌摘要。适配器不接受任意 URL 或头部，不跟随重定向，响应有大小上限。业务 API 按账号校验，不将设备日历权限当作双方日期确认。

测试：`DATABASE_URL=<独立管理库DSN> python3 tests/run_ci.py` 创建并删除独立临时库；不对线上库执行此测试。
