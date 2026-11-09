//! 插件市场：从 GitHub 搜索并安装 xrp-* 反代插件。
//!
//! 安全边界（硬编码，**不接受前端传入任意 URL/仓库**）：
//! - 只能安装在 `wpy030414` 名下的仓库
//! - 仓库名必须匹配 `^xrp-[a-z0-9-]+$`
//!
//! 安装动作 = `git clone --depth 1`（已存在则 fetch + reset）+ `pnpm install`，
//! 落地目录 `{data_dir}/plugins/{repo}`，随后由 handler 调用 `spawn_serve` 拉起。

use std::path::Path;
use std::process::Stdio;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use anyhow::{bail, Context};
use serde::{Deserialize, Serialize};

pub const REPO_OWNER: &str = "wpy030414";
const REPO_PREFIX: &str = "xrp-";
/// GitHub 未认证搜索限流较严（10 次/分钟），搜索结果做短缓存
const CACHE_TTL: Duration = Duration::from_secs(30);
const CLONE_TIMEOUT: Duration = Duration::from_secs(180);
const INSTALL_TIMEOUT: Duration = Duration::from_secs(600);
/// 登录是人工环节（扫码/账密），给足 10 分钟（插件侧默认 5 分钟窗口）
const LOGIN_TIMEOUT: Duration = Duration::from_secs(600);

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MarketPlugin {
    /// 仓库名，如 `xrp-workbuddy`
    pub repo: String,
    /// 展示名（去掉 xrp- 前缀）
    pub display_name: String,
    pub description: String,
    pub html_url: String,
    pub pushed_at: String,
    pub default_branch: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct InstallOutcome {
    pub repo: String,
    pub work_dir: String,
}

#[derive(Deserialize)]
struct SearchResponse {
    #[serde(default)]
    items: Vec<SearchItem>,
}

#[derive(Deserialize)]
struct SearchItem {
    name: String,
    full_name: String,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    html_url: String,
    #[serde(default)]
    pushed_at: Option<String>,
    #[serde(default)]
    default_branch: Option<String>,
}

static CACHE: OnceLock<Mutex<Option<(Instant, Vec<MarketPlugin>)>>> = OnceLock::new();

fn cache() -> &'static Mutex<Option<(Instant, Vec<MarketPlugin>)>> {
    CACHE.get_or_init(|| Mutex::new(None))
}

