//! 分页结果桥接：把数据库层的 `Page<T>` 转成 Web 层的 `PageData` / `Res`。
//!
//! `alun-db` 与 `alun-core` 互不依赖（前者只依赖 `alun-db-core`），而 `Page`
//! 与 `PageData` 分属两个 crate —— 按孤儿规则，`From`/`Into` 只能实现在同时
//! 依赖二者的 `alun-web`，因此这里以 trait 形式提供转换。
//!
//! 两者字段名不同（`Page::size` ↔ `PageData::page_size`，且 `Page` 额外有
//! `pages`），手工映射极易错位；用这里的转换可避免直接搬运字段。

use crate::api::{PageData, Res};
use alun_db::Page;
use serde::Serialize;

/// `alun_db::Page<T>` → `PageData<Vec<T>>` / `Res<PageData<Vec<T>>>`
///
/// `T` 需实现 `Serialize`（`Res` / `PageData` 的要求）。数据库原始行
/// `alun_db::Row` 不实现 `Serialize`，因此需先用 [`Page::map`] 映射成 DTO：
///
/// # 示例
///
/// ```ignore
/// #[derive(serde::Serialize)]
/// struct UserRes { id: String, name: String }
///
/// let page = db()
///     .paginate("SELECT * FROM users ORDER BY id", pq.page, pq.page_size)
///     .await?
///     .map(|r| UserRes {
///         id: r.get_typed::<String>("id").unwrap_or_default(),
///         name: r.get_typed::<String>("name").unwrap_or_default(),
///     });
///
/// Ok(page.into_res())   // { code:0, msg:"ok", data:{ list, total, page, page_size } }
/// ```
pub trait PageResponseExt<T: Serialize> {
    /// 转成 `PageData`，保留 `total` / `page` / `page_size`
    fn into_page_data(self) -> PageData<Vec<T>>;

    /// 转成标准分页响应 `Res<PageData<Vec<T>>>`
    fn into_res(self) -> Res<PageData<Vec<T>>>;
}

impl<T: Serialize> PageResponseExt<T> for Page<T> {
    fn into_page_data(self) -> PageData<Vec<T>> {
        PageData::new(self.list, self.total, self.page, self.size)
    }

    fn into_res(self) -> Res<PageData<Vec<T>>> {
        Res::page(self.list, self.total, self.page, self.size)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Page<String> {
        Page {
            list: vec!["a".into(), "b".into()],
            total: 7,
            page: 2,
            size: 2,
            pages: 4,
        }
    }

    #[test]
    fn page_data_keeps_pagination_metadata() {
        let data = sample().into_page_data();
        assert_eq!(data.list, vec!["a".to_string(), "b".to_string()]);
        assert_eq!(data.total, 7);
        assert_eq!(data.page, 2);
        assert_eq!(
            data.page_size, 2,
            "Page::size 必须映射到 PageData::page_size"
        );
    }

    #[test]
    fn into_res_wraps_page_data() {
        let res = sample().into_res();
        assert_eq!(res.code, 0);
        let data = res.data.expect("分页响应必须带 data");
        assert_eq!(data.total, 7);
        assert_eq!(data.page_size, 2);
    }
}
