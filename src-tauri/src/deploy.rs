// Deploy 模块 — Codex config.toml 备份/修改/恢复

use regex::Regex;
use serde::Serialize;
use std::fs;
use std::path::{Path, PathBuf};

pub struct DeployManager {
    codex_home: PathBuf,
}

#[derive(Clone, Serialize)]
pub struct DeployStatus {
    pub bridge_active: bool,
    pub bridge_exists: bool,
    pub skills_count: usize,
    pub config_backed_up: bool,
    pub relay_url_valid: bool,
    pub codex_home_found: bool,
}

fn home_dir() -> Option<PathBuf> {
    std::env::var_os("USERPROFILE")
        .or_else(|| std::env::var_os("HOME"))
        .map(PathBuf::from)
}

impl DeployManager {
    /// 查找 Codex 配置目录
    pub fn find_codex_home() -> Option<PathBuf> {
        // CODEX_HOME 环境变量
        if let Ok(home) = std::env::var("CODEX_HOME") {
            let p = PathBuf::from(home);
            if p.join("config.toml").exists() {
                return Some(p);
            }
        }
        // ~/.codex
        if let Some(home) = home_dir() {
            let codex = home.join(".codex");
            if codex.join("config.toml").exists() {
                return Some(codex);
            }
        }
        None
    }

    pub fn new() -> Option<Self> {
        Self::find_codex_home().map(|codex_home| Self { codex_home })
    }

    pub fn codex_home(&self) -> &Path {
        &self.codex_home
    }

    /// 部署 bridge.md + skills 到 Codex，修改 base_url 指向代理
    pub fn apply(&self, bridge_md: &str, skills_dir: &Path) -> Result<String, String> {
        self.apply_with_optional_skills(bridge_md, Some(skills_dir))
    }

