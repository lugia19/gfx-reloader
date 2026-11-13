//! The package's menu files: a snapshot of their modification times, what changed since, and the watcher.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use crate::game::menu_names;
use crate::log;

type Stamps = HashMap<PathBuf, (SystemTime, u64)>;

pub struct MenuFiles {
    folder: PathBuf,
    /// State at the last reload (or launch).
    snapshot: Stamps,
    /// Watcher: the changed state seen on the previous poll, reloaded once it stays the same (file fully written).
    settling: Option<Stamps>,
}

fn scan(dir: &Path, out: &mut Stamps) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        let Ok(meta) = entry.metadata() else { continue };
        if meta.is_dir() {
            scan(&path, out);
        } else if path.extension().is_some_and(|e| e.eq_ignore_ascii_case("gfx")) {
            out.insert(path, (meta.modified().unwrap_or(SystemTime::UNIX_EPOCH), meta.len()));
        }
    }
}

impl MenuFiles {
    pub fn new(package: &Path) -> Self {
        let folder = package.join("menu");
        let mut snapshot = Stamps::new();
        scan(&folder, &mut snapshot);
        log(&format!("watching {} ({} menu files)", folder.display(), snapshot.len()));
        Self { folder, snapshot, settling: None }
    }

    fn current(&self) -> Stamps {
        let mut stamps = Stamps::new();
        scan(&self.folder, &mut stamps);
        stamps
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
            if before.is_none() {
                log(&format!(
                    "{file}: new since the game started; me3 only serves files that existed at launch, so restart \
                     the game once to use it"
                ));
                continue;
            }
            let stem = path.file_stem().map(|s| s.to_string_lossy().to_uppercase()).unwrap_or_default();
            match names.iter().find(|n| n.to_uppercase() == stem) {
                Some(name) => changed.push(name.clone()),
                None => log(&format!("{file}: changed, but no menu uses that name; ignored")),
            }
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
