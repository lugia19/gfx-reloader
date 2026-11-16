//! Package menu files: content hashes, changes, watcher. Hashes, not times: copies and zip files share times.

use std::collections::{HashMap, HashSet};
use std::hash::Hasher;
use std::path::{Path, PathBuf};

use crate::game::menu_names;
use crate::log;

/// Unreadable files (being written) are left out.
type Stamps = HashMap<PathBuf, u64>;

fn content_hash(path: &Path) -> Option<u64> {
    let bytes = std::fs::read(path).ok()?;
    let mut hasher = std::hash::DefaultHasher::new();
    hasher.write(&bytes);
    Some(hasher.finish())
}

pub struct MenuFiles {
    folder: PathBuf,
    /// me3 only serves these.
    at_launch: HashSet<PathBuf>,
    snapshot: Stamps,
    /// Reloaded once unchanged for one poll (file fully written).
    settling: Option<Stamps>,
}

fn scan(dir: &Path, out: &mut Stamps) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            scan(&path, out);
        } else if path.extension().is_some_and(|e| e.eq_ignore_ascii_case("gfx")) {
            if let Some(hash) = content_hash(&path) {
                // case-insensitive, like the game
                out.insert(PathBuf::from(path.to_string_lossy().to_lowercase()), hash);
            }
        }
    }
}

fn stamps(folder: &Path) -> Stamps {
    let mut stamps = Stamps::new();
    scan(folder, &mut stamps);
    stamps
}

impl MenuFiles {
    pub fn new(package: &Path) -> Self {
        let folder = package.join("menu");
        let snapshot = stamps(&folder);
        log(&format!("watching {} ({} menu files)", folder.display(), snapshot.len()));
        Self { folder, at_launch: snapshot.keys().cloned().collect(), snapshot, settling: None }
    }

    /// An unreadable existing file keeps its old hash (not removed-then-new).
    fn current(&self) -> Stamps {
        let mut now = stamps(&self.folder);
        for (path, hash) in &self.snapshot {
            if !now.contains_key(path) && path.exists() {
                now.insert(path.clone(), *hash);
            }
        }
        now
    }

    pub fn take_changes(&mut self) -> Vec<String> {
        let now = self.current();
        let names = menu_names();
        let mut changed = Vec::new();
        for (path, stamp) in &now {
            let before = self.snapshot.get(path);
            if before == Some(stamp) {
                continue;
            }
            let file = path.file_name().map(|f| f.to_string_lossy().into_owned()).unwrap_or_default();
            let stem = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
            let Some(name) = names.iter().find(|n| n.eq_ignore_ascii_case(&stem)) else {
                log(&format!("{file}: no menu uses that name; ignored"));
                continue;
            };
            // not served by me3; say so once
            if !self.at_launch.contains(path) {
                if before.is_none() {
                    log(&format!(
                        "{file}: new since the game started; me3 only serves files that existed at launch, so \
                         restart the game once to use it"
                    ));
                    crate::toast_for(&format!("{file} is new: restart the game to use it"), crate::TOAST_LONG_SECONDS);
                }
                continue;
            }
            changed.push(name.clone());
        }
        self.snapshot = now;
        self.settling = None;
        changed
    }

    pub fn changes_settled(&mut self) -> bool {
        let now = self.current();
        if now == self.snapshot {
            self.settling = None;
            return false;
        }
        if self.settling.as_ref() == Some(&now) {
            return true;
        }
        self.settling = Some(now);
        false
    }
}
