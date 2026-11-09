//! 搜索后端配置：settings 表读写 + API Key 加密。
//!
//! 键位：
//! - `search_backend`：`auto` | `bing` | `searxng` | `tavily` | `exa` | `brave` | `doubao`
//! - `search_searxng_url`：SearXNG 实例根地址（明文）
//! - `search_tavily_key` / `search_exa_key` / `search_brave_key`：AES-256-GCM 密文
//! - `search_doubao_enabled`：`1`/`0`，是否让豆包参与 `auto` 融合（需已登录）
//!
//! 密钥与 Provider API Key 同源（`crypto::MasterKey`），但独立键位——搜索服务
//! 不属于 Provider 注册表，不参与密钥轮换与用量统计。
//!
//! 豆包凭证不走 settings 表，独立加密文件（见 `doubao::auth`）。

use anyhow::Context;

use crate::crypto::{self, MasterKey};
use crate::db::Database;

use super::Engine;

/// 可选的后端名（`auto` = 已配置后端并发融合）。
pub const BACKENDS: &[&str] = &[
    "auto", "bing", "searxng", "tavily", "exa", "brave", "doubao",
];

/// 豆包网页端对话路径（`search::doubao` 引用）。
pub const DOUBAO_CHAT_PATH: &str = "/chat/completion";

const KEY_TAVILY: &str = "search_tavily_key";
const KEY_EXA: &str = "search_exa_key";
const KEY_BRAVE: &str = "search_brave_key";
const KEY_SEARXNG_URL: &str = "search_searxng_url";
const KEY_DOUBAO_ENABLED: &str = "search_doubao_enabled";

/// 运行时搜索配置（AppState 持有，设置变更后热更新）。
#[derive(Debug, Clone, Default)]
pub struct SearchConfig {
    pub backend: String,
    pub searxng_url: Option<String>,
    pub tavily_key: Option<String>,
    pub exa_key: Option<String>,
    pub brave_key: Option<String>,
    /// 豆包是否参与 `auto` 融合（默认 true，但需实际登录才生效）
    pub doubao_enabled: bool,
    /// 豆包网关根地址
    pub doubao_base_url: String,
    /// 豆包网页端 aid
    pub doubao_aid: String,
    /// 豆包网页端版本参数
    pub doubao_pc_version: String,
    /// 豆包 bot id
    pub doubao_bot_id: String,
    /// 搜索使用的模型别名：`doubao-fast`（快速，会触发联网）/
    /// `doubao-expert`（专家，实测不触发联网，仅作答）
    pub doubao_model: String,
}

impl SearchConfig {
    /// 从 settings 表加载（密钥解密失败视为未配置，不阻塞启动）。
    pub fn load(db: &Database, master: &MasterKey) -> Self {
        let get = |key: &str| db.get_setting(key).ok().flatten();
        let backend = get("search_backend")
            .filter(|v| BACKENDS.contains(&v.as_str()))
            .unwrap_or_else(|| "bing".to_string());
        // 豆包默认不参与：需登录，未登录时不应挤掉 Bing 兜底。
        // （`auto` 后端已从 UI 移除，但仍作为历史值兼容融合执行。）
        let doubao_enabled = get(KEY_DOUBAO_ENABLED)
            .map(|v| v.trim() == "1" || v.trim() == "true")
            .unwrap_or(false);
        Self {
            backend,
            searxng_url: get(KEY_SEARXNG_URL)
                .map(|v| v.trim().trim_end_matches('/').to_string())
                .filter(|v| !v.is_empty()),
            tavily_key: decrypt_setting(db, KEY_TAVILY, master),
            exa_key: decrypt_setting(db, KEY_EXA, master),
            brave_key: decrypt_setting(db, KEY_BRAVE, master),
            doubao_enabled,
            doubao_base_url: get("search_doubao_base_url")
                .map(|v| v.trim().trim_end_matches('/').to_string())
                .filter(|v| !v.is_empty())
                .unwrap_or_else(|| "https://www.doubao.com".to_string()),
            doubao_aid: get("search_doubao_aid")
                .map(|v| v.trim().to_string())
                .filter(|v| !v.is_empty())
                .unwrap_or_else(|| "497858".to_string()),
            doubao_pc_version: get("search_doubao_pc_version")
                .map(|v| v.trim().to_string())
                .filter(|v| !v.is_empty())
                .unwrap_or_else(|| "3.44.8".to_string()),
            doubao_bot_id: get("search_doubao_bot_id")
                .map(|v| v.trim().to_string())
                .filter(|v| !v.is_empty())
                .unwrap_or_else(|| "7338286299411103781".to_string()),
            doubao_model: get("search_doubao_model")
                .map(|v| v.trim().to_string())
                .filter(|v| !v.is_empty())
                // 实测：快速模式才会触发联网（专家模式只答不搜）
                .unwrap_or_else(|| "doubao-fast".to_string()),
        }
    }

    /// 设置后端（非法值报错，避免静默回退）。
    pub fn set_backend(&mut self, value: &str) -> anyhow::Result<()> {
        let value = value.trim().to_ascii_lowercase();
        if !BACKENDS.contains(&value.as_str()) {
            anyhow::bail!("未知搜索后端：{value}（可选：{}）", BACKENDS.join("/"));
        }
        self.backend = value;
        Ok(())
    }

