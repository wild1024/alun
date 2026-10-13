//! 验证 `#[derive(Model)]` 生成的代码。

mod common;
use common::models::*;
// 本文件以 `Order` 作领域模型；prelude 亦导出排序枚举 `Order`。
// 显式导入领域模型覆盖 glob 导入，避免与排序枚举同名冲突。
use alun_db::prelude::*;
use common::models::Order;

#[test]
fn model_table_name() {
    assert_eq!(User::TABLE, "users");
    assert_eq!(Order::TABLE, "orders");
}

#[test]
fn model_primary_key() {
    assert_eq!(User::PRIMARY_KEY, "id");
    assert_eq!(Order::PRIMARY_KEY, "id");
}

#[test]
fn model_readonly() {
    const { assert!(!User::READONLY) };
    const { assert!(!Order::READONLY) };
}

#[test]
fn model_fields_metadata() {
    let fields = User::fields();
    assert_eq!(fields.len(), 7);

    // id 字段
    let id_field = &fields[0];
    assert_eq!(id_field.name, "id");
    assert_eq!(id_field.column, "id");
    assert!(id_field.is_key);
    assert!(id_field.is_auto);
    assert!(!id_field.nullable);
    assert_eq!(id_field.ty, Type::I64);

    // email 字段
    let email_field = &fields[1];
    assert_eq!(email_field.name, "email");
    assert!(email_field.is_unique);
    assert_eq!(email_field.ty, Type::Text);

    // profile 字段（JSON + nullable）
    let profile_field = &fields[4];
    assert_eq!(profile_field.name, "profile");
    assert!(profile_field.nullable);
    assert!(matches!(profile_field.ty, Type::Nullable(_)));
}

#[test]
fn model_field_exprs_exist() {
    // 只要能编译就说明字段表达式生成正确
    let _ = User::id();
    let _ = User::email();
    let _ = User::name();
    let _ = User::status();
    let _ = User::profile();
    let _ = User::created_at();
    let _ = User::updated_at();
}

#[test]
fn model_field_expr_metadata() {
    let expr = User::email();
    assert_eq!(expr.column(), "email");
    assert_eq!(expr.table(), "users");
}

#[test]
fn model_as_row_roundtrip() {
    use chrono::Utc;
    let user = User {
        id: 42,
        email: "a@b.com".into(),
        name: "Alice".into(),
        status: 1,
        profile: None,
        created_at: Utc::now(),
        updated_at: Utc::now(),
    };

    let row = user.as_row().unwrap();
    assert_eq!(row.len(), 7);
    assert_eq!(row.get_typed::<i64>("id").unwrap(), 42);
    assert_eq!(row.get_typed::<String>("email").unwrap(), "a@b.com");
}

#[test]
fn model_from_row_roundtrip() {
    use chrono::Utc;
    let now = Utc::now();
    let mut row = Row::new();
    row.insert("id", 42i64);
    row.insert("email", "a@b.com");
    row.insert("name", "Alice");
    row.insert("status", 1i32);
    row.insert("profile", Value::Null);
    row.insert("created_at", now);
    row.insert("updated_at", now);

    let user = User::from_row(row).unwrap();
    assert_eq!(user.id, 42);
    assert_eq!(user.email, "a@b.com");
    assert_eq!(user.name, "Alice");
}

#[test]
fn model_pk_value() {
    use chrono::Utc;
    let user = User {
        id: 42,
        email: "a@b.com".into(),
        name: "Alice".into(),
        status: 1,
        profile: None,
        created_at: Utc::now(),
        updated_at: Utc::now(),
    };
    assert_eq!(user.pk_value(), Value::I64(42));
}

#[test]
fn type_mapper_value_map() {
    assert_eq!(OrderStatus::Pending.to_value(), Value::Text("P".into()));
    assert_eq!(OrderStatus::Confirmed.to_value(), Value::Text("C".into()));

    assert_eq!(
        OrderStatus::from_value(&Value::Text("S".into())).unwrap(),
        OrderStatus::Shipped
    );

    let err = OrderStatus::from_value(&Value::Text("X".into()));
    assert!(err.is_err());
}

/// D008 回归：`#[value_map(serde, …)]` 让 **JSON 编码与落库编码一致**。
///
/// 修复前 `#[value_map]` 只决定落库值（`Value::Text("draft")`），而 JSON 响应走
/// 派生的 serde（输出变体名 `"Draft"`）→ 同一枚举在库内与 API 上两套写法。
mod value_map_serde {
    use alun_db::prelude::*;

    // serde 实现由 `value_map(serde, …)` 生成，不额外 derive（否则重复实现）
    #[derive(TypeMapper, Debug, Clone, Copy, PartialEq, Eq)]
    #[value_map(serde, "draft" => Draft, "issued" => Issued)]
    pub enum Status {
        Draft,
        Issued,
    }

    #[test]
    fn json_uses_the_same_keys_as_the_database() {
        use alun_db_core::stmt::Value;

        // 落库编码
        assert_eq!(Status::Issued.to_value(), Value::Text("issued".into()));
        // JSON 编码一致
        assert_eq!(
            serde_json::to_string(&Status::Issued).expect("serialize"),
            "\"issued\""
        );
        assert_eq!(
            serde_json::from_str::<Status>("\"draft\"").expect("deserialize"),
            Status::Draft
        );
        // 变体名不再被接受（编码只有一套）
        assert!(serde_json::from_str::<Status>("\"Issued\"").is_err());
    }
}
