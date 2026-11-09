//! 豆包登录凭证：加密落盘 + 运行时读取。
//!
//! 存储位置与 Provider 主密钥同源数据目录，但独立文件 `doubao-credential.json`。
//! Cookie 整体用 `crypto::encrypt`（AES-256-GCM + 主密钥）加密后 base64 落盘，
//! 明文不落盘、不进日志。

use anyhow::{anyhow, Context};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

use crate::crypto::{self, MasterKey};

use super::protocol::Device;

/// 落盘结构（cookies 已加密）。
#[derive(Debug, Serialize, Deserialize)]
struct StoredCredential {
    version: u32,
    /// base64(AES-256-GCM(cookie_header))
    cookies_enc: String,
    uid: Option<String>,
    nickname: Option<String>,
    device: StoredDevice,
    /// Unix 秒，仅展示用
    saved_at: i64,
}

#[derive(Debug, Serialize, Deserialize)]
struct StoredDevice {
    device_id: String,
    web_id: String,
    fp: String,
}

/// 运行时凭证。
#[derive(Debug, Clone)]
pub struct Credential {
    /// 完整 Cookie 头（`sessionid=...; ttwid=...`）
    pub cookie_header: String,
    pub uid: Option<String>,
    pub nickname: Option<String>,
    pub device: Device,
    pub saved_at: i64,
}

impl Credential {
    /// 从 Cookie 头里提取 `sessionid`（判断登录态是否存在的关键字段）。
    pub fn sessionid(&self) -> Option<String> {
        extract_cookie(&self.cookie_header, "sessionid")
    }
}

fn extract_cookie(header: &str, name: &str) -> Option<String> {
    for part in header.split(';') {
        let part = part.trim();
        let mut kv = part.splitn(2, '=');
        let (k, v) = (kv.next()?, kv.next()?);
        if k.trim() == name && !v.trim().is_empty() {
            return Some(v.trim().to_string());
        }
    }
    None
}

/// 凭证文件默认路径：与 master.key 同目录。
pub fn default_path(data_dir: &Path) -> PathBuf {
    data_dir.join("doubao-credential.json")
}

/// 读取并解密凭证；文件不存在或解不开都返回 `None`（视为未登录）。
pub fn load(path: &Path, master: &MasterKey) -> Option<Credential> {
    let raw = std::fs::read_to_string(path).ok()?;
    let stored: StoredCredential = serde_json::from_str(&raw).ok()?;
    let cookie_header = crypto::decrypt(&stored.cookies_enc, master).ok()?;
    if cookie_header.trim().is_empty() {
        return None;
    }
    Some(Credential {
        cookie_header,
        uid: stored.uid,
        nickname: stored.nickname,
        device: Device {
            device_id: stored.device.device_id,
            web_id: stored.device.web_id,
            fp: stored.device.fp,
        },
        saved_at: stored.saved_at,
    })
}

/// 加密保存凭证（原子写入：先写临时文件再 rename）。
pub fn save(
    path: &Path,
    master: &MasterKey,
    cookie_header: &str,
    uid: Option<String>,
    nickname: Option<String>,
    device: &Device,
) -> anyhow::Result<()> {
    let cookies_enc =
        crypto::encrypt(cookie_header.trim(), master).context("加密豆包 Cookie 失败")?;
    let stored = StoredCredential {
        version: 1,
        cookies_enc,
        uid,
        nickname,
        device: StoredDevice {
            device_id: device.device_id.clone(),
            web_id: device.web_id.clone(),
            fp: device.fp.clone(),
        },
        saved_at: chrono::Utc::now().timestamp(),
    };
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).context("创建凭证目录失败")?;
    }
    let tmp = path.with_extension("json.tmp");
    let body = serde_json::to_string_pretty(&stored).context("序列化凭证失败")?;
    std::fs::write(&tmp, body).context("写入临时凭证失败")?;
    std::fs::rename(&tmp, path).map_err(|e| anyhow!("替换凭证文件失败：{e}"))?;
    Ok(())
}

/// 删除本地凭证（登出）。
pub fn clear(path: &Path) -> anyhow::Result<()> {
    match std::fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(anyhow!("删除凭证失败：{e}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::crypto::load_or_create_master_key;

    fn tmp_dir() -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "xrl-doubao-cred-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn test_roundtrip_encrypted() {
        let dir = tmp_dir();
        let master = load_or_create_master_key(&dir.join("master.key")).unwrap();
        let path = default_path(&dir);
        let device = Device {
            device_id: "1".into(),
            web_id: "2".into(),
            fp: "verify_x".into(),
        };
        save(
            &path,
            &master,
            "sessionid=abc; ttwid=def",
            Some("u1".into()),
            Some("昵称".into()),
            &device,
        )
        .unwrap();

        // 落盘文件不含明文 Cookie
        let raw = std::fs::read_to_string(&path).unwrap();
        assert!(!raw.contains("sessionid=abc"));

        let cred = load(&path, &master).expect("should load");
        assert_eq!(cred.cookie_header, "sessionid=abc; ttwid=def");
        assert_eq!(cred.sessionid().as_deref(), Some("abc"));
        assert_eq!(cred.nickname.as_deref(), Some("昵称"));

        // 错误主密钥解不开
        let other = [9u8; 32];
        assert!(load(&path, &other).is_none());

        clear(&path).unwrap();
        assert!(load(&path, &master).is_none());
    }

    #[test]
    fn test_extract_cookie() {
        assert_eq!(
            extract_cookie("a=1; sessionid=xyz; b=2", "sessionid").as_deref(),
            Some("xyz")
        );
        assert_eq!(extract_cookie("a=1", "sessionid"), None);
    }
}
