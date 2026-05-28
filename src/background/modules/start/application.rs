use std::path::{Path, PathBuf};
use std::sync::Arc;

use arc_swap::ArcSwap;
use lazy_static::lazy_static;
use libs_core::system_state::StartMenuItem;
use windows::Win32::UI::Shell::{FOLDERID_CommonPrograms, FOLDERID_Programs};

use crate::{error::Result, log_error, utils::constants::VAR_COMMON, windows_api::WindowsApi};

lazy_static! {
    pub static ref START_MENU_MANAGER: ArcSwap<StartMenuManager> = ArcSwap::from_pointee({
        let mut manager = StartMenuManager::new();
        if let Err(e) = manager.init() {
            log::error!("Failed to initialize StartMenuManager: {e}");
        }
        manager
    });
}

pub struct StartMenuManager {
    pub list: Vec<StartMenuItem>,
    cache_path: PathBuf,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StartMenuMatchKind {
    ExactUmid,
    TargetSuffix,
    FuzzyIdentity,
}

impl StartMenuManager {
    /// programs shared by all users
    pub fn common_items_path() -> PathBuf {
        WindowsApi::known_folder(FOLDERID_CommonPrograms)
            .expect("Failed to get common programs folder")
    }

    /// programs specific to the current user
    pub fn user_items_path() -> PathBuf {
        WindowsApi::known_folder(FOLDERID_Programs).expect("Failed to get user programs folder")
    }

    pub fn new() -> StartMenuManager {
        StartMenuManager {
            list: Vec::new(),
            cache_path: VAR_COMMON.app_cache_dir().join("start_menu_v2.json"),
        }
    }

    fn init(&mut self) -> Result<()> {
        if self.cache_path.exists() {
            match self.load_cache() {
                Ok(_) => {
                    // refresh without blocking
                    std::thread::spawn(|| {
                        let mut menu = StartMenuManager::new();
                        log_error!(menu.read_start_menu_folders());
                        log_error!(menu.store_cache());
                        START_MENU_MANAGER.swap(Arc::new(menu));
                    });
                    return Ok(());
                }
                Err(e) => {
                    log::error!("Failed to load start menu cache: {e}");
                }
            }
        }

        self.read_start_menu_folders()?;
        self.store_cache()?;
        Ok(())
    }

    pub fn get_by_target(&self, target: &Path) -> Option<&StartMenuItem> {
        self.list
            .iter()
            .find(|item| item.target.as_ref().is_some_and(|t| t == target))
    }

    /// https://learn.microsoft.com/en-us/windows/win32/properties/props-system-appusermodel-relaunchiconresource
    pub fn get_by_file_umid(&self, umid: &str) -> Option<&StartMenuItem> {
        self.get_by_file_umid_with_match_kind(umid)
            .map(|(item, _)| item)
    }

    /// Returns the matched Start Menu item together with how strong the match is.
    ///
    /// Exact UMID matches are reliable. Target suffix and fuzzy identity matches are
    /// compatibility fallbacks and callers should verify them against process/relaunch
    /// paths before using the shortcut as the window identity.
    pub fn get_by_file_umid_with_match_kind(
        &self,
        umid: &str,
    ) -> Option<(&StartMenuItem, StartMenuMatchKind)> {
        self.get_by_file_umid_candidates(umid)
            .into_iter()
            .max_by_key(|(item, kind)| {
                let match_score = match kind {
                    StartMenuMatchKind::ExactUmid => 1000,
                    StartMenuMatchKind::TargetSuffix => 500,
                    StartMenuMatchKind::FuzzyIdentity => 0,
                };
                match_score + score_start_menu_item_identity(umid, item)
            })
    }

    pub fn get_by_file_umid_candidates(
        &self,
        umid: &str,
    ) -> Vec<(&StartMenuItem, StartMenuMatchKind)> {
        let exact: Vec<_> = self
            .list
            .iter()
            .filter(|item| item.umid.as_deref() == Some(umid))
            .map(|item| (item, StartMenuMatchKind::ExactUmid))
            .collect();
        if !exact.is_empty() {
            return exact;
        }

        let target_suffix: Vec<_> = self
            .list
            .iter()
            .filter(|item| {
                if let Some(target) = &item.target {
                    // some apps registered as media player as example use the process name as umid
                    return target.ends_with(umid);
                }
                false
            })
            .map(|item| (item, StartMenuMatchKind::TargetSuffix))
            .collect();
        if !target_suffix.is_empty() {
            return target_suffix;
        }

        self.get_by_fuzzy_identity(umid)
            .into_iter()
            .map(|item| (item, StartMenuMatchKind::FuzzyIdentity))
            .collect()
    }

