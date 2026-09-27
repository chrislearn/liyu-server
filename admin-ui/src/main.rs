use dioxus::prelude::*;
use gloo_net::http::{Request, RequestBuilder};
use serde_json::{json, Value};
fn main() {
    dioxus::launch(App);
}
const MODULES: &[(&str, &str)] = &[
    ("overview", "工作概览"),
    ("users", "用户管理"),
    ("products", "商品管理"),
    ("inventory", "库存管理"),
    ("low-stock", "低库存预警"),
    ("prices", "商品价格"),
    ("history", "库存 / 价格流水"),
    ("coupons", "优惠券设置"),
    ("issued-coupons", "优惠券发放记录"),
    ("recycling", "礼品回收价格"),
    ("wallet", "钱包流水"),
    ("orders", "订单管理"),
    ("gifts", "礼物管理"),
    ("shipments", "物流管理"),
    ("friends", "熟人关系"),
    ("wishlists", "心愿单"),
    ("contracts", "礼物契约"),
    ("notifications", "运营通知"),
    ("deliveries", "邮件 / 短信投递"),
    ("audit", "操作审计"),
];
async fn api(path: &str, method: &str, body: Option<Value>, csrf: &str) -> Result<Value, String> {
    let mut req = RequestBuilder::new(&format!("/admin/api/{path}"))
        .method(method.parse().unwrap())
        .header("X-Admin-Request", "1");
    if !csrf.is_empty() {
        req = req.header("X-CSRF-Token", csrf);
    }
    let req = if let Some(body) = body {
        req.header("Content-Type", "application/json")
            .body(body.to_string())
            .map_err(|e| e.to_string())?
    } else {
        req.build().map_err(|e| e.to_string())?
    };
    let r = req.send().await.map_err(|e| e.to_string())?;
    if r.status() == 204 {
        return Ok(Value::Null);
    }
    let data: Value = r.json().await.map_err(|e| e.to_string())?;
    if !r.ok() {
        return Err(if r.status() == 401 {
            "登录已失效，请重新登录".into()
        } else {
            data["error"].as_str().unwrap_or("操作失败").into()
        });
    }
    Ok(data)
}
fn label(k: &str) -> &str {
    match k {
        "id" => "编号",
        "name" => "名称",
        "identifier" => "登录标识",
        "display_name" => "用户称呼",
        "is_active" => "启用",
        "created_at" => "创建时间",
        "phone" => "手机",
        "email" => "邮箱",
        "sessions" => "有效会话",
        "balance_cents" => "余额（分）",
        "price_cents" => "售价（分）",
        "stock" => "库存",
        "category" => "分类",
        "physical" => "实物礼品",
        "brand" => "品牌",
        "kind" => "种类",
        "spec" => "规格",
        "description" => "描述 / 轨迹说明",
        "tags" => "标签（逗号分隔）",
        "product_id" => "商品编号",
        "old_price_cents" => "原价（分）",
        "new_price_cents" => "新价（分）",
        "old_stock" => "原库存",
        "new_stock" => "新库存",
        "old_active" => "原上架状态",
        "new_active" => "新上架状态",
        "administrator_id" => "操作人编号",
        "reason" => "操作原因",
        "delta" => "库存调整数量（正数入库 / 负数出库）",
        "discount_kind" => "优惠方式",
        "value" => "优惠值（分 / 万分比）",
        "min_spend_cents" => "使用门槛（分）",
        "max_discount_cents" => "最高优惠（分，0 为不限）",
        "starts_at" => "开始时间（UTC）",
        "expires_at" => "过期时间（UTC）",
        "issue_limit" => "总发行上限",
        "per_user_limit" => "每人限领",
        "issued_count" => "已发行",
        "template_id" => "券模板编号",
        "user_id" => "用户编号",
        "status" => "状态",
        "order_id" => "订单编号",
        "issued_at" => "发放时间",
        "used_at" => "核销时间",
        "mode" => "回收方式",
        "amount_cents" => "变动金额（分）",
        "gift_id" => "礼物编号",
        "buyer_id" => "买家编号",
        "subtotal_cents" => "原总价（分）",
        "discount_cents" => "优惠（分）",
        "total_cents" => "实付（分）",
        "coupon_id" => "使用券编号",
        "paid_at" => "付款时间",
        "items" => "订单明细",
        "sender_id" => "送礼人编号",
        "recipient_id" => "收礼人编号",
        "state" => "礼物状态",
        "unlock_kind" => "解谜方式",
        "attempts" => "答题次数",
        "available_at" => "预约时间",
        "settled_at" => "处理时间",
        "exchanged_item_id" => "换礼商品编号",
        "carrier" => "快递公司",
        "tracking_number" => "运单号",
        "recipient_name" => "收件人",
        "recipient_phone" => "收件电话",
        "recipient_address" => "收货地址",
        "delivered_at" => "快递签收时间",
        "recipient_confirmed_at" => "本人确认时间",
        "delivered" => "标记快递签收",
        "events" => "轨迹",
        "event_at" | "at" => "时间",
        "user_low_id" => "熟人甲",
        "user_high_id" => "熟人乙",
        "owner_id" => "用户编号",
        "title" => "标题",
        "note" => "备注",
        "occasion" => "场景",
        "event_on" => "事件日期",
        "closed_at" => "关闭时间",
        "item_count" => "心愿数",
        "claimed_count" => "已认领",
        "contract_text" => "契约内容",
        "gift_state" => "礼物状态",
        "updated_at" => "更新时间",
        "body" => "通知内容",
        "revoked_at" => "撤销时间",
        "read_at" => "已读时间",
        "username" => "管理员",
        "action" => "操作",
        "entity" => "业务对象",
        "entity_id" => "对象编号",
        "before_data" => "修改前",
        "after_data" => "修改后",
        "users" => "用户数",
        "active_products" => "上架商品",
        "low_stock" => "低库存商品（≤10）",
        "pending_orders" => "待支付订单",
        "paid_cents" => "测试成交额（分）",
        "wallet_cents" => "钱包余额合计（分）",
        _ => k,
    }
}
fn display(v: &Value) -> String {
    match v {
        Value::Null => "—".into(),
        Value::Bool(b) => if *b { "是" } else { "否" }.into(),
        Value::String(s) => match s.as_str() {
            "coffee" => "咖啡茶饮",
            "movie" => "电影演出",
            "trendy" => "潮流小物",
            "blind" => "盲盒",
            "sweet" => "甜点鲜花",
            "digital" => "数码家电",
            "home" => "家居生活",
            "baby" => "母婴亲子",
            "pending" => "待处理",
            "paid_test" => "测试已付",
            "cancelled" => "已取消",
            "sealed" => "未拆",
            "opened" => "已拆",
            "revealed" => "已揭晓",
            "accepted" => "已收下",
            "exchanged" => "已换礼",
            "cashed_out" => "已折余额",
            "withdrawn" => "已撤回",
            "expired" => "已过期",
            "available" => "可使用",
            "reserved" => "订单占用",
            "used" => "已核销",
            "revoked" => "已撤销",
            "fulfilled" => "已履约",
            "waived" => "已豁免",
            "voided" => "已作废",
            "promotion" => "促销券",
            "new_user" => "新人券",
            "compensation" => "补偿券",
            "fixed" => "固定金额",
            "percentage" => "百分比（万分比）",
            "cash_out" => "礼品回收",
            "exchange" => "换礼差额",
            "withdraw" => "撤回退款",
            _ => s,
        }
        .into(),
        Value::Array(a) => a.iter().map(display).collect::<Vec<_>>().join("；"),
        Value::Object(o) => o
            .iter()
            .map(|(k, v)| format!("{}：{}", label(k), display(v)))
            .collect::<Vec<_>>()
            .join(" · "),
        _ => v.to_string(),
    }
}
#[derive(Clone, PartialEq)]
struct Field {
    key: &'static str,
    kind: &'static str,
    options: Vec<(&'static str, &'static str)>,
}
fn field(k: &'static str, t: &'static str) -> Field {
    Field {
        key: k,
        kind: t,
        options: vec![],
    }
}
fn select(k: &'static str, opts: &[(&'static str, &'static str)]) -> Field {
    Field {
        key: k,
        kind: "select",
        options: opts.to_vec(),
    }
}
fn bool_field(k: &'static str) -> Field {
    select(k, &[("true", "是"), ("false", "否")])
}
const CATEGORIES: &[(&str, &str)] = &[
    ("coffee", "咖啡茶饮"),
    ("movie", "电影演出"),
    ("trendy", "潮流小物"),
    ("blind", "盲盒"),
    ("sweet", "甜点鲜花"),
    ("digital", "数码家电"),
    ("home", "家居生活"),
    ("baby", "母婴亲子"),
];
fn fields(action: &str) -> Vec<Field> {
    let mut f = match action {
        "product" => vec![
            field("name", "text"),
            select("category", CATEGORIES),
            field("price_cents", "number"),
            field("stock", "number"),
            bool_field("is_active"),
            bool_field("physical"),
            field("brand", "text"),
            field("kind", "text"),
            field("spec", "text"),
            field("description", "textarea"),
            field("tags", "text"),
        ],
        "user-status" | "listing" | "coupon-status" => vec![bool_field("is_active")],
        "stock" => vec![field("delta", "number")],
        "price" => vec![field("price_cents", "number")],
        "coupon" => vec![
            field("name", "text"),
            select(
                "kind",
                &[
                    ("promotion", "促销券"),
                    ("new_user", "新人券"),
                    ("compensation", "补偿券"),
                ],
            ),
            select(
                "discount_kind",
                &[
                    ("fixed", "固定金额减免"),
                    ("percentage", "比例减免（1000 = 10%）"),
                ],
            ),
            field("value", "number"),
            field("min_spend_cents", "number"),
            field("max_discount_cents", "number"),
            field("product_id", "optional-number"),
            select(
                "category",
                &[
                    ("", "全部分类"),
                    ("coffee", "咖啡茶饮"),
                    ("movie", "电影演出"),
                    ("trendy", "潮流小物"),
                    ("blind", "盲盒"),
                    ("sweet", "甜点鲜花"),
                    ("digital", "数码家电"),
                    ("home", "家居生活"),
                    ("baby", "母婴亲子"),
                ],
            ),
            field("starts_at", "datetime-local"),
            field("expires_at", "datetime-local"),
            field("issue_limit", "number"),
            field("per_user_limit", "number"),
            bool_field("is_active"),
        ],
        "coupon-issue" => vec![field("user_id", "number")],
        "recycle" => vec![
            bool_field("is_active"),
            select(
                "mode",
                &[
                    ("fixed", "固定回收金额"),
                    ("percentage", "比例回收（9200 = 92%）"),
                ],
            ),
            field("value", "number"),
            field("expires_at", "optional-date"),
        ],
        "shipment" => vec![
            field("carrier", "text"),
            field("tracking_number", "text"),
            field("recipient_name", "text"),
            field("recipient_phone", "text"),
            field("recipient_address", "text"),
            field("description", "text"),
            bool_field("delivered"),
        ],
        "contract" => vec![select(
            "status",
            &[("fulfilled", "已履约"), ("waived", "豁免契约")],
        )],
        "notify" => vec![
            field("user_id", "number"),
            field("title", "text"),
            field("body", "textarea"),
            field("expires_at", "datetime-local"),
        ],
        _ => vec![],
    };
    f.push(field("reason", "text"));
    f
}
#[derive(Clone, PartialEq)]
struct Editor {
    create: bool,
    action: String,
    id: i64,
    values: Value,
}
fn editor(action: &str, row: Value) -> Editor {
    let id = row["id"].as_i64().unwrap_or(0);
    let mut values = json!({"category":"home","price_cents":0,"stock":100,"physical":true,"is_active":true,"tags":[],"kind":"","discount_kind":"fixed","value":100,"min_spend_cents":0,"max_discount_cents":0,"issue_limit":100,"per_user_limit":1,"mode":"percentage","delta":0,"delivered":false});
    if let Some(o) = row.as_object() {
        for (k, v) in o {
            if !v.is_null() {
                values[k] = v.clone();
            }
        }
    }
    if action == "coupon" {
        values["category"] = json!("");
        values["kind"] = json!("promotion");
    }
    if action == "user-status" || action == "listing" || action == "coupon-status" {
        values["is_active"] = json!(row["is_active"] != true);
    }
    if action == "recycle" && row["mode"].is_null() {
        values["value"] = json!(9200);
    }
    Editor {
        create: row.is_null(),
        action: action.into(),
        id,
        values,
    }
}
fn actions(module: &str) -> Vec<(&str, &str)> {
    match module {
        "users" => vec![
            ("user-status", "启用 / 禁用"),
            ("user-sessions", "撤销登录会话"),
        ],
        "products" => vec![("product", "编辑"), ("listing", "上架 / 下架")],
        "inventory" | "low-stock" => vec![("stock", "调整库存")],
        "prices" => vec![("price", "调整价格")],
        "coupons" => vec![("coupon-issue", "发放"), ("coupon-status", "启停")],
        "issued-coupons" => vec![("coupon-revoke", "撤销")],
        "recycling" => vec![("recycle", "设置回收价")],
        "orders" => vec![("cancel-order", "取消待付订单")],
        "gifts" | "shipments" => vec![("shipment", "运单 / 轨迹")],
        "friends" => vec![("remove-friend", "解除关系")],
        "wishlists" => vec![("close-wishlist", "关闭")],
        "contracts" => vec![("contract", "履约 / 豁免")],
        "notifications" => vec![("revoke-notification", "撤销通知")],
        "deliveries" => vec![("retry-delivery", "重试投递")],
        _ => vec![],
    }
}
fn columns(module: &str) -> Vec<&str> {
    match module {
        "users" => vec![
            "id",
            "display_name",
            "identifier",
            "is_active",
            "balance_cents",
        ],
        "products" | "inventory" | "prices" | "low-stock" => vec![
            "id",
            "name",
            "category",
            "price_cents",
            "stock",
            "is_active",
        ],
        "history" => vec![
            "id",
            "product_id",
            "old_stock",
            "new_stock",
            "old_price_cents",
            "new_price_cents",
            "reason",
        ],
        "coupons" => vec![
            "id",
            "name",
            "kind",
            "discount_kind",
            "value",
            "expires_at",
            "is_active",
        ],
        "issued-coupons" => vec!["id", "name", "user_id", "status", "order_id", "expires_at"],
        "recycling" => vec!["id", "name", "mode", "value", "is_active", "expires_at"],
        "wallet" => vec![
            "id",
            "user_id",
            "amount_cents",
            "kind",
            "gift_id",
            "created_at",
        ],
        "orders" => vec![
            "id",
            "buyer_id",
            "subtotal_cents",
            "discount_cents",
            "total_cents",
            "status",
        ],
        "gifts" => vec![
            "id",
            "sender_id",
            "recipient_id",
            "product_id",
            "state",
            "price_cents",
        ],
        "shipments" => vec![
            "id",
            "carrier",
            "tracking_number",
            "delivered_at",
            "recipient_confirmed_at",
        ],
        "friends" => vec!["id", "user_low_id", "user_high_id", "status"],
        "wishlists" => vec![
            "id",
            "owner_id",
            "title",
            "event_on",
            "item_count",
            "claimed_count",
            "closed_at",
        ],
        "contracts" => vec!["id", "sender_id", "recipient_id", "contract_text", "status"],
        "notifications" => vec![
            "id",
            "user_id",
            "title",
            "expires_at",
            "revoked_at",
            "read_at",
        ],
        "deliveries" => vec![
            "id",
            "gift_id",
            "kind",
            "status",
            "attempts",
            "next_attempt_at",
            "last_error",
            "sent_at",
        ],
        "audit" => vec![
            "id",
            "username",
            "action",
            "entity",
            "entity_id",
            "reason",
            "created_at",
        ],
        _ => vec![],
    }
}

