mod backend;
mod local;
mod plugin;
pub mod registry;
pub mod types;

#[cfg(feature = "minio")]
pub mod minio;

pub use backend::StorageBackend;
pub use local::LocalFs;
pub use plugin::FsPlugin;

pub use plugin::FileMeta;
pub use plugin::StoreResult;

// Backward compatibility: re-export types from plugin
// FsBackend removed; use StorageBackend trait + BackendRegistry for multi-backend support