    /// 部署 bridge.md 到 Codex，skills 可选。修改 base_url 指向代理
    pub fn apply_with_optional_skills(
        &self,
        bridge_md: &str,
        skills_dir: Option<&Path>,
    ) -> Result<String, String> {
        let cfg = self.codex_home.join("config.toml");
        let bak = self.codex_home.join("config.toml.super-instruct-bak");
        let relay_file = self.codex_home.join("relay_url.txt");

        tracing::info!("deploy: codex_home = {}", self.codex_home.display());

        // 1. 读取当前 config.toml，提取 base_url
        let content = fs::read_to_string(&cfg).map_err(|e| format!("read config failed: {}", e))?;
        let re = Regex::new(r#"base_url\s*=\s*"([^"]+)""#).unwrap();

        // 2. 保存真实中转站地址到 relay_url.txt（只要当前 base_url 不是代理地址）
        if let Some(caps) = re.captures(&content) {
            let current_url = caps.get(1).map(|m| m.as_str()).unwrap_or("");
            if !current_url.contains("127.0.0.1:8080") {
                fs::write(&relay_file, current_url)
                    .map_err(|e| format!("write relay_url.txt failed: {}", e))?;
                tracing::info!("deploy: relay_url.txt saved: {}", current_url);
            }
        }

        // 3. 备份 config.toml（每次都刷新，确保备份的是未修改版本）
        fs::copy(&cfg, &bak).map_err(|e| format!("backup failed: {}", e))?;
        tracing::info!("deploy: backed up config.toml -> config.toml.super-instruct-bak");

        // 4. 修改 base_url + 补入 model_instructions_file
        let modified = re.replace_all(&content, r#"base_url = "http://127.0.0.1:8080""#);

        // model_instructions_file: 若已存在则替换，否则在 model = 行后插入，都没有则追加
        let instructions_line = r#"model_instructions_file = "./bridge.md""#;
        let final_config = if modified.contains("model_instructions_file") {
            let re2 = Regex::new(r#"model_instructions_file\s*=\s*"[^"]*""#).unwrap();
            re2.replace_all(&modified, instructions_line).into_owned()
        } else if modified.contains("model") && modified.lines().any(|l| l.trim_start().starts_with("model")) {
            // 在 model = 行之后插入
            let mut lines = modified.lines().collect::<Vec<_>>();
            let mut inserted = false;
            for i in 0..lines.len() {
                if lines[i].trim_start().starts_with("model") {
                    lines.insert(i + 1, instructions_line);
                    inserted = true;
                    break;
                }
            }
            if inserted {
                lines.join("\n")
            } else {
                format!("{}\n{}", modified, instructions_line)
            }
        } else {
            format!("{}\n{}", modified, instructions_line)
        };

        fs::write(&cfg, &final_config).map_err(|e| format!("write config failed: {}", e))?;
        tracing::info!("deploy: base_url patched + model_instructions_file set");

        // 5. 复制 bridge.md
        let dst_bridge = self.codex_home.join("bridge.md");
        fs::write(&dst_bridge, bridge_md).map_err(|e| format!("write bridge.md failed: {}", e))?;
        tracing::info!("deploy: bridge.md written ({} bytes)", bridge_md.len());

        // 6. 复制 skills (可选)
        let skill_count = if let Some(skills_dir) = skills_dir {
            let dst_skills = self.codex_home.join("skills");
            if dst_skills.exists() {
                fs::remove_dir_all(&dst_skills).map_err(|e| format!("remove old skills failed: {}", e))?;
            }
            copy_dir_recursive(skills_dir, &dst_skills)
                .map_err(|e| format!("copy skills failed: {}", e))?;
            count_skills(&dst_skills)
        } else {
            tracing::warn!("deploy: skills dir not provided, skipping skills");
            0
        };
        tracing::info!("deploy: {} skills deployed", skill_count);
        Ok(format!("bridge.md + {} skills deployed", skill_count))
    }

    /// 从备份恢复 Codex 配置
    pub fn restore(&self) -> Result<String, String> {
        let cfg = self.codex_home.join("config.toml");
        let bak = self.codex_home.join("config.toml.super-instruct-bak");

        tracing::info!("restore: codex_home = {}", self.codex_home.display());

        if bak.exists() {
            fs::copy(&bak, &cfg).map_err(|e| format!("restore config failed: {}", e))?;
            fs::remove_file(&bak).map_err(|e| format!("remove backup failed: {}", e))?;
            tracing::info!("restore: config.toml restored from backup");
        } else {
            tracing::warn!("restore: no backup found, config.toml unchanged");
        }

        let bridge = self.codex_home.join("bridge.md");
        if bridge.exists() {
            let _ = fs::remove_file(&bridge);
            tracing::info!("restore: bridge.md removed");
        }

        let skills = self.codex_home.join("skills");
        if skills.exists() {
            let _ = fs::remove_dir_all(&skills);
            tracing::info!("restore: skills/ removed");
        }

        Ok("Codex config restored".to_string())
    }

    /// 设置中转站地址（写入 relay_url.txt，如果 config.toml 存在则同步更新）
    pub fn set_relay_url(&self, url: &str) -> Result<String, String> {
        let relay_file = self.codex_home.join("relay_url.txt");
        fs::write(&relay_file, url).map_err(|e| format!("write relay_url.txt failed: {}", e))?;
        tracing::info!("set_relay_url: relay_url.txt saved: {}", url);

        // 如果 config.toml 存在且当前 base_url 不是代理地址，同步更新
        let cfg = self.codex_home.join("config.toml");
        if cfg.exists() {
            let content = fs::read_to_string(&cfg).map_err(|e| format!("read config failed: {}", e))?;
            // 只有当 base_url 不指向本地代理时才同步（避免覆盖正在运行的代理配置）
            if !content.contains("127.0.0.1:8080") {
                let re = Regex::new(r#"base_url\s*=\s*"[^"]*""#).unwrap();
                let modified = re.replace_all(&content, format!(r#"base_url = "{}""#, url));
                fs::write(&cfg, modified.as_ref()).map_err(|e| format!("write config failed: {}", e))?;
                tracing::info!("set_relay_url: config.toml base_url updated to {}", url);
            } else {
                tracing::info!("set_relay_url: proxy active, config.toml not modified");
            }
        }

        Ok(format!("Relay URL saved: {}", url))
    }

    pub fn status(&self) -> DeployStatus {
        let cfg = self.codex_home.join("config.toml");
        let bridge = self.codex_home.join("bridge.md");
        let skills = self.codex_home.join("skills");
        let bak = self.codex_home.join("config.toml.super-instruct-bak");
        let relay_file = self.codex_home.join("relay_url.txt");

        let bridge_active = cfg.exists() && {
            let content = fs::read_to_string(&cfg).unwrap_or_default();
            content.contains("127.0.0.1:8080")
        };

        let relay_url_valid = relay_file.exists() && {
            let content = fs::read_to_string(&relay_file).unwrap_or_default();
            let url = content.trim();
            !url.is_empty() && !url.contains("127.0.0.1:8080")
        };

        DeployStatus {
            bridge_active,
            bridge_exists: bridge.exists(),
            skills_count: if skills.exists() {
                count_skills(&skills)
            } else {
                0
            },
            config_backed_up: bak.exists(),
            relay_url_valid,
            codex_home_found: true,
        }
    }
}

/// 读取中转站地址（优先级: relay_url.txt > config.toml 备份 > config.toml 当前）
pub fn find_relay_url() -> Option<String> {
    let home = DeployManager::find_codex_home()?;

    // 1. 优先读 relay_url.txt（用户显式设置的，或部署时自动保存的）
    let relay_file = home.join("relay_url.txt");
    if relay_file.exists() {
        if let Ok(content) = fs::read_to_string(&relay_file) {
            let url = content.trim();
            if !url.is_empty() && !url.contains("127.0.0.1:8080") {
                return Some(url.to_string());
            }
        }
    }

    // 2. 从 config.toml 备份读取（部署前的原始地址）
    let bak = home.join("config.toml.super-instruct-bak");
    if bak.exists() {
        if let Ok(content) = fs::read_to_string(&bak) {
            let re = Regex::new(r#"base_url\s*=\s*"([^"]+)""#).ok()?;
            if let Some(caps) = re.captures(&content) {
                let url = caps.get(1).map(|m| m.as_str()).unwrap_or("");
                if !url.contains("127.0.0.1:8080") {
                    return Some(url.to_string());
                }
            }
        }
    }

    // 3. 从当前 config.toml 读取（排除代理自身地址，防自环）
    let cfg = home.join("config.toml");
    if let Ok(content) = fs::read_to_string(&cfg) {
        let re = Regex::new(r#"base_url\s*=\s*"([^"]+)""#).ok()?;
        if let Some(caps) = re.captures(&content) {
            let url = caps.get(1).map(|m| m.as_str()).unwrap_or("");
            if !url.is_empty() && !url.contains("127.0.0.1:8080") {
                return Some(url.to_string());
            }
        }
    }

    None
}

fn copy_dir_recursive(src: &Path, dst: &Path) -> std::io::Result<()> {
    fs::create_dir_all(dst)?;
    for entry in fs::read_dir(src)? {
        let entry = entry?;
        let path = entry.path();
        let dest = dst.join(entry.file_name());
        if path.is_dir() {
            copy_dir_recursive(&path, &dest)?;
        } else {
            fs::copy(&path, &dest)?;
        }
    }
    Ok(())
}

fn count_skills(dir: &Path) -> usize {
    let mut count = 0;
    if let Ok(entries) = fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                count += count_skills(&path);
            } else if path.file_name().map(|n| n == "SKILL.md").unwrap_or(false) {
                count += 1;
            }
        }
    }
    count
}