    fn get_by_fuzzy_identity(&self, identity: &str) -> Option<&StartMenuItem> {
        let normalized_identity = normalize_identity(identity);
        let identity_tokens = tokenize_identity(identity);

        if normalized_identity.is_empty() || identity_tokens.is_empty() {
            return None;
        }

        self.list
            .iter()
            .filter_map(|item| {
                let mut candidates = Vec::new();

                if let Some(stem) = item.path.file_stem().and_then(|stem| stem.to_str()) {
                    candidates.push(stem.to_string());
                }
                if let Some(target) = &item.target {
                    if let Some(stem) = target.file_stem().and_then(|stem| stem.to_str()) {
                        candidates.push(stem.to_string());
                    }
                }

                let best_score = candidates
                    .into_iter()
                    .map(|candidate| {
                        score_identity_match(&normalized_identity, &identity_tokens, &candidate)
                    })
                    .max()
                    .unwrap_or(0);

                (best_score >= 4).then_some((item, best_score))
            })
            .max_by_key(|(_, score)| *score)
            .map(|(item, _)| item)
    }

    pub fn store_cache(&self) -> Result<()> {
        let file = std::fs::File::create(&self.cache_path)?;
        let writer = std::io::BufWriter::new(file);
        serde_json::to_writer_pretty(writer, &self.list)?;
        Ok(())
    }

    pub fn load_cache(&mut self) -> Result<()> {
        let file = std::fs::File::open(&self.cache_path)?;
        let reader = std::io::BufReader::new(file);
        self.list = serde_json::from_reader(reader)?;
        Ok(())
    }

    fn _get_items(dir: &Path) -> Result<Vec<StartMenuItem>> {
        let mut items = Vec::new();
        let entries = match std::fs::read_dir(dir) {
            Ok(entries) => entries,
            Err(e) => {
                log::warn!("[StartMenu] Cannot read directory {:?}: {e}", dir);
                return Ok(items);
            }
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let file_type = match entry.file_type() {
                Ok(ft) => ft,
                Err(_) => continue,
            };
            if file_type.is_dir() {
                items.extend(Self::_get_items(&path)?);
                continue;
            }
            if file_type.is_file() {
                let target = WindowsApi::resolve_lnk_target(&path).ok().map(|(t, _)| t);
                items.push(StartMenuItem {
                    umid: WindowsApi::get_file_umid(&path).ok(),
                    toast_activator: WindowsApi::get_file_toast_activator(&path).ok(),
                    path,
                    target,
                })
            }
        }
        Ok(items)
    }

    pub fn read_start_menu_folders(&mut self) -> Result<()> {
        let mut items = vec![];
        items.extend(Self::_get_items(&Self::common_items_path())?);
        items.extend(Self::_get_items(&Self::user_items_path())?);
        self.list = items;
        Ok(())
    }
}

fn score_start_menu_item_identity(identity: &str, item: &StartMenuItem) -> usize {
    let normalized_identity = normalize_identity(identity);
    let identity_tokens = tokenize_identity(identity);

    if normalized_identity.is_empty() || identity_tokens.is_empty() {
        return 0;
    }

    let mut candidates = Vec::new();
    if let Some(stem) = item.path.file_stem().and_then(|stem| stem.to_str()) {
        candidates.push(stem.to_string());
    }
    if let Some(target) = &item.target {
        if let Some(stem) = target.file_stem().and_then(|stem| stem.to_str()) {
            candidates.push(stem.to_string());
        }
    }

    candidates
        .into_iter()
        .map(|candidate| score_identity_match(&normalized_identity, &identity_tokens, &candidate))
        .max()
        .unwrap_or(0)
}

fn normalize_identity(input: &str) -> String {
    let mut normalized = String::with_capacity(input.len());
    let mut prev_lowercase = false;

    for ch in input.chars() {
        if ch.is_ascii_uppercase() && prev_lowercase {
            normalized.push(' ');
        }

        if ch.is_ascii_alphanumeric() {
            normalized.push(ch.to_ascii_lowercase());
        } else {
            normalized.push(' ');
        }

        prev_lowercase = ch.is_ascii_lowercase() || ch.is_ascii_digit();
    }

    normalized.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn tokenize_identity(input: &str) -> Vec<String> {
    normalize_identity(input)
        .split_whitespace()
        .filter(|token| token.len() >= 2)
        .map(str::to_string)
        .collect()
}

fn score_identity_match(
    normalized_identity: &str,
    identity_tokens: &[String],
    candidate: &str,
) -> usize {
    let normalized_candidate = normalize_identity(candidate);
    if normalized_candidate.is_empty() {
        return 0;
    }

    let candidate_tokens = tokenize_identity(candidate);
    if candidate_tokens.is_empty() && normalized_candidate.len() < 3 {
        return 0;
    }

    let mut score = 0;
    if normalized_candidate == normalized_identity {
        score += 4;
    }
    if normalized_candidate.len() >= 3
        && (normalized_identity.contains(&normalized_candidate)
            || normalized_candidate.contains(normalized_identity))
    {
        score += 2;
    }

    score
        + identity_tokens
            .iter()
            .filter(|token| {
                candidate_tokens
                    .iter()
                    .any(|candidate_token| candidate_token == *token)
            })
            .count()
            * 2
}
