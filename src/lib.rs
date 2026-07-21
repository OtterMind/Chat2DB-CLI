pub mod app_plist;
pub mod background;
pub mod cli;
pub mod completions;
pub mod db;
pub mod direct;
pub mod download;
pub mod dry_run;
pub mod edition;
pub mod error;
pub mod frontend_store;
pub mod http;
pub mod installer;
pub mod login;
pub mod logout;
pub mod logs;
pub mod manifest;
pub mod mcp;
pub mod mcp_client;
pub mod mcp_config;
pub mod output;
pub mod platform;
pub mod process;
pub mod restart;
pub mod runtime;
pub mod runtime_client;
pub mod self_update;
pub mod skill;
pub mod start;
pub mod status;
pub mod stop;
pub mod stop_all;
pub mod update_check;

#[cfg(test)]
pub(crate) mod test_support;

pub use error::{Chat2DbError, Result};