/// 仓库名白名单校验（`xrp-` + 小写字母/数字/连字符）
pub fn is_allowed_repo(repo: &str) -> bool {
    let Some(rest) = repo.strip_prefix(REPO_PREFIX) else {
        return false;
    };
    !rest.is_empty()
        && rest.len() <= 40
        && rest
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

/// 搜索 `owner:wpy030414 xrp-`。`force` 为 true 时跳过 30s 缓存。
pub async fn search(client: &reqwest::Client, force: bool) -> anyhow::Result<Vec<MarketPlugin>> {
    if !force {
        if let Ok(guard) = cache().lock() {
            if let Some((at, list)) = guard.as_ref() {
                if at.elapsed() < CACHE_TTL {
                    return Ok(list.clone());
                }
            }
        }
    }

    let res = client
        .get("https://api.github.com/search/repositories")
        .header("Accept", "application/vnd.github+json")
        .header("User-Agent", "xrl-router")
        .query(&[
            ("q", format!("owner:{REPO_OWNER} {REPO_PREFIX}")),
            ("sort", "updated".to_string()),
            ("order", "desc".to_string()),
            ("per_page", "50".to_string()),
        ])
        .timeout(Duration::from_secs(20))
        .send()
        .await
        .context("GitHub 搜索请求失败")?;

    if !res.status().is_success() {
        bail!("GitHub 搜索失败：HTTP {}", res.status());
    }

    let body: SearchResponse = res.json().await.context("GitHub 响应解析失败")?;
    let owner_prefix = format!("{REPO_OWNER}/");
    let mut list: Vec<MarketPlugin> = body
        .items
        .into_iter()
        .filter(|i| {
            i.full_name.starts_with(&owner_prefix) && i.name.starts_with(REPO_PREFIX) && is_allowed_repo(&i.name)
        })
        .map(|i| MarketPlugin {
            display_name: i.name.trim_start_matches(REPO_PREFIX).to_string(),
            description: i.description.unwrap_or_default(),
            html_url: i.html_url,
            pushed_at: i.pushed_at.unwrap_or_default(),
            default_branch: i.default_branch.unwrap_or_else(|| "main".to_string()),
            repo: i.name,
        })
        .collect();
    // 仓库数量很少，按名字排序让列表稳定（不受 GitHub 更新时间抖动影响）
    list.sort_by(|a, b| a.repo.cmp(&b.repo));

    if let Ok(mut guard) = cache().lock() {
        *guard = Some((Instant::now(), list.clone()));
    }
    Ok(list)
}

/// 下载并安装插件：clone/fetch → pnpm install。
pub async fn install(data_dir: &Path, repo: &str) -> anyhow::Result<InstallOutcome> {
    let repo = repo.trim();
    if !is_allowed_repo(repo) {
        bail!("非法仓库名：{repo}");
    }

    let root = data_dir.join("plugins");
    tokio::fs::create_dir_all(&root).await.context("创建插件目录失败")?;
    let dest = root.join(repo);
    let dest_str = dest.to_string_lossy().to_string();
    let url = format!("https://github.com/{REPO_OWNER}/{repo}.git");
    // git/pnpm 不读网关 reqwest 的代理配置，环境变量需显式注入，否则 GitHub 直连超时
    let proxy = proxy_envs();

    if dest.join(".git").is_dir() {
        // 已装过：拉最新代码（浅克隆，FETCH_HEAD 即远端默认分支头）
        run("git", &["-C", &dest_str, "fetch", "--depth", "1", "origin"], None, &proxy, CLONE_TIMEOUT).await?;
        run("git", &["-C", &dest_str, "reset", "--hard", "FETCH_HEAD"], None, &proxy, CLONE_TIMEOUT).await?;
    } else {
        if dest.exists() {
            tokio::fs::remove_dir_all(&dest).await.context("清理残留目录失败")?;
        }
        run("git", &["clone", "--depth", "1", &url, &dest_str], None, &proxy, CLONE_TIMEOUT).await?;
    }

    if !dest.join("package.json").is_file() {
        bail!("{repo} 不是有效的插件仓库（缺少 package.json）");
    }

    let pnpm = crate::plugin::host::pnpm_bin();
    run(
        pnpm,
        &["install", "--prefer-offline", "--reporter=append-only"],
        Some(&dest),
        &proxy,
        INSTALL_TIMEOUT,
    )
    .await
    .with_context(|| format!("{repo} 依赖安装失败"))?;

    Ok(InstallOutcome {
        repo: repo.to_string(),
        work_dir: dest_str,
    })
}

/// 系统代理的环境变量形式（`https_proxy` 等），供 git / pnpm / 登录脚本继承。
///
/// 统一取 `http::system_proxy()`：网关内所有出站请求同源，用户只在一处配代理。
/// 已存在的环境变量不覆盖（`resolve_system_proxy` 本就优先读环境变量）。
fn proxy_envs() -> Vec<(String, String)> {
    let Some(proxy) = crate::http::system_proxy() else {
        return Vec::new();
    };
    ["HTTPS_PROXY", "https_proxy", "HTTP_PROXY", "http_proxy", "ALL_PROXY", "all_proxy"]
        .into_iter()
        .filter(|k| std::env::var_os(k).is_none())
        .map(|k| (k.to_string(), proxy.to_string()))
        .collect()
}

/// 登录脚本探测统一走 `host`（市场安装与「插件菜单 → 登录」共用同一套判定）
pub use crate::plugin::host::detect_login_script;

/// 跑 `pnpm run <login|log-in>`，阻塞到登录完成后才返回。
///
/// 必须在 `pnpm run serve` 之前执行：插件没有凭证时网关照常监听、但所有推理请求都会 401。
pub async fn login(work_dir: &Path) -> anyhow::Result<String> {
    let script = detect_login_script(work_dir)
        .ok_or_else(|| anyhow::anyhow!("该插件未提供 login / log-in 脚本，无法完成登录"))?;
    let pnpm = crate::plugin::host::pnpm_bin();
    run(pnpm, &["run", &script], Some(work_dir), &proxy_envs(), LOGIN_TIMEOUT)
        .await
        .with_context(|| format!("登录失败（pnpm run {script}）"))?;
    Ok(script)
}

async fn run(
    cmd: &str,
    args: &[&str],
    cwd: Option<&Path>,
    envs: &[(String, String)],
    timeout: Duration,
) -> anyhow::Result<std::process::Output> {
    let mut c = tokio::process::Command::new(cmd);
    c.args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    for (k, v) in envs {
        c.env(k, v);
    }
    if let Some(dir) = cwd {
        c.current_dir(dir);
    }
    // Windows 下避免弹出控制台窗口（tokio Command 自带该扩展方法）
    #[cfg(windows)]
    {
        c.creation_flags(0x0800_0000);
    }

    let out = tokio::time::timeout(timeout, c.output())
        .await
        .map_err(|_| anyhow::anyhow!("{cmd} 超时（{}s）", timeout.as_secs()))?
        .with_context(|| format!("{cmd} 启动失败（是否已安装？）"))?;

    if !out.status.success() {
        // stderr 为空时回退 stdout：登录脚本常把 authUrl / 失败原因打到 stdout
        let mut detail = tail(&out.stderr);
        if detail.is_empty() {
            detail = tail(&out.stdout);
        }
        bail!("{cmd} 退出码 {}：{}", out.status, detail);
    }
    Ok(out)
}

fn tail(buf: &[u8]) -> String {
    let s = String::from_utf8_lossy(buf);
    let chars: Vec<char> = s.chars().collect();
    let start = chars.len().saturating_sub(600);
    chars[start..].iter().collect::<String>().trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn allow_only_xrp_repos() {
        assert!(is_allowed_repo("xrp-workbuddy"));
        assert!(is_allowed_repo("xrp-doubao"));
        assert!(is_allowed_repo("xrp-a1-b2"));
        assert!(!is_allowed_repo("xrp-"));
        assert!(!is_allowed_repo("workbuddy"));
        assert!(!is_allowed_repo("xrp-WorkBuddy"));
        assert!(!is_allowed_repo("xrp-../etc"));
        assert!(!is_allowed_repo("xrp-work_buddy"));
        assert!(!is_allowed_repo("https://github.com/wpy030414/evil"));
    }
}
