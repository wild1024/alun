use proc_macro::TokenStream;
use quote::quote;
use syn::{parse_macro_input, ItemStruct, Lit};
use syn::parse::Parser;

/// `#[task_handler]`：标记一个 TaskHandler 实现，编译期自动注册到 TASK_HANDLERS 分布式切片
///
/// # 参数
///
/// | 参数 | 类型 | 默认值 | 说明 |
/// |------|------|--------|------|
/// | `task_type` | i16 | **必填** | 任务类型标识，唯一 |
/// | `priority` | Normal/High/Low/Critical | Normal | 任务优先级 |
/// | `timeout_seconds` | u64 | 300 | 执行超时（秒） |
/// | `max_retries` | u32 | 3 | 最大重试次数 |
/// | `retry_strategy` | Fixed/Linear/Exponential | Linear | 重试策略 |
/// | `retry_delay_seconds` | u64 | 30 | 基础重试延迟（秒） |
/// | `max_retry_delay_seconds` | u64 | 300 | 最大重试延迟（秒） |
/// | `topic` | &str | "task_{task_type}" | Kafka topic 名称 |
/// | `description` | &str | "" | 任务描述 |
/// | `dead_letter_topic` | Option<&str> | None | 死信队列 topic |
///
/// # 示例
///
/// ```ignore
/// #[alun::task_handler(
///     task_type = 1,
///     topic = "export_tasks",
///     timeout_seconds = 60,
///     max_retries = 3,
///     description = "数据导出任务"
/// )]
/// struct ExportHandler;
/// ```
pub fn task_handler_impl(attr: TokenStream, item: TokenStream) -> TokenStream {
    let input = parse_macro_input!(item as ItemStruct);
    let struct_name = &input.ident;

    let mut task_type: Option<i16> = None;
    let mut priority = quote! { ::alun::alun_task::TaskPriority::Normal };
    let mut timeout_seconds = 300u64;
    let mut max_retries = 3u32;
    let mut retry_strategy = quote! { ::alun::alun_task::RetryStrategy::Linear };
    let mut retry_delay_seconds = 30u64;
    let mut max_retry_delay_seconds = 300u64;
    let mut topic: Option<String> = None;
    let mut description = "";
    let mut dead_letter_topic: Option<String> = None;

    let parser = syn::meta::parser(|meta| {
        let name = meta.path.get_ident().map(|i| i.to_string()).unwrap_or_default();
        match name.as_str() {
            "task_type" => {
                let value = meta.value().unwrap();
                let lit: Lit = value.parse().unwrap();
                if let Lit::Int(v) = lit {
                    task_type = Some(v.base10_parse().unwrap_or(0));
                }
            }
            "priority" => {
                let value = meta.value().unwrap();
                let lit: Lit = value.parse().unwrap();
                if let Lit::Str(v) = lit {
                    let p = match v.value().as_str() {
                        "High" => quote! { ::alun::alun_task::TaskPriority::High },
                        "Low" => quote! { ::alun::alun_task::TaskPriority::Low },
                        "Critical" => quote! { ::alun::alun_task::TaskPriority::Critical },
                        _ => quote! { ::alun::alun_task::TaskPriority::Normal },
                    };
                    priority = p;
                }
            }
            "timeout_seconds" => {
                let value = meta.value().unwrap();
                let lit: Lit = value.parse().unwrap();
                if let Lit::Int(v) = lit {
                    timeout_seconds = v.base10_parse().unwrap_or(300);
                }
            }
            "max_retries" => {
                let value = meta.value().unwrap();
                let lit: Lit = value.parse().unwrap();
                if let Lit::Int(v) = lit {
                    max_retries = v.base10_parse().unwrap_or(3);
                }
            }
            "retry_strategy" => {
                let value = meta.value().unwrap();
                let lit: Lit = value.parse().unwrap();
                if let Lit::Str(v) = lit {
                    let s = match v.value().as_str() {
                        "Fixed" => quote! { ::alun::alun_task::RetryStrategy::Fixed },
                        "Exponential" => quote! { ::alun::alun_task::RetryStrategy::Exponential },
                        _ => quote! { ::alun::alun_task::RetryStrategy::Linear },
                    };
                    retry_strategy = s;
                }
            }
            "retry_delay_seconds" => {
                let value = meta.value().unwrap();
                let lit: Lit = value.parse().unwrap();
                if let Lit::Int(v) = lit {
                    retry_delay_seconds = v.base10_parse().unwrap_or(30);
                }
            }
            "max_retry_delay_seconds" => {
                let value = meta.value().unwrap();
                let lit: Lit = value.parse().unwrap();
                if let Lit::Int(v) = lit {
                    max_retry_delay_seconds = v.base10_parse().unwrap_or(300);
                }
            }
            "topic" => {
                let value = meta.value().unwrap();
                let lit: Lit = value.parse().unwrap();
                if let Lit::Str(v) = lit {
                    topic = Some(v.value());
                }
            }
            "description" => {
                let value = meta.value().unwrap();
                let lit: Lit = value.parse().unwrap();
                if let Lit::Str(v) = lit {
                    let s: &'static str = Box::leak(v.value().into_boxed_str());
                    description = s;
                }
            }
            "dead_letter_topic" => {
                let value = meta.value().unwrap();
                let lit: Lit = value.parse().unwrap();
                if let Lit::Str(v) = lit {
                    dead_letter_topic = Some(v.value());
                }
            }
            _ => {}
        }
        Ok(())
    });

    parser.parse(attr).unwrap();

    let task_type_val = task_type.unwrap_or_else(|| {
        panic!(
            "{} 的 #[task_handler] 宏缺少必填参数 task_type",
            struct_name
        )
    });

    let topic_str = topic.unwrap_or_else(|| format!("task_{}", task_type_val));

    let entry_name = quote::format_ident!(
        "__ALUN_TASK_HANDLER_{}",
        struct_name.to_string().to_uppercase()
    );

    let dlq_expr: syn::Expr = if let Some(dlq) = dead_letter_topic {
        syn::parse_quote!(Some(#dlq.to_string()))
    } else {
        syn::parse_quote!(None)
    };

    let expanded = quote! {
        #input

        #[::linkme::distributed_slice(::alun::alun_task::TASK_HANDLERS)]
        static #entry_name: ::alun::alun_task::TaskHandlerEntry = ::alun::alun_task::TaskHandlerEntry {
            task_type: #task_type_val,
            handler_fn: || Box::new(#struct_name),
            config_fn: || ::alun::alun_task::TaskConfig {
                task_type: #task_type_val,
                priority: #priority,
                topic: #topic_str.to_string(),
                timeout_seconds: #timeout_seconds,
                max_retries: #max_retries,
                retry_strategy: #retry_strategy,
                retry_delay_seconds: #retry_delay_seconds,
                max_retry_delay_seconds: #max_retry_delay_seconds,
                description: #description,
                dead_letter_topic: #dlq_expr,
            },
        };
    };

    expanded.into()
}