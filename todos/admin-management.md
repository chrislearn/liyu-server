# 礼遇管理后台功能表与实施核对

日期：2026-09-27；主审/实施：外环(codex)。目标是 liyu-server 的运营后台；消费端 OctoSense/apps/liyu 作为业务依据，本轮 Dioxus 改造指 /admin。现有消费端仍是 Makepad。

## 前端业务到管理功能的映射

| 管理模块 | 细分功能 | 前端依据 | 验收要点 |
|---|---|---|---|
| 用户管理 | 搜索/分页、资料及注册时间、启用/禁用、撤销会话、关联订单/钱包 | profile_client.rs、account_ui.rs、lib.rs 账户页 | 禁用后已有 token 及新登录失效，敏感凭据不返回 |
| 商品管理 | 新建/编辑、分类品牌标签、实物/电子礼、详情、图片上传 | data.rs 商品目录、lib.rs 挑礼/详情 | 保留原商品 ID，字段校验、图片真实解码 |
| 上架/下架 | 独立状态操作、下架后禁止新增订单及支付 | commerce_client.rs 购物车/结算 | 状态变更有审计，历史订单价格不变 |
| 商品价格 | 分单位售价编辑、变更历史、订单快照 | commerce_client.rs、data.rs 金额规则 | 非负上限、旧订单不随改价变化 |
| 库存管理 | 商品库存、入库/出库/盘点调整、原因、库存流水、低库存查看 | commerce_client.rs 结算 | 行锁、不可负库存、支付扣减进入流水 |
| 优惠券 | 种类(促销/新人/补偿)、固定减免/百分比、门槛、最高优惠、适用商品/分类、开始/过期时间、发行上限、每人限领、启用/停用、发放/撤销、使用记录 | data.rs 电子券与 commerce_client.rs 结算 | 优惠券与商品兑换券分开，订单服务端算折扣、占用/使用/取消释放 |
| 礼品回收价格 | 按商品启用/停用、固定回收价/比例、有效期、回收报价、折余额记录 | data.rs cashout_quote、lib.rs 4676 折成余额 | 以礼物支付价值封顶，报价/执行用同一规则，重复不重复入账 |
| 钱包及售后 | 服务端余额/流水查询、回收入账、撤回退款、换礼补差/退差 | data.rs ledger、lib.rs wallet/exchange | 事务账本、幂等、不可任意覆盖余额 |
| 订单 | 查询/分页、用户/商品明细、原价/优惠/实付、未付款取消 | commerce_client.rs、commerce.rs | 已付单禁止直接改价/状态，取消释放券 |
| 礼物 | 查询状态、收送双方、预约/过期、解谜次数、撤回/回收/换礼记录 | gift_client.rs、gifting.rs、lib.rs 礼盒 | 不展示答案哈希，消费端维持送礼人隐私投影 |
| 物流 | 查询/编辑运单、轨迹追加、签收标记、确认收货查询 | gift_client.rs、fulfillment.rs | 仅已收下实物可发货，不冒充本人确认 |
| 熟人与心愿单 | 双方关系查询/解除、心愿单查询/关闭、认领状态 | wishui.rs、wishlist.rs | 解除后不再拥有关系授权，关闭不篡改已付认领 |
| 契约 | 礼物契约查询、履约/豁免状态 | data.rs Pact、lib.rs 契约页 | 必须关联已处理礼物，不篡改礼物金额 |
| 运营通知 | 定向通知发布、到期时间、列表/撤销、用户已读 | lib.rs 提醒开关/本地通知 | 不上传联系人簿，不暴露谜底 |
| 概览与审计 | 用户/商品/库存/订单指标、操作人/时间/原因/变更记录 | 所有业务页 | 管理 Cookie 与用户 Bearer 隔离、写入 CSRF、审计与写入同事务 |
| Dioxus 前端 | 登录、模块导航、表格查询/分页、中文表单、加载/失败/空状态、商品媒体 | 现有 web/admin.* | 原生 JS 界面替换，编译 WASM 并由 Salvo 同源提供 |

## 任务与验收状态

- [x] A01 从前端梳理功能表及细分功能；任务文件仅放 todos。
- [x] A02 唯一 init up/down SQL 扩展及已有库无损自动升级。
- [x] A03 用户管理与禁用授权链。
- [x] A04 商品、上下架、价格/库存历史及有原因的库存调整。
- [x] A05 优惠券规则、发放撤销及结算占用/核销/取消。
- [x] A06 回收定价、服务端报价/折余额/换礼与钱包账本。
- [x] A07 订单/礼物/物流管理。
- [x] A08 熟人/心愿单/契约/通知管理。
- [x] A09 概览、管理写入审计与安全边界。
- [x] A10 Dioxus 管理前端及构建/服务接入。
- [x] A11 PostgreSQL fresh up/down、旧库重复升级、Rust fmt/test/clippy、HTTP 正负向流程及浏览器验收。

## 范围边界

真实支付、提现、短信邮件供应商未接入，本轮沿用测试支付；钱包余额只能在礼遇内使用。消费端部分页面仍是本地演示（详见 app-api.md），管理后台和领域 API 完成不等于消费端已经切换在线；不将演示余额导入真实服务端。既有 docs/_todo.md 已移到 todos/app-api.md，避免多个任务目录。

