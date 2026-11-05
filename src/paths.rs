use anyhow::{Context, Result};
use rusqlite::Connection;
use std::path::{Path, PathBuf};

/// TMDBHelper schema plus its seed rows (`languages`, `countries`), dumped
/// from `tests/fixtures/ItemDetails-empty.db`.
const ITEM_DETAILS_SCHEMA: &str = include_str!("../schema/ItemDetails.sql");

pub const DEFAULT_OUTPUT_DIR: &str = "./output";

/// Where the binary is running, which decides the default file locations.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Platform {
    /// CoreELEC, LibreELEC, or OpenELEC: write into Kodi's own files.
    KodiAppliance,
    /// Any other OS (Windows, macOS, desktop Linux): write under an output folder.
    Desktop,
}

const APPLIANCE_IDS: &[&str] = &["coreelec", "libreelec", "openelec"];

/// Returns `true` when an `os-release` file names a Kodi appliance distro.
pub fn is_appliance_os_release(contents: &str) -> bool {
    contents.lines().any(|line| {
        let Some((key, value)) = line.split_once('=') else {
            return false;
        };
        if !matches!(key.trim(), "ID" | "ID_LIKE" | "NAME") {
            return false;
        }
        let value = value.trim().trim_matches('"').to_ascii_lowercase();
        value
            .split(|c: char| c.is_whitespace() || c == ',')
            .any(|word| APPLIANCE_IDS.contains(&word))
    })
}

impl Platform {
    pub fn detect() -> Self {
        if cfg!(target_os = "linux") {
            for path in ["/etc/os-release", "/usr/lib/os-release"] {
                if let Ok(contents) = std::fs::read_to_string(path) {
                    if is_appliance_os_release(&contents) {
                        return Platform::KodiAppliance;
                    }
                    break;
                }
            }
        }
        Platform::Desktop
    }

    pub fn name(self) -> &'static str {
        match self {
            Platform::KodiAppliance => "kodi-appliance",
            Platform::Desktop => "desktop",
        }
    }
}

/// Resolved file locations for one run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Paths {
    pub state_db: PathBuf,
    pub item_details_db: PathBuf,
    pub textures_db: PathBuf,
    pub thumbnails_dir: PathBuf,
}

impl Paths {
    /// Default locations for a platform. `output_dir` applies only to `Desktop`.
    pub fn defaults(platform: Platform, output_dir: &Path) -> Self {
        match platform {
            Platform::KodiAppliance => Paths {
                state_db: "/var/media/CACHE_DRIVE/tmdb/scriptcache/state.db".into(),
                item_details_db: "/storage/.kodi/userdata/addon_data/plugin.video.themoviedb.helper/database_07/ItemDetails.db".into(),
                textures_db: "/var/media/CACHE_DRIVE/tmdb/Textures13.db".into(),
                thumbnails_dir: "/var/media/CACHE_DRIVE/tmdb/Thumbnails".into(),
            },
            Platform::Desktop => Paths {
                state_db: output_dir.join("state.db"),
                item_details_db: output_dir.join("ItemDetails.db"),
                textures_db: output_dir.join("Textures13.db"),
                thumbnails_dir: output_dir.join("images"),
            },
        }
    }
}

/// Free space on the disk that holds `path`, or `None` if the system can't say.
#[cfg(target_os = "macos")]
pub fn available_space(path: &Path) -> Option<u64> {
    use std::os::unix::ffi::OsStrExt;
    let path = std::ffi::CString::new(path.as_os_str().as_bytes()).ok()?;
    let mut stat = std::mem::MaybeUninit::<libc::statfs>::uninit();
    // macOS keeps `statvfs` block counts in 32 bits, so a share with over 2^32
    // free blocks wraps around. `statfs` uses 64 bits.
    if unsafe { libc::statfs(path.as_ptr(), stat.as_mut_ptr()) } != 0 {
        return None;
    }
    let stat = unsafe { stat.assume_init() };
    Some(stat.f_bavail * u64::from(stat.f_bsize))
}

/// Free space on the disk that holds `path`, or `None` if the system can't say.
#[cfg(not(target_os = "macos"))]
pub fn available_space(path: &Path) -> Option<u64> {
    fs2::available_space(path).ok()
}

/// The folder that holds `path`. A bare file name's parent is the current folder.
pub fn parent_dir(path: &std::path::Path) -> &std::path::Path {
    match path.parent() {
        Some(p) if !p.as_os_str().is_empty() => p,
        _ => std::path::Path::new("."),
    }
}

/// Creates the parent folders of every database and the thumbnails folder.
pub fn ensure_dirs(paths: &Paths) -> Result<()> {
    for db in [&paths.state_db, &paths.item_details_db, &paths.textures_db] {
        if let Some(parent) = db.parent().filter(|p| !p.as_os_str().is_empty()) {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("create {}", parent.display()))?;
        }
    }
    std::fs::create_dir_all(&paths.thumbnails_dir)
        .with_context(|| format!("create {}", paths.thumbnails_dir.display()))?;
    Ok(())
}

/// Creates `ItemDetails.db` from the bundled TMDBHelper schema if it doesn't exist.
/// Returns `true` when it created the file.
pub fn create_item_details_if_missing(path: &Path) -> Result<bool> {
    if path.exists() {
        return Ok(false);
    }
    let conn = Connection::open(path).with_context(|| format!("create {}", path.display()))?;
    conn.execute_batch(ITEM_DETAILS_SCHEMA)
        .with_context(|| format!("apply TMDBHelper schema to {}", path.display()))?;
    Ok(true)
}
