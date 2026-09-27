# 按联系方式送礼

## 业务边界

普通送礼不要求好友关系。发送方提供一个手机号或邮箱，后端只用已验证的 `contact_identities` 查找收件人；已有账号直接写入站内通知，未匹配账号的礼物保留空 `recipient_id`，由邮件／短信邀请注册或登录并验证联系方式。注册／绑定成功后自动归入收件人的礼盒，不创建 `friendships`。心愿认领仍保留原有好友／受众规则。

历史 `user_profiles.phone/email` 未经验证，不参与投递匹配，资料 API 明确返回 `phone_verified/email_verified`。送礼接口不返回联系方式是否已注册；订单中的联系方式收件人 ID 隐藏。礼物归属确定后固定为平台用户 ID，后续改绑不会转移旧礼物。

## API

- `POST /api/v1/auth/challenges`：`{"kind":"email","value":"recipient@example.com","purpose":"register"}`。返回 `challenge_id` 和有效期。`purpose:"bind"` 必须带当前用户 Bearer token。
- `POST /api/v1/auth/register`：`{"identifier":"recipient@example.com","password":"至少八字符","challenge_id":"…","code":"…"}`。
- `PUT /api/v1/me/email` 或 `/me/phone`：带 Bearer token，`{"value":"…","challenge_id":"…","code":"…"}`。
- `POST /api/v1/cart/items`：`{"product_id":0,"recipient":{"kind":"phone","value":"13800138000","label":"朋友"}}`。一次只支持一个联系方式，也兼容旧 `recipient_id` 参数；不能同时提供两种形式。下单及测试支付沿用现有订单 API。
- `GET /api/v1/notifications`、`POST /api/v1/notifications/{id}/read`：只允许本人读取／标记通知。
- `GET /gift-invitations/{token}`：注册、登录及验证领取页面，不泄露礼物内容或发送方身份。
- `POST /api/v1/gift-invitations/{token}/claim`：必须登录且已验证目标联系方式。持有链接本身不能领取别人的礼物；成功领取可重复调用。

手机号统一为 E.164，国内 11 位手机号码补 `+86`。邮箱域名转小写，本地部分保留大小写。验证码有效期十分钟、最多五次尝试；每联系方式一分钟一次，每来源／联系方式每小时最多三十次。验证码存哈希，成功后不可重用。新账号密码用 Argon2；历史密码摘要保持兼容登录。

## 配置邮件／短信供应商

默认不发真实消息。不配置供应商时，正式验证码请求返回 503，未注册收件人的邀请留在投递队列，前端显示等待投递。配置示例见 `.env.example`：

```dotenv
LIYU_TEST_DELIVERY=false
LIYU_PUBLIC_URL=https://liyu.example.com
LIYU_DELIVERY_WEBHOOK=https://delivery.example.com/send
LIYU_DELIVERY_TOKEN=由运营配置的服务凭证
```

`LIYU_DELIVERY_WEBHOOK` 是供应商适配服务：根据 `channel` 选择邮件或短信。后端以 POST JSON 调用，发送 `Content-Type: application/json`、`Idempotency-Key: <event_key>`，配置 token 时发送 Bearer 凭证。生产必须使用 HTTPS，校验证书。请求体：

```json
{
  "id": 123,
  "event_key": "gift:456:invite",
  "channel": "email",
  "destination": "recipient@example.com",
  "payload": {"type":"gift_invitation","title":"有一份礼物等你领取","body":"…","url":"https://liyu.example.com/gift-invitations/…"}
}
```

验证码消息的 `payload` 为 `{"type":"verification","code":"随机六位码","expires_in_seconds":600}`。适配服务需按 `event_key` 去重，在接受消息后返回 HTTP 2xx；这只代表供应商接受，不能证明收件人已读或运营商最终送达。提供商的模板、退订／发送限制和最终投递回执在适配服务实现。目前未内置特定供应商 SDK。

投递队列与支付写入同一数据库事务。worker 每五秒处理任务，有租约恢复，单次 HTTP 最多十秒，失败指数退避，最多八次；管理台“邮件 / 短信投递”可查看状态及重试有效任务。管理报告和审计不返回号码、邮箱、验证码或邀请 token。成功／取消后清除消息 payload；目的联系方式仍保留在业务记录及队列中，部署方需按自己的数据保留政策管理。验证码过期后停止投递；礼物领取、撤回或到期后停止未开始的邀请投递。已交给供应商的消息无法撤回。

`LIYU_TEST_DELIVERY=true` 是明确启用的开发模式：固定测试码 123456 可由 API 返回，仅该模式允许回环 HTTP。`just dev` 默认打开它；生产应显式设置 false。不要在公网生产环境启用测试模式。

## 数据迁移与上线

启动时自动执行 `20260928000000_contact_delivery` 迁移：新增联系方式身份、验证码、投递队列表，购物车／订单／礼物允许联系方式收件人，并扩展站内通知。已有用户、好友和礼物保留，不把历史未验证号码推定为真实身份。回滚前必须解决所有无用户 ID 的待领取记录；回滚迁移会拒绝直接丢弃这些记录。部署前按现有数据库流程备份。

未注册收件人的礼物到期自动退款、回补库存，复用现有退款账本，避免重复退款。注册和支付并发时后台补充归属，通知事件去重。

## 客户端与验证

LiYu 可手工新增／修改电话和邮箱；macOS Contacts 导入只在用户点击后请求系统访问；其他平台使用文件导入。文件选择器支持 UTF-8 CSV／VCF，保留多号码、多邮箱，导入前显示有效／无效／重复统计并要求确认。CSV 使用姓名、电话、邮箱等常见表头；不支持任意列映射和 Excel 工作簿。老的仅姓名联系人会保留，需要补充联系方式才能用于新流程。

站内通知由 App 轮询并支持已读及打开礼物。目前没有 APNs／FCM 系统推送，支付仍为测试支付。正式邮件／短信要配置上述适配服务。

测试：`cargo test --all-targets -- --test-threads=8`；`tests/contact_delivery_e2e.py` 运行真实隔离 PostgreSQL 与本地 HTTP mock，涵盖非好友送礼、待领取注册归属、绑定归属、通知权限、失败重试、验证码重放、重复支付和退款；`tests/contact_provider_e2e.py` 用本地证书校验 HTTPS 验证正式随机码通道。测试要求数据库名包含 `contact_test`，禁止使用业务数据库。前端覆盖四种窗口宽度下的控件动作、导入确认／取消及联系方式选择。邀请页已进行浏览器视觉检查；系统通讯录权限交互和真实供应商收件箱尚未端到端验证。