## 验收记录

本轮完成 A01–A11。以下命令在隔离 PostgreSQL 数据库运行，未对 operator 的现有业务库重建或清数据：

- `DATABASE_URL=<独立库> cargo test --offline --locked --all-targets -- --test-threads=8`：33 passed，包含从旧完整 schema 安装管理扩展、重复安装及数据/库存保留。
- 服务端 `cargo fmt --all -- --check`、`cargo clippy --offline --locked --all-targets -- -D warnings`：通过。
- Dioxus `cargo fmt --manifest-path admin-ui/Cargo.toml -- --check`、`cargo clippy --offline --locked --manifest-path admin-ui/Cargo.toml --target wasm32-unknown-unknown -- -D warnings`：通过。
- `python3 scripts/build-admin.py`：release WASM 构建与资源安装成功。避免动态 document::Title（会请求 JS eval），保留 CSP 的 self + wasm-unsafe-eval；标题由静态生成 index 提供。
- `LIYU_TEST_ADMIN_USERNAME/PASSWORD=<测试账号> python3 tests/admin_e2e.py http://127.0.0.1:18805`：53 项通过，原商品/媒体/鉴权/CSRF/库存竞争流程保持。
- 同一全新测试库 `python3 tests/management_e2e.py http://127.0.0.1:18805`：146 项 HTTP 检查通过，另含真实并发库存调整、限额发券、换礼竞争；覆盖百分比最高优惠、日期错误/未来/过期、新人券资格、取消释放、支付价格快照、回收实际实付封顶、重复结算响应一致、钱包守恒、会话撤销、关系解除、契约履约、定向通知隔离。
- 独立新库 `pwsh -NoProfile -File tests/e2e.ps1 -BaseUrl http://127.0.0.1:18803`：消费端既有端到端 API 流程通过。
- 独立数据库执行唯一 init `up.sql`→`down.sql`：成功，public 表数 0。由 HEAD 旧 schema 建库，运行 `scripts/upgrade-admin.py` 两次后：3 用户 / 33 商品 / 1 管理版本，未重播 seed。
- 浏览器实际登录、查看全部模块导航、在商品 0 将库存 100 调至 101、填写原因保存，显示结果正确；新建“浏览器验收券”（种类/减免/日期/配额表单）实际保存成功。截图：`/Users/chris/.octos/outer/verify/liyu-management-dioxus.png`。验证数据只写入独立测试库。

界面通过键盘操作完成上述保存验收；其余细分操作由真实 HTTP 测试验证，不声称逐页进行了全部手工操作。


## 另行跟踪的后续工作（不与本轮管理后台完成混淆）

- [ ] 消费端钱包/回收报价/优惠券选择/通知/契约页面接入新服务端 API，并清晰标记现有本地演示。
- [ ] 独立钱包流水分页、普通结算钱包抵扣、测试充值（本轮换礼已支持钱包补差）。
- [ ] 后台定时到期退款与自动业务通知、通知偏好同步。
- [ ] 真实支付、提现及供应商接入；管理员角色权限细分。

## 下拉控件修正（2026-09-28）

- [x] A12 将所有管理表单原生 select 替换为统一 Dioxus 下拉；对齐触发框、统一白底绿系选中样式，处理弹窗滚动/视口边界，支持键盘及点击外部关闭；构建及浏览器验证后核对完成。

A12 验收：所有管理表单统一使用 FormSelect；菜单与触发框等宽、间隔 6px，白底及绿系选中样式，fixed 定位避开弹窗裁切，按视口空间向上/下展开。支持方向键/Home/End/Enter/Space、Escape/Tab、鼠标选择及外部关闭，长选项键盘定位自动滚动；未匹配值显示“请选择”。表单滚动及窗口 resize 关闭菜单。CSS 随前端构建安装为内容哈希资源，避免正在运行的 Rust 服务提供编译时旧样式。

- Dioxus fmt、wasm32 clippy `-D warnings`、release WASM 构建、服务端构建及 `git diff --check` 通过。CSS 资源 HTTP 200、Content-Type text/css，内容与源文件一致。
- 实际浏览器：用户设置菜单 x=270、宽360，与触发框一致，top=420.53，距触发框 bottom=414.53 为6px；鼠标选择/外部关闭，Home/End/Enter/Escape/Tab 通过。优惠券种类和优惠方式切换正确；分类9项 End 后 scrollTop=120、末项可见，仅一个菜单。窄屏390×720单列，菜单和触发框 x=50、宽290、位于视口内；resize 关闭菜单。优惠券表单 PageDown 后 modal scrollTop=223、菜单数0。商品分类选择末项正确。浏览器无 error/warn。
- 独立测试库 liyu_management_e2e_final 用户1通过新组件选择“是”、填写原因并提交，显示“已保存”，保持既有启用状态。未修改 operator 业务库；临时验证服务与标签页已关闭。截图：`/Users/chris/.octos/outer/verify/liyu-dropdown-fixed.png`。