    /// 设置 SearXNG 地址（空串 = 清除）。
    pub fn set_searxng_url(&mut self, value: &str) {
        let trimmed = value.trim().trim_end_matches('/');
        self.searxng_url = if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_string())
        };
    }

    /// 设置豆包是否参与自动融合。
    pub fn set_doubao_enabled(&mut self, enabled: bool) {
        self.doubao_enabled = enabled;
    }

    /// 设置某后端的 API Key（空串 = 清除）。
    pub fn set_key(&mut self, engine: Engine, value: &str) -> anyhow::Result<()> {
        let trimmed = value.trim();
        let slot = match engine {
            Engine::Tavily => &mut self.tavily_key,
            Engine::Exa => &mut self.exa_key,
            Engine::Brave => &mut self.brave_key,
            Engine::Searxng | Engine::Bing | Engine::Doubao => {
                anyhow::bail!("{} 不需要 API Key", engine.name())
            }
        };
        *slot = if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_string())
        };
        Ok(())
    }

    /// 是否已配置该后端。豆包的「已配置」由登录凭证单独判断
    /// （见 `SearchCtx::doubao_credential`），这里返回开关值。
    pub fn has(&self, engine: Engine) -> bool {
        match engine {
            Engine::Bing => true,
            Engine::Searxng => self.searxng_url.is_some(),
            Engine::Tavily => self.tavily_key.is_some(),
            Engine::Exa => self.exa_key.is_some(),
            Engine::Brave => self.brave_key.is_some(),
            Engine::Doubao => self.doubao_enabled,
        }
    }

    /// 生效来源（设置页展示用）：`auto` 展开为实际计划。
    pub fn effective(&self) -> Vec<&'static str> {
        self.plan().iter().map(|e| e.name()).collect()
    }
}

fn decrypt_setting(db: &Database, key: &str, master: &MasterKey) -> Option<String> {
    let blob = db.get_setting(key).ok().flatten()?;
    if blob.trim().is_empty() {
        return None;
    }
    match crypto::decrypt(&blob, master) {
        Ok(plain) if !plain.trim().is_empty() => Some(plain),
        Ok(_) => None,
        Err(e) => {
            tracing::warn!(key, error = %e, "search: failed to decrypt api key");
            None
        }
    }
}

/// 持久化：后端名。
pub fn persist_backend(db: &Database, value: &str) -> anyhow::Result<()> {
    db.set_setting("search_backend", value)
        .context("写入 search_backend 失败")
}

/// 持久化：SearXNG 地址（空串清除）。
pub fn persist_searxng_url(db: &Database, value: &str) -> anyhow::Result<()> {
    db.set_setting(KEY_SEARXNG_URL, value.trim())
        .context("写入 search_searxng_url 失败")
}

/// 持久化：豆包是否参与自动融合（`"1"` / `"0"`）。
pub fn persist_doubao_enabled(db: &Database, enabled: bool) -> anyhow::Result<()> {
    db.set_setting(KEY_DOUBAO_ENABLED, if enabled { "1" } else { "0" })
        .context("写入 search_doubao_enabled 失败")
}

/// 持久化：后端 API Key（空串清除，否则加密存储）。
pub fn persist_key(
    db: &Database,
    master: &MasterKey,
    engine: Engine,
    plain: &str,
) -> anyhow::Result<()> {
    let setting = match engine {
        Engine::Tavily => KEY_TAVILY,
        Engine::Exa => KEY_EXA,
        Engine::Brave => KEY_BRAVE,
        Engine::Searxng | Engine::Bing | Engine::Doubao => {
            anyhow::bail!("{} 不需要 API Key", engine.name())
        }
    };
    let value = if plain.trim().is_empty() {
        String::new()
    } else {
        crypto::encrypt(plain.trim(), master)?
    };
    db.set_setting(setting, &value)
        .context("写入搜索 API Key 失败")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::crypto::load_or_create_master_key;

    fn master_key(dir: &std::path::Path) -> MasterKey {
        load_or_create_master_key(&dir.join("master.key")).unwrap()
    }

    #[test]
    fn test_load_defaults_and_roundtrip() {
        let db = Database::open_in_memory().unwrap();
        db.migrate().unwrap();
        let tmp = tempdir();
        let master = master_key(&tmp);

        let cfg = SearchConfig::load(&db, &master);
        assert_eq!(cfg.backend, "bing");
        assert!(cfg.tavily_key.is_none());
        assert_eq!(cfg.effective(), vec!["bing"]);

        persist_backend(&db, "tavily").unwrap();
        persist_key(&db, &master, Engine::Tavily, "tvly-secret").unwrap();
        persist_searxng_url(&db, "https://search.example.com/").unwrap();

        let cfg = SearchConfig::load(&db, &master);
        assert_eq!(cfg.backend, "tavily");
        assert_eq!(cfg.tavily_key.as_deref(), Some("tvly-secret"));
        assert_eq!(
            cfg.searxng_url.as_deref(),
            Some("https://search.example.com")
        );

        // 清除
        persist_key(&db, &master, Engine::Tavily, "").unwrap();
        let cfg = SearchConfig::load(&db, &master);
        assert!(cfg.tavily_key.is_none());
    }

    #[test]
    fn test_set_backend_rejects_unknown() {
        let mut cfg = SearchConfig::default();
        assert!(cfg.set_backend("google").is_err());
        cfg.set_backend("BRAVE").unwrap();
        assert_eq!(cfg.backend, "brave");
    }

    #[test]
    fn test_set_key_on_keyless_engine_errors() {
        let mut cfg = SearchConfig::default();
        assert!(cfg.set_key(Engine::Bing, "x").is_err());
        assert!(cfg.set_key(Engine::Searxng, "x").is_err());
        cfg.set_key(Engine::Exa, " exa-key ").unwrap();
        assert_eq!(cfg.exa_key.as_deref(), Some("exa-key"));
        cfg.set_key(Engine::Exa, "").unwrap();
        assert!(cfg.exa_key.is_none());
    }

    fn tempdir() -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "xrl-search-config-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }
}
