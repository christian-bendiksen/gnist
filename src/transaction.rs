//! Publishes a staged render by atomically renaming it into place.
use crate::{ctx::Ctx, util};
use anyhow::{Context, Result};
use rustix::fs::{CWD, RenameFlags, renameat_with};
use std::{
    fs,
    path::{Path, PathBuf},
};
use tempfile::{Builder, TempDir};

pub struct Transaction {
    stage: TempDir,
    live: PathBuf,
    link: PathBuf,
}

impl Transaction {
    /// Start a transaction with a private staging directory under `generated/`.
    pub fn begin(ctx: &Ctx) -> Result<Self> {
        fs::create_dir_all(&ctx.generated_dir).context("create generated dir")?;

        let stage = Builder::new()
            .prefix(".stage.")
            .tempdir_in(&ctx.generated_dir)
            .context("create staging dir")?;

        Ok(Self {
            stage,
            live: ctx.live_dir.clone(),
            link: ctx.current_link.clone(),
        })
    }

    /// Return the directory that receives rendered files.
    #[inline]
    pub fn stage(&self) -> &Path {
        self.stage.path()
    }

    /// Promote the staged tree to `live/`, then point `current` at it.
    ///
    /// Promotion is atomic, but updating the symlink is a separate operation.
    pub fn commit(self) -> Result<()> {
        // Keep the staging tree if publishing fails so TempDir cannot remove it.
        let stage_path = self.stage.keep();

        if fs::symlink_metadata(&self.live).is_ok() {
            renameat_with(CWD, &stage_path, CWD, &self.live, RenameFlags::EXCHANGE)
                .context("atomic exchange stage <-> live")?;
            fs::remove_dir_all(&stage_path).context("remove old live dir")?;
        } else {
            // The first publish has no live tree to exchange.
            fs::rename(&stage_path, &self.live).context("rename stage -> live")?;
        }

        util::symlink_force(&self.live, &self.link).context("update current symlink")
    }
}
