use crate::{HotIndexError, Network, Result};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::Path;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppConfig {
    pub network: Network,
    pub aptos: AptosConfig,
    pub decibel: DecibelConfig,
    pub dataset: DatasetConfig,
    pub storage: StorageConfig,
    pub api: ApiConfig,
    pub bench: BenchConfig,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AptosConfig {
    pub indexer_grpc_data_service_address: Option<String>,
    pub auth_token: Option<String>,
    pub starting_version: u64,
    pub ending_version: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DecibelConfig {
    pub package_address: String,
    pub orderbook_address: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DatasetConfig {
    pub id: String,
    pub root: String,
    pub raw_format: String,
    pub normalized_format: String,
    pub raw_chunk_versions: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StorageConfig {
    pub engine: String,
    pub path: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApiConfig {
    pub bind: String,
    pub max_limit: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BenchConfig {
    pub warmup_seconds: u64,
    pub duration_seconds: u64,
    pub concurrency: usize,
    pub report_path: String,
}

impl AppConfig {
    pub fn from_path(path: impl AsRef<Path>) -> Result<Self> {
        let text = fs::read_to_string(path)?;
        let expanded = expand_env_placeholders(&text)?;
        Ok(serde_yaml::from_str(&expanded)?)
    }
}

fn expand_env_placeholders(input: &str) -> Result<String> {
    let mut output = String::with_capacity(input.len());
    let mut rest = input;

    while let Some(start) = rest.find("${") {
        output.push_str(&rest[..start]);
        let after_start = &rest[start + 2..];
        let end = after_start.find('}').ok_or_else(|| {
            HotIndexError::Config("unterminated environment placeholder".to_string())
        })?;
        let name = &after_start[..end];
        if name.is_empty() {
            return Err(HotIndexError::Config(
                "empty environment placeholder".to_string(),
            ));
        }
        let value = std::env::var(name).map_err(|_| {
            HotIndexError::Config(format!("missing required environment variable: {name}"))
        })?;
        output.push_str(&value);
        rest = &after_start[end + 1..];
    }

    output.push_str(rest);
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::{expand_env_placeholders, AppConfig};
    use crate::Network;
    use std::fs;

    #[test]
    fn loads_local_config_without_env() {
        let config = AppConfig::from_path("../../config/example.local.yaml").unwrap();
        assert_eq!(config.network, Network::Local);
        assert_eq!(config.storage.engine, "memory");
    }

    #[test]
    fn expands_existing_env_var() {
        let expanded = expand_env_placeholders("token: ${PATH}").unwrap();
        assert!(expanded.starts_with("token: "));
        assert!(!expanded.contains("${PATH}"));
    }

    #[test]
    fn fails_on_missing_env_var() {
        let missing = format!(
            "token: ${{DECIBEL_HOTINDEX_MISSING_ENV_{}}}",
            std::process::id()
        );
        assert!(expand_env_placeholders(&missing).is_err());
    }

    #[test]
    fn loads_config_from_temp_file() {
        let path = std::env::temp_dir().join(format!(
            "decibel-hotindex-config-test-{}.yaml",
            std::process::id()
        ));
        fs::write(
            &path,
            r#"
network: local
aptos:
  indexer_grpc_data_service_address: null
  auth_token: null
  starting_version: 0
  ending_version: null
decibel:
  package_address: "0x0"
  orderbook_address: "0x0"
dataset:
  id: "synthetic"
  root: "./datasets/fixtures/synthetic_smoke"
  raw_format: "synthetic"
  normalized_format: "ndjson"
storage:
  engine: "memory"
  path: "./data/test"
api:
  bind: "127.0.0.1:8080"
  max_limit: 100
bench:
  warmup_seconds: 1
  duration_seconds: 2
  concurrency: 1
  report_path: "./reports/test.json"
"#,
        )
        .unwrap();
        let config = AppConfig::from_path(&path).unwrap();
        fs::remove_file(path).unwrap();
        assert_eq!(config.dataset.id, "synthetic");
    }
}
