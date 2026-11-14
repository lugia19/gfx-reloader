//! The package's menu files: a snapshot of their contents (as hashes), what changed since, and the watcher.
//!
//! Contents, not modification times: a file copied over another keeps its source's time, and files from one zip
//! share theirs, so two different files can have the same time and size.

use std::collections::{HashMap, HashSet};
use std::hash::Hasher;
use std::path::{Path, PathBuf};

use crate::game::menu_names;
use crate::log;

/// Hash of each file's contents. A file that can't be read right now (being written) is left out.
type Stamps = HashMap<PathBuf, u64>;

fn content_hash(path: &Path) -> Option<u64> {
    let bytes = std::fs::read(path).ok()?;
    let mut hasher = std::hash::DefaultHasher::new();
    hasher.write(&bytes);
    Some(hasher.finish())
}

pub struct MenuFiles {
    folder: PathBuf,
    /// The files that were in the package when the game started: the only ones me3 serves.
    at_launch: HashSet<PathBuf>,
    /// State at the last reload (or launch).
    snapshot: Stamps,
    /// Watcher: the changed state seen on the previous poll, reloaded once it stays the same (file fully written).
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
                // lowercase, so a rename that only changes the case is the same file (as it is for the game)
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

    /// The package now. A file that exists but can't be read at the moment keeps the hash it had, so it counts as
    /// unchanged until it can be read (not as removed and then new).
    fn current(&self) -> Stamps {
        let mut now = stamps(&self.folder);
        for (path, hash) in &self.snapshot {
            if !now.contains_key(path) && path.exists() {
                now.insert(path.clone(), *hash);
            }
        }
        now
    }

    /// F7: menu names of the files changed since the last reload, and takes a new snapshot.
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
            // A file that was there at launch and is back after being deleted or renamed away (no `before`) counts
            // as changed. One that wasn't there at launch is never served: say so once, when it first shows up.
            let stem = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
            let Some(name) = names.iter().find(|n| n.eq_ignore_ascii_case(&stem)) else {
                // e.g. a copy waiting to be renamed over a menu's file
                log(&format!("{file}: no menu uses that name; ignored"));
                continue;
            };
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

    /// Watcher poll: true once a change has stayed the same for one poll (so half-written files aren't loaded).
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
