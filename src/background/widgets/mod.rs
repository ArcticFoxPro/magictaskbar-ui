pub mod contextmenu;
pub mod popup_glass_effect;
pub mod preview;
pub mod taskbar;
pub mod toolbar;

use std::path::PathBuf;

use crate::utils::constants::VAR_COMMON;

pub struct WebviewArgs {
    pub args: Vec<String>,
}

impl WebviewArgs {
    const BASE_1: &str = "--disable-features=msWebOOUI,msPdfOOUI,msSmartScreenProtection,RendererAppContainer,msEdgeMiniMenu,msEdgeCollections,msEdgeIdentity,msEdgeTranslate,msBingChat,msEdgeSidebar,msDownloadsHub,msEdgeVisualSearch,msEnvironmentVariableReduction,msWebView2EnableTopControls,BackForwardCache,SpareRendererForSitePerProcess,CalculateNativeWinOcclusion,Translate,HeavyAdPrivacyMitigations,MediaRouter,IntensiveWakeUpThrottling,BackgroundResourceIntervention";
    const BASE_2: &str = "--disable-site-isolation-trials";
    const MEMORY_OPTS: &str = "--disable-background-networking --disable-sync --disable-translate --no-first-run --disable-domain-reliability --disable-component-update --disable-renderer-accessibility --disable-infobars --disable-client-side-phishing-detection --disable-background-timer-throttling --disable-renderer-backgrounding --disable-backgrounding-occluded-windows --no-service-autorun";

    pub fn new() -> Self {
        Self {
            args: vec![
                Self::BASE_1.to_string(),
                Self::BASE_2.to_string(),
                Self::MEMORY_OPTS.to_string(),
            ],
        }
    }

    pub fn with(mut self, arg: &str) -> Self {
        self.args.push(arg.to_string());
        self
    }

    pub fn disable_gpu(self) -> Self {
        self.with("--disable-gpu --in-process-gpu")
    }

    pub fn data_directory(&self) -> PathBuf {
        // remove bases
        let mut args = self.args.clone();
        args.remove(0);
        args.remove(0);
        args.remove(0);

        if args.is_empty() {
            VAR_COMMON.app_cache_dir().to_path_buf()
        } else {
            VAR_COMMON
                .app_cache_dir()
                .join(args.join("").replace("-", ""))
        }
    }
}

impl std::fmt::Display for WebviewArgs {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.args.join(" "))
    }
}
