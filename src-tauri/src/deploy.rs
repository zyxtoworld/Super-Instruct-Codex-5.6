// Deploy 模块 — Codex config.toml 备份/修改/恢复

use regex::Regex;
use std::fs;
use std::path::{Path, PathBuf};

pub struct DeployManager {
    codex_home: PathBuf,
}

#[derive(Clone)]
pub struct DeployStatus {
    pub bridge_active: bool,
    pub bridge_exists: bool,
    pub skills_count: usize,
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
        let cfg = self.codex_home.join("config.toml");
        let bak = self.codex_home.join("config.toml.super-instruct-bak");

        tracing::info!("deploy: codex_home = {}", self.codex_home.display());

        // 1. 备份
        if !bak.exists() {
            fs::copy(&cfg, &bak).map_err(|e| format!("backup failed: {}", e))?;
            tracing::info!("deploy: backed up config.toml -> config.toml.super-instruct-bak");
        } else {
            tracing::debug!("deploy: backup already exists, skipping");
        }

        // 2. 修改 base_url + 补入 model_instructions_file
        let content = fs::read_to_string(&cfg).map_err(|e| format!("read config failed: {}", e))?;
        let re = Regex::new(r#"base_url\s*=\s*"[^"]*""#).unwrap();
        let modified = re.replace_all(&content, r#"base_url = "http://127.0.0.1:8080/v1""#);

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

        // 3. 复制 bridge.md
        let dst_bridge = self.codex_home.join("bridge.md");
        fs::write(&dst_bridge, bridge_md).map_err(|e| format!("write bridge.md failed: {}", e))?;
        tracing::info!("deploy: bridge.md written ({} bytes)", bridge_md.len());

        // 4. 复制 skills
        let dst_skills = self.codex_home.join("skills");
        if dst_skills.exists() {
            fs::remove_dir_all(&dst_skills).map_err(|e| format!("remove old skills failed: {}", e))?;
        }
        copy_dir_recursive(skills_dir, &dst_skills)
            .map_err(|e| format!("copy skills failed: {}", e))?;

        let skill_count = count_skills(&dst_skills);
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

    pub fn status(&self) -> DeployStatus {
        let cfg = self.codex_home.join("config.toml");
        let bridge = self.codex_home.join("bridge.md");
        let skills = self.codex_home.join("skills");

        let bridge_active = cfg.exists() && {
            let content = fs::read_to_string(&cfg).unwrap_or_default();
            content.contains("127.0.0.1:8080")
        };

        DeployStatus {
            bridge_active,
            bridge_exists: bridge.exists(),
            skills_count: if skills.exists() {
                count_skills(&skills)
            } else {
                0
            },
        }
    }
}

/// 读取 Codex 配置中的中转站地址 (优先从备份读取原始地址)
pub fn find_relay_url() -> Option<String> {
    let home = DeployManager::find_codex_home()?;
    let cfg = home.join("config.toml");
    let bak = home.join("config.toml.super-instruct-bak");

    let cfg_to_read = if bak.exists() { &bak } else { &cfg };
    let content = fs::read_to_string(cfg_to_read).ok()?;
    let re = Regex::new(r#"base_url\s*=\s*"([^"]+)""#).ok()?;
    re.captures(&content)
        .and_then(|c| c.get(1).map(|m| m.as_str().to_string()))
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