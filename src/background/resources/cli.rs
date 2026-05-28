use std::path::PathBuf;

use libs_core::resource::ResourceKind;
use serde::{Deserialize, Serialize};

use crate::{error::Result, resources::RESOURCES};

/// Manage the Resources.
#[derive(Debug, Serialize, Deserialize, clap::Args)]
pub struct ResourceManagerCli {
    #[command(subcommand)]
    subcommand: SubCommand,
}

#[derive(Debug, Serialize, Deserialize, clap::Subcommand)]
enum SubCommand {
    /// loads a widget into the internal registry
    Load {
        kind: ClapResourceKind,
        path: PathBuf,
    },
    /// deletes the widget from internal registry
    Unload {
        kind: ClapResourceKind,
        path: PathBuf,
    },
}

impl ResourceManagerCli {
    pub fn process(self) -> Result<()> {
        match self.subcommand {
            SubCommand::Load { kind, path } => {
                let kind = kind.into();
                RESOURCES.load(&kind, &path)?;
                let _ = RESOURCES.manual.insert(path);
                RESOURCES.emit_kind_changed(&kind)?;
            }
            SubCommand::Unload { kind, path } => {
                let kind = kind.into();
                RESOURCES.unload(&kind, &path);
                RESOURCES.manual.remove(&path);
                RESOURCES.emit_kind_changed(&kind)?;
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, clap::ValueEnum)]
enum ClapResourceKind {
    Theme,
    IconPack,
    Widget,
}

impl From<ClapResourceKind> for ResourceKind {
    fn from(value: ClapResourceKind) -> Self {
        match value {
            ClapResourceKind::Theme => ResourceKind::Theme,
            ClapResourceKind::IconPack => ResourceKind::IconPack,
            ClapResourceKind::Widget => ResourceKind::Widget,
        }
    }
}