#[component]
fn App() -> Element {
    let mut identity = use_signal(|| Value::Null);
    let mut initialized = use_signal(|| false);
    let mut username = use_signal(String::new);
    let mut password = use_signal(String::new);
    let mut module = use_signal(|| "overview".to_string());
    let mut query = use_signal(String::new);
    let mut search = use_signal(String::new);
    let mut cursor = use_signal(|| -1_i64);
    let mut revision = use_signal(|| 0_u64);
    let mut notice = use_signal(String::new);
    let mut busy = use_signal(|| false);
    let mut editing = use_signal(|| None::<Editor>);
    let mut detail = use_signal(|| None::<Value>);
    use_future(move || async move {
        if let Ok(v) = api("me", "GET", None, "").await {
            identity.set(v);
        }
        initialized.set(true);
    });
    let data = use_resource(move || async move {
        let _ = revision();
        if identity().is_null() {
            return Ok(Value::Null);
        }
        let m = module();
        let path = if m == "overview" {
            "overview".to_string()
        } else {
            format!(
                "{}?q={}&cursor={}",
                if m == "products" {
                    "products".to_string()
                } else {
                    format!("reports/{m}")
                },
                encode(&search()),
                if m == "products" && cursor() == 0 {
                    -1
                } else {
                    cursor()
                }
            )
        };
        api(&path, "GET", None, "").await
    });
    let title = MODULES
        .iter()
        .find(|(id, _)| *id == module())
        .map(|(_, t)| *t)
        .unwrap_or("");
    let result = if data.state()() == UseResourceState::Pending {
        None
    } else {
        data.read().clone()
    };
    let content = if !initialized() {
        rsx! { main { p { "正在确认登录…" } } }
    } else if identity().is_null() {
        rsx! { main { section { class: "login",
            p { class: "eyebrow", "LIYU OPERATIONS" } h1 { "管理登录" } p { class: "muted", "独立管理员账户" }
            form {
                onsubmit: move |e| { e.prevent_default(); async move {
                    busy.set(true); notice.set(String::new());
                    match api("login", "POST", Some(json!({"username":username(),"password":password()})), "").await {
                        Ok(v) => { identity.set(v); password.set(String::new()); revision += 1; }, Err(e) => notice.set(e)
                    } busy.set(false);
                } },
                label { "用户名" input { value: username(), oninput: move |e| username.set(e.value()), autocomplete: "username" } }
                label { "密码" input { r#type: "password", value: password(), oninput: move |e| password.set(e.value()), autocomplete: "current-password" } }
                button { disabled: busy(), r#type: "submit", if busy() { "正在登录…" } else { "登录" } }
            }
        } } }
    } else {
        rsx! { div { class: "layout",
            nav { for (id,name) in MODULES {
                button { class: if module()==*id { "nav-active" } else { "nav-item" },
                    onclick: move |_| { module.set((*id).into()); cursor.set(-1); search.set(String::new()); query.set(String::new()); notice.set(String::new()); }, "{name}"
                }
            } }
            main { class: "workspace",
                div { class: "toolbar",
                    div { p { class: "eyebrow", "LIYU OPERATIONS" } h1 { "{title}" } }
                    div {
                        button { class: "quiet", onclick: move |_| revision += 1, "刷新" }
                        if module()=="products" { button { onclick: move |_| editing.set(Some(editor("product", Value::Null))), "新增商品" } }
                        if module()=="coupons" { button { onclick: move |_| editing.set(Some(editor("coupon", Value::Null))), "新建券规则" } }
                        if module()=="notifications" { button { onclick: move |_| editing.set(Some(editor("notify", Value::Null))), "发布通知" } }
                    }
                }
                if module()=="coupons" { p { class: "muted", "优惠规则创建后保留，启停控制使用；变更请新建券，保留历史订单依据。比例按万分比填写，1000 表示减免 10%。" } }
                if module()=="recycling" { p { class: "muted", "固定金额单位为分；9200 表示支付价值的 92%。回收金额以实际支付价值为上限，余额只能在礼遇内使用。" } }
                if module()!="overview" {
                    form { class: "search", onsubmit: move |e| { e.prevent_default(); search.set(query()); cursor.set(-1); },
                        input { placeholder: "搜索名称、编号或状态…", value: query(), oninput: move |e| query.set(e.value()) }
                        button { r#type: "submit", "查询" }
                    }
                }
                Report { module: module(), result, cursor, editing, detail, identity }
            }
        } }
    };
    rsx! {
        header {
            div { span { class: "mark", "礼" } strong { "礼遇" } span { class: "muted", "运营管理" } }
            if !identity().is_null() {
                div { span { {identity()["username"].as_str().unwrap_or("").to_string()} }
                    button { class: "quiet", disabled: busy(), onclick: move |_| async move {
                        busy.set(true);
                        match api("logout","POST",None,identity()["csrf_token"].as_str().unwrap_or("")).await {
                            Ok(_) => { identity.set(Value::Null); editing.set(None); detail.set(None); password.set(String::new()); }, Err(e)=>notice.set(e)
                        } busy.set(false);
                    }, "退出登录" }
                }
            }
        }
        {content}
        if !notice().is_empty() { div { class: "toast", role: "status", "{notice()}" } }
        if editing().is_some() { EditDialog { editing, identity, notice, busy, revision, title: title.to_string() } }
        if let Some(row)=detail() {
            div { class: "backdrop", section { class: "modal", role: "dialog", "aria-modal": "true",
                h2 { "记录详情" }
                dl { for (k,v) in row.as_object().into_iter().flatten() { dt { "{label(k)}" } dd { "{display(v)}" } } }
                button { class: "quiet", onclick: move |_| detail.set(None), "关闭" }
            } }
        }
    }
}
#[component]
fn Report(
    module: String,
    result: Option<Result<Value, String>>,
    mut cursor: Signal<i64>,
    mut editing: Signal<Option<Editor>>,
    mut detail: Signal<Option<Value>>,
    mut identity: Signal<Value>,
) -> Element {
    let v = match result {
        None => return rsx! {p{"正在加载…"}},
        Some(Err(e)) => {
            return rsx! {p{class:"notice","{e}"} button{onclick:move |_|{identity.set(Value::Null);editing.set(None);},"重新登录"}}
        }
        Some(Ok(v)) => v,
    };
    if module == "overview" {
        return rsx! { div { class:"metrics", for(k,val)in v.as_object().into_iter().flatten() {
            section { class:"metric",p{class:"muted","{label(k)}"}strong{"{display(val)}"} }
        } } p { class:"muted", "当前交易使用测试支付。所有金额以分记录。" } };
    }
    let rows = v["items"].as_array().cloned().unwrap_or_default();
    let cols = columns(&module);
    let acts = actions(&module);
    let next = v["next_cursor"].as_i64();
    rsx! {
        div { class:"table-wrap", table {
            thead { tr { for col in &cols { th { "{label(col)}" } } th { "操作" } } }
            tbody {
                for row in rows.clone() {
                    tr { key: "{row}",
                        for col in &cols { td { if module=="products"&&*col=="name" {img {src:format!("/api/v1/media/products/{}/thumb",row["id"]),alt:"",loading:"lazy"}} "{display(&row[*col])}" } }
                        td {
                            button { class:"quiet", onclick:{let row=row.clone();move |_|detail.set(Some(row.clone()))}, "详情" }
                            for(action,name)in &acts {
                                button { class:"quiet", onclick:{let row=row.clone();let action=action.to_string();move |_|editing.set(Some(editor(&action,row.clone())))}, "{name}" }
                            }
                        }
                    }
                }
                if rows.is_empty() { tr { td { colspan:"20", "暂无记录" } } }
            }
        } }
        div { class:"pagination",
            if cursor()>-1 {button{class:"quiet",onclick:move |_|cursor.set(-1),"返回首页"}}
            if let Some(n)=next {button{onclick:move |_|cursor.set(n),"下一页"}}
            span{class:"muted","本页 {rows.len()} 条"}
        }
    }
}
// Menus use viewport coordinates so the modal's scroll container cannot clip them.
fn menu_position(id: &str, count: usize) -> Option<(f64, f64, f64, f64)> {
    let window = web_sys::window()?;
    let rect = window
        .document()?
        .get_element_by_id(id)?
        .get_bounding_client_rect();
    let height = window.inner_height().ok()?.as_f64()?;
    let width = window.inner_width().ok()?.as_f64()?;
    let desired = (count as f64 * 40.0 + 10.0).min(250.0);
    let below = (height - rect.bottom() - 14.0).max(0.0);
    let above = (rect.top() - 14.0).max(0.0);
    let upwards = below < desired && above > below;
    let available = if upwards { above } else { below };
    let menu_height = desired.min(available);
    let top = if upwards {
        rect.top() - menu_height - 6.0
    } else {
        rect.bottom() + 6.0
    };
    let menu_width = rect.width().min(width - 16.0);
    Some((
        rect.left().clamp(8.0, (width - menu_width - 8.0).max(8.0)),
        top,
        menu_width,
        menu_height,
    ))
}

#[component]
fn FormSelect(
    field: Field,
    value: String,
    mut editing: Signal<Option<Editor>>,
    mut open_select: Signal<Option<String>>,
) -> Element {
    let key = field.key;
    let id = format!("select-{key}");
    let list_id = format!("options-{key}");
    let selected = field.options.iter().position(|(v, _)| *v == value);
    let mut highlighted = use_signal(|| selected.unwrap_or(0));
    let mut position = use_signal(|| (0.0, 0.0, 0.0, 0.0));
    let is_open = open_select().as_deref() == Some(key);
    let options = field.options.clone();
    let keyboard_options = options.clone();
    let button_id = id.clone();
    let keyboard_id = id.clone();
    use_effect(move || {
        use wasm_bindgen::JsCast;
        let index = highlighted();
        if open_select().as_deref() != Some(key) {
            return;
        }
        if let Some(document) = web_sys::window().and_then(|w| w.document()) {
            if let (Some(menu), Some(option)) = (
                document.get_element_by_id(&format!("options-{key}")),
                document.get_element_by_id(&format!("option-{key}-{index}")),
            ) {
                let bounds = menu.get_bounding_client_rect();
                let item = option.get_bounding_client_rect();
                if let Some(menu) = menu.dyn_ref::<web_sys::HtmlElement>() {
                    let delta = if item.bottom() > bounds.bottom() - 5.0 {
                        item.bottom() - bounds.bottom() + 5.0
                    } else if item.top() < bounds.top() + 5.0 {
                        item.top() - bounds.top() - 5.0
                    } else {
                        0.0
                    };
                    menu.set_scroll_top(menu.scroll_top() + delta.round() as i32);
                }
            }
        }
    });
    let (left, top, width, height) = position();
    rsx! {
        div { class: "form-select",
            button {
                id, r#type: "button", class: "select-trigger", role: "combobox",
                "aria-label": label(key), "aria-haspopup": "listbox",
                "aria-expanded": is_open, "aria-controls": list_id.clone(),
                "aria-activedescendant": if is_open { format!("option-{key}-{}", highlighted()) } else { String::new() },
                onblur: move |_| { if open_select().as_deref() == Some(key) { open_select.set(None); } },
                onclick: move |_| {
                    if is_open { open_select.set(None); }
                    else if let Some(p) = menu_position(&button_id, options.len()) {
                        position.set(p); highlighted.set(selected.unwrap_or(0)); open_select.set(Some(key.into()));
                    }
                },
                onkeydown: move |event| {
                    let count = keyboard_options.len();
                    let next = match event.key() {
                        Key::ArrowDown => Some(if is_open { (highlighted() + 1).min(count - 1) } else { selected.unwrap_or(0) }),
                        Key::ArrowUp => Some(if is_open { highlighted().saturating_sub(1) } else { selected.unwrap_or(0) }),
                        Key::Home => Some(0),
                        Key::End => Some(count - 1),
                        k if k == Key::Enter || k == Key::Character(" ".into()) => {
                            event.prevent_default();
                            if is_open {
                                if let Some(v) = editing.write().as_mut() { v.values[key] = json!(keyboard_options[highlighted()].0); }
                                open_select.set(None);
                            } else if let Some(p) = menu_position(&keyboard_id, count) {
                                position.set(p); highlighted.set(selected.unwrap_or(0)); open_select.set(Some(key.into()));
                            }
                            None
                        }
                        Key::Escape | Key::Tab => { open_select.set(None); None }
                        _ => None,
                    };
                    if let Some(next) = next {
                        event.prevent_default();
                        if let Some(p) = menu_position(&keyboard_id, count) {
                            position.set(p); highlighted.set(next); open_select.set(Some(key.into()));
                        }
                    }
                },
                span { {selected.map(|i| field.options[i].1).unwrap_or("请选择")} }
                span { class: "select-chevron", "aria-hidden": "true", "⌄" }
            }
            if is_open {
                div { class: "select-dismiss", onpointerdown: move |e| { e.prevent_default(); open_select.set(None); } }
                div {
                    id: list_id.clone(), class: "select-menu", role: "listbox", "aria-label": label(key),
                    left: "{left}px", top: "{top}px", width: "{width}px", max_height: "{height}px",
                    onpointerdown: move |e| e.prevent_default(),
                    for (index, (value, name)) in field.options.iter().enumerate() {
                        button {
                            key: "{value}", id: "option-{key}-{index}", r#type: "button", role: "option", tabindex: "-1",
                            class: if highlighted() == index { "select-option highlighted" } else { "select-option" },
                            "aria-selected": selected == Some(index),
                            onmouseenter: move |_| highlighted.set(index),
                            onclick: { let value = value.to_string(); move |_| {
                                if let Some(v) = editing.write().as_mut() { v.values[key] = json!(value); }
                                open_select.set(None);
                            } },
                            span { "{name}" }
                            if selected == Some(index) { span { "aria-hidden": "true", "✓" } }
                        }
                    }
                }
            }
        }
    }
}

#[component]
fn EditDialog(
    mut editing: Signal<Option<Editor>>,
    identity: Signal<Value>,
    mut notice: Signal<String>,
    mut busy: Signal<bool>,
    mut revision: Signal<u64>,
    title: String,
) -> Element {
    let mut open_select = use_signal(|| None::<String>);
    // A resize invalidates viewport coordinates; scrolling the modal closes the menu too.
    let resize_listener = use_hook(move || {
        use wasm_bindgen::JsCast;
        let handler =
            wasm_bindgen::closure::Closure::<dyn FnMut()>::new(move || open_select.set(None));
        if let Some(window) = web_sys::window() {
            let _ =
                window.add_event_listener_with_callback("resize", handler.as_ref().unchecked_ref());
        }
        std::rc::Rc::new(handler)
    });
    use_drop(move || {
        use wasm_bindgen::JsCast;
        if let Some(window) = web_sys::window() {
            let _ = window.remove_event_listener_with_callback(
                "resize",
                resize_listener.as_ref().as_ref().unchecked_ref(),
            );
        }
    });
    let ed = editing().unwrap();
    let heading = if ed.action == "product" {
        if ed.create {
            "新增商品".into()
        } else {
            "编辑商品".into()
        }
    } else {
        format!("{title} · 设置")
    };
    rsx! {
        div { class:"backdrop", section { class:"modal", role:"dialog", "aria-modal":"true", onscroll: move |_| open_select.set(None),
            h2 { "{heading}" } p { class:"muted","记录 #{ed.id} · 金额单位为分，日期按 UTC 填写" }
            form { onsubmit:move |ev|{ev.prevent_default(); async move {
                busy.set(true);notice.set(String::new());let ed=editing().unwrap();let body=prepare(&ed);
                let path=if ed.action=="product"{if ed.create{"products".into()}else{format!("products/{}",ed.id)}}else{format!("manage/{}/{}",ed.action,ed.id)};
                let method=if ed.action=="product"&&!ed.create{"PUT"}else{"POST"};
                match api(&path,method,Some(body),identity()["csrf_token"].as_str().unwrap_or("")).await {
                    Ok(v)=>{if ed.action=="product"&&ed.create{let mut saved=ed.clone();saved.id=v["id"].as_i64().unwrap();saved.create=false;editing.set(Some(saved));notice.set("商品已创建，可上传图片。".into());}else{editing.set(None);notice.set("已保存".into());}revision+=1;},Err(e)=>notice.set(e)
                }busy.set(false);
            }},
                div { class:"fields", for f in fields(&ed.action) {
                    label { key:"{f.key}","{label(f.key)}",
                        if f.kind=="select" {
                            FormSelect { value: form_value(&ed.values,f.key,f.kind), field: f.clone(), editing, open_select }
                        } else if f.kind=="textarea" {
                            textarea { value:form_value(&ed.values,f.key,f.kind),oninput:{let key=f.key;move|e|{if let Some(v)=editing.write().as_mut(){v.values[key]=json!(e.value());}}} }
                        } else {
                            input { r#type:match f.kind{"optional-number"=>"number","optional-date"=>"datetime-local",t=>t},value:form_value(&ed.values,f.key,f.kind),required:f.key=="reason",oninput:{let key=f.key;move|e|{if let Some(v)=editing.write().as_mut(){v.values[key]=json!(e.value());}}} }
                        }
                    }
                } }
                div { class:"toolbar",
                    button{r#type:"button",class:"quiet",disabled:busy(),onclick:move|_|editing.set(None),"取消"}
                    button{r#type:"submit",disabled:busy(),if busy(){"保存中…"}else{"确认保存"}}
                }
            }
            if ed.action=="product"&&!ed.create { ImageUpload { id:ed.id,identity,notice,busy } }
        } }
    }
}
fn prepare(ed: &Editor) -> Value {
    let mut body = ed.values.clone();
    for f in fields(&ed.action) {
        if body[f.key].is_null() {
            body[f.key] = json!("");
        }
        if let Some(raw) = body[f.key].as_str().map(str::to_owned) {
            body[f.key] = match f.kind {
                "number" | "optional-number" => {
                    raw.parse::<i64>().map_or(Value::Null, |n| json!(n))
                }
                "datetime-local" | "optional-date" => {
                    if raw.is_empty() {
                        Value::Null
                    } else {
                        json!(format!("{}:00Z", raw.chars().take(16).collect::<String>()))
                    }
                }
                _ => {
                    if ["is_active", "physical", "delivered"].contains(&f.key) {
                        json!(raw == "true")
                    } else if f.key == "tags" {
                        json!(raw
                            .split(',')
                            .map(str::trim)
                            .filter(|s| !s.is_empty())
                            .collect::<Vec<_>>())
                    } else {
                        json!(raw)
                    }
                }
            };
        }
    }
    body
}
#[component]
fn ImageUpload(
    id: i64,
    identity: Signal<Value>,
    mut notice: Signal<String>,
    mut busy: Signal<bool>,
) -> Element {
    rsx! { h3{"商品图片"}p{class:"muted","选择图片即上传 · JPEG / PNG / WebP · 最大 4 MiB"}
        for variant in ["thumb","card","detail"] {
            label { "{variant}", input {r#type:"file",accept:"image/png,image/jpeg,image/webp",disabled:busy(),onchange:move|e| {
                let file=e.files().into_iter().next();async move {
                    if let Some(file)=file {busy.set(true);let csrf=identity()["csrf_token"].as_str().unwrap_or("").to_string();
                        match file.read_bytes().await {
                            Ok(bytes)=>{
                                let req=Request::post(&format!("/admin/api/products/{id}/images/{variant}")).header("X-Admin-Request","1").header("X-CSRF-Token",&csrf).body(bytes.to_vec());
                                match req {Ok(req)=>match req.send().await {Ok(r)if r.ok()=>notice.set("图片已上传".into()),Ok(r)=>notice.set(format!("图片上传失败（{}）",r.status())),Err(e)=>notice.set(e.to_string())},Err(e)=>notice.set(e.to_string())}
                            },Err(e)=>notice.set(e.to_string())
                        }busy.set(false);
                    }
                }
            } } }
        }
    }
}
fn form_value(v: &Value, k: &str, kind: &str) -> String {
    let val = &v[k];
    if val.is_null() {
        return String::new();
    }
    if k == "tags" {
        return val
            .as_array()
            .map(|a| {
                a.iter()
                    .filter_map(Value::as_str)
                    .collect::<Vec<_>>()
                    .join(",")
            })
            .unwrap_or_else(|| val.as_str().unwrap_or("").into());
    }
    if kind == "datetime-local" || kind == "optional-date" {
        return val.as_str().unwrap_or("").chars().take(16).collect();
    }
    val.as_str()
        .map(str::to_owned)
        .unwrap_or_else(|| val.to_string())
}
fn encode(s: &str) -> String {
    s.bytes()
        .map(|b| {
            if b.is_ascii_alphanumeric() || b"-_.~".contains(&b) {
                (b as char).to_string()
            } else {
                format!("%{b:02X}")
            }
        })
        .collect()
}
