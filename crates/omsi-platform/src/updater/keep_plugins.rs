//! K-OMSI: an update keeps the player's plugins.
//!
//! The update swaps every top-level item of the archive in whole, and K-OMSI's archive brings
//! a `Plugins` folder (its OBIS and tachograph plugins): the folder the player had - with the
//! plugins added by hand or by an add-on's installer (the EB terminal's `eb_nearby`, a newer
//! `komsi_obis` it ships) - was put aside and deleted. So before the swap, what the old
//! `Plugins` folder holds is brought into the new one:
//!
//! * a plugin the archive does not bring stays as it is;
//! * a plugin the archive brings too is the archive's, unless the installed one says it is
//!   newer: its `main.lua` starts with `-- PLUGIN_STAMP <number>` and the archive's has none or
//!   a smaller one - then the installed folder stays whole (its files belong together);
//! * a file of the installed plugin the archive's does not have (its settings, its data) stays.
//!
//! The swap and its way back are unchanged: they only see a fuller staged folder.

use std::path::Path;

/// The folder of plugins in the program's folder (as release.yml packs it).
const PLUGINS: &str = "Plugins";

/// Bring the installed `Plugins` folder's contents into the staged one (see the module).
pub(super) fn merge_into_staging(staging: &Path, dir: &Path) {
    let Ok(entries) = std::fs::read_dir(staging) else { return };
    for e in entries.flatten() {
        let name = e.file_name().to_string_lossy().to_string();
        if !name.eq_ignore_ascii_case(PLUGINS) || !e.path().is_dir() {
            continue;
        }
        let installed = dir.join(&name);
        if !installed.is_dir() {
            continue;
        }
        if let Err(err) = merge_plugins(&installed, &e.path()) {
            log::warn!("update: the installed plugins could not all be kept ({err})");
        }
    }
}

fn merge_plugins(installed: &Path, staged: &Path) -> std::io::Result<()> {
    for e in std::fs::read_dir(installed)?.flatten() {
        let from = e.path();
        let to = staged.join(e.file_name());
        if !to.exists() {
            copy_all(&from, &to)?;
        } else if from.is_dir() && to.is_dir() {
            if stamp(&from) > stamp(&to) {
                log::info!("update: kept the newer installed plugin {}", e.file_name().to_string_lossy());
                std::fs::remove_dir_all(&to)?;
                copy_all(&from, &to)?;
            } else {
                copy_missing(&from, &to)?;
            }
        }
    }
    Ok(())
}

/// The `PLUGIN_STAMP` of a plugin folder's `main.lua` (None: no file, or no stamp).
fn stamp(plugin: &Path) -> Option<u64> {
    let text = std::fs::read(plugin.join("main.lua")).ok()?;
    let first = text.split(|b| *b == b'\n').next()?;
    let first = String::from_utf8_lossy(first);
    let rest = first.trim_start().strip_prefix("--")?.trim_start().strip_prefix("PLUGIN_STAMP")?;
    rest.split_whitespace().next()?.parse().ok()
}

fn copy_all(from: &Path, to: &Path) -> std::io::Result<()> {
    if from.is_dir() {
        std::fs::create_dir_all(to)?;
        for e in std::fs::read_dir(from)?.flatten() {
            copy_all(&e.path(), &to.join(e.file_name()))?;
        }
        Ok(())
    } else {
        std::fs::copy(from, to).map(|_| ())
    }
}

fn copy_missing(from: &Path, to: &Path) -> std::io::Result<()> {
    for e in std::fs::read_dir(from)?.flatten() {
        let src = e.path();
        let dst = to.join(e.file_name());
        if !dst.exists() {
            copy_all(&src, &dst)?;
        } else if src.is_dir() && dst.is_dir() {
            copy_missing(&src, &dst)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(p: &Path, text: &str) {
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, text).unwrap();
    }

    fn read(p: &Path) -> String {
        std::fs::read_to_string(p).unwrap()
    }

    #[test]
    fn the_players_plugins_survive_an_update() {
        let root = std::env::temp_dir().join(format!("komsi-keep-plugins-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let (dir, staging) = (root.join("game"), root.join("game/.stage"));
        // installed: an add-on's plugin, a newer komsi_obis, an older komsi_taco with settings
        write(&dir.join("Plugins/eb_nearby/main.lua"), "-- PLUGIN_STAMP 202611190100\neb");
        write(&dir.join("Plugins/eb_nearby/names.json"), "{}");
        write(&dir.join("Plugins/komsi_obis/main.lua"), "-- PLUGIN_STAMP 202611182320\nnew obis");
        write(&dir.join("Plugins/komsi_obis/sender.lua"), "new sender");
        write(&dir.join("Plugins/komsi_taco/main.lua"), "old taco");
        write(&dir.join("Plugins/komsi_taco/settings.json"), "mine");
        // the archive: an older komsi_obis (no stamp) and a newer komsi_taco
        write(&staging.join("Plugins/komsi_obis/main.lua"), "old obis");
        write(&staging.join("Plugins/komsi_obis/sender.lua"), "old sender");
        write(&staging.join("Plugins/komsi_taco/main.lua"), "new taco");
        write(&staging.join("openomsi.exe"), "exe");

        merge_into_staging(&staging, &dir);

        assert_eq!(read(&staging.join("Plugins/eb_nearby/main.lua")), "-- PLUGIN_STAMP 202611190100\neb");
        assert_eq!(read(&staging.join("Plugins/eb_nearby/names.json")), "{}");
        assert_eq!(read(&staging.join("Plugins/komsi_obis/main.lua")), "-- PLUGIN_STAMP 202611182320\nnew obis");
        assert_eq!(read(&staging.join("Plugins/komsi_obis/sender.lua")), "new sender");
        assert_eq!(read(&staging.join("Plugins/komsi_taco/main.lua")), "new taco");
        assert_eq!(read(&staging.join("Plugins/komsi_taco/settings.json")), "mine");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_newer_stamp_in_the_archive_wins() {
        let root = std::env::temp_dir().join(format!("komsi-keep-plugins-b-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let (dir, staging) = (root.join("game"), root.join("game/.stage"));
        write(&dir.join("Plugins/komsi_obis/main.lua"), "-- PLUGIN_STAMP 202611010000\nold");
        write(&staging.join("Plugins/komsi_obis/main.lua"), "-- PLUGIN_STAMP 202612010000\nnew");
        merge_into_staging(&staging, &dir);
        assert_eq!(read(&staging.join("Plugins/komsi_obis/main.lua")), "-- PLUGIN_STAMP 202612010000\nnew");
        let _ = std::fs::remove_dir_all(&root);
    }
